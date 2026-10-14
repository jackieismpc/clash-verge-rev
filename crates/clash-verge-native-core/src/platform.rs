//! macOS networking ownership transactions. No constructor performs OS writes.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkMode {
    Interactive,
    IntegrationTest,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NetworkValue {
    pub service: String,
    pub protocol: String,
    pub configuration: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnedChange {
    pub before: NetworkValue,
    pub applied: NetworkValue,
    #[serde(default)]
    pub restore_pending: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OwnershipSnapshot {
    pub owner: String,
    pub changes: Vec<OwnedChange>,
}

#[derive(Debug)]
pub struct MissingNetworkService;
impl std::fmt::Display for MissingNetworkService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "network service or protocol was removed")
    }
}
impl std::error::Error for MissingNetworkService {}

pub trait NetworkBackend {
    fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue>;
    /// Commit one configuration atomically. The transaction guards mode before calling this.
    fn write(&mut self, value: &NetworkValue) -> Result<()>;
    fn compare_and_write(&mut self, expected: &NetworkValue, desired: &NetworkValue) -> Result<bool> {
        if self.read(&expected.service, &expected.protocol)? != *expected {
            return Ok(false);
        }
        self.write(desired)?;
        Ok(true)
    }
}

pub struct NetworkTransaction<B> {
    pub backend: B,
    mode: NetworkMode,
    pub ownership: OwnershipSnapshot,
}
impl<B: NetworkBackend> NetworkTransaction<B> {
    pub fn new(backend: B, mode: NetworkMode, owner: String) -> Result<Self> {
        if !owner.starts_with("native-mac:") || owner.len() < 16 {
            bail!("independent native owner identity required");
        }
        Ok(Self {
            backend,
            mode,
            ownership: OwnershipSnapshot { owner, changes: vec![] },
        })
    }
    pub fn adopt_snapshot(&mut self, snapshot: OwnershipSnapshot) -> Result<()> {
        if snapshot.owner != self.ownership.owner {
            bail!("network journal belongs to another instance");
        }
        for change in &snapshot.changes {
            if [&change.before.configuration, &change.applied.configuration]
                .into_iter()
                .any(|configuration| configuration.as_ref().is_some_and(|value| !value.is_object()))
            {
                bail!("invalid ownership configuration dictionary");
            }
            if change.before.service != change.applied.service
                || change.before.protocol != change.applied.protocol
                || !matches!(change.applied.protocol.as_str(), "DNS" | "Proxies")
            {
                bail!("invalid ownership journal");
            }
        }
        self.ownership = snapshot;
        Ok(())
    }
    pub fn get_state(&mut self) -> Result<Vec<NetworkValue>> {
        self.ownership
            .changes
            .iter()
            .map(|change| self.backend.read(&change.applied.service, &change.applied.protocol))
            .collect()
    }
    fn allow_write(&self) -> Result<()> {
        if self.mode == NetworkMode::IntegrationTest {
            bail!("integrationTest forbids OS networking and helper writes");
        }
        Ok(())
    }
    pub fn apply(&mut self, desired: Vec<NetworkValue>) -> Result<()> {
        self.apply_with_checkpoint(desired, |_| Ok(()))
    }
    /// The caller atomically persists the journal in its private data directory before each write.
    pub fn apply_with_checkpoint(
        &mut self,
        desired: Vec<NetworkValue>,
        mut checkpoint: impl FnMut(&OwnershipSnapshot) -> Result<()>,
    ) -> Result<()> {
        self.allow_write()?;
        if !self.ownership.changes.is_empty() {
            bail!("restore existing ownership before applying another transaction");
        }
        // Capture all originals before the first write.
        let mut changes = Vec::new();
        for value in desired {
            if value.configuration.as_ref().is_some_and(|value| !value.is_object()) {
                bail!("network configuration must be a dictionary");
            }
            if value.protocol != "Proxies" && value.protocol != "DNS" {
                bail!("unsupported network protocol");
            }
            if changes
                .iter()
                .any(|c: &OwnedChange| c.applied.service == value.service && c.applied.protocol == value.protocol)
            {
                bail!("duplicate service protocol");
            }
            let before = self.backend.read(&value.service, &value.protocol)?;
            if before != value {
                changes.push(OwnedChange {
                    before,
                    applied: value,
                    restore_pending: false,
                });
            }
        }
        for change in changes {
            // Record even a failed write: commit/apply may fail after preferences changed.
            self.ownership.changes.push(change.clone());
            let result = checkpoint(&self.ownership)
                .and_then(|_| self.backend.compare_and_write(&change.before, &change.applied))
                .and_then(|changed| {
                    if changed {
                        Ok(())
                    } else {
                        bail!("network settings changed before apply")
                    }
                });
            if let Err(error) = result {
                let recovery = self.restore();
                return Err(error.context(format!("network apply failed; recovery: {recovery:?}")));
            }
        }
        Ok(())
    }
    /// Restore only changed fields still equal to our applied values.
    /// External edits are preserved. Failed restores remain available for retry.
    pub fn restore(&mut self) -> Result<Vec<String>> {
        self.allow_write()?;
        let mut skipped = Vec::new();
        let mut retained = Vec::new();
        let mut failures = Vec::new();
        for mut change in self.ownership.changes.drain(..).rev() {
            match self.backend.read(&change.applied.service, &change.applied.protocol) {
                Ok(current) => {
                    let (mut desired, externally_changed) = restoration_value(&current, &change);
                    // A prior commit can succeed while SCPreferencesApplyChanges fails.
                    if desired.is_none() && change.restore_pending && owned_fields_restored(&current, &change) {
                        desired = Some(current.clone());
                    }
                    if externally_changed {
                        skipped.push(format!(
                            "{}:{} externally changed fields preserved",
                            change.applied.service, change.applied.protocol
                        ));
                    }
                    if let Some(desired) = desired {
                        match self.backend.compare_and_write(&current, &desired) {
                            Ok(true) => (),
                            Ok(false) => {
                                failures.push("network changed during restoration; retry required".into());
                                retained.push(change);
                            }
                            Err(error) => {
                                failures.push(error.to_string());
                                change.restore_pending = true;
                                retained.push(change);
                            }
                        }
                    }
                }
                Err(error) if error.is::<MissingNetworkService>() => {
                    skipped.push(format!(
                        "{}:{} was removed",
                        change.applied.service, change.applied.protocol
                    ));
                }
                Err(error) => {
                    failures.push(error.to_string());
                    retained.push(change);
                }
            }
        }
        retained.reverse();
        self.ownership.changes = retained;
        if !failures.is_empty() {
            bail!("network restoration incomplete: {}", failures.join("; "));
        }
        Ok(skipped)
    }
}

fn owned_fields_restored(current: &NetworkValue, change: &OwnedChange) -> bool {
    let before = change.before.configuration.as_ref().and_then(Value::as_object);
    let applied = change.applied.configuration.as_ref().and_then(Value::as_object);
    let current = current.configuration.as_ref().and_then(Value::as_object);
    let mut keys = std::collections::BTreeSet::new();
    if let Some(before) = before {
        keys.extend(before.keys());
    }
    if let Some(applied) = applied {
        keys.extend(applied.keys());
    }
    keys.into_iter()
        .filter(|key| before.and_then(|v| v.get(*key)) != applied.and_then(|v| v.get(*key)))
        .all(|key| current.and_then(|v| v.get(key)) == before.and_then(|v| v.get(key)))
}

fn restoration_value(current: &NetworkValue, change: &OwnedChange) -> (Option<NetworkValue>, bool) {
    if *current == change.applied {
        return (Some(change.before.clone()), false);
    }
    let Some(applied) = change.applied.configuration.as_ref().and_then(Value::as_object) else {
        return (None, true);
    };
    let Some(current_object) = current.configuration.as_ref().and_then(Value::as_object) else {
        return (None, true);
    };
    let before = change.before.configuration.as_ref().and_then(Value::as_object);
    let mut restored = current_object.clone();
    let mut changed = false;
    let mut external = false;
    let mut keys: std::collections::BTreeSet<_> = applied.keys().collect();
    if let Some(before) = before {
        keys.extend(before.keys());
    }
    for key in keys {
        let original = before.and_then(|before| before.get(key));
        if original == applied.get(key) {
            continue;
        }
        if current_object.get(key) != applied.get(key) {
            external = true;
            continue;
        }
        changed = true;
        match original {
            Some(value) => {
                restored.insert(key.clone(), value.clone());
            }
            None => {
                restored.remove(key);
            }
        }
    }
    if !changed {
        return (None, true);
    }
    let configuration = if restored.is_empty() && change.before.configuration.is_none() {
        None
    } else {
        Some(Value::Object(restored))
    };
    (
        Some(NetworkValue {
            configuration,
            ..current.clone()
        }),
        external || *current != change.applied,
    )
}

/// Merge only proxy keys into the service's complete original dictionary.
pub fn proxy_configuration(original: &NetworkValue, port: u16) -> Result<NetworkValue> {
    if port == 0 || original.protocol != "Proxies" {
        bail!("invalid proxy target");
    }
    let mut configuration = original.configuration.clone().unwrap_or_else(|| json!({}));
    let object = configuration
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("proxy configuration is not a dictionary"))?;
    for (key, value) in [
        ("ProxyAutoConfigEnable", json!(0)),
        ("ProxyAutoDiscoveryEnable", json!(0)),
        ("HTTPEnable", json!(1)),
        ("HTTPProxy", json!("127.0.0.1")),
        ("HTTPPort", json!(port)),
        ("HTTPSEnable", json!(1)),
        ("HTTPSProxy", json!("127.0.0.1")),
        ("HTTPSPort", json!(port)),
        ("SOCKSEnable", json!(1)),
        ("SOCKSProxy", json!("127.0.0.1")),
        ("SOCKSPort", json!(port)),
    ] {
        object.insert(key.into(), value);
    }
    Ok(NetworkValue {
        configuration: Some(configuration),
        ..original.clone()
    })
}

pub fn validate_dns_servers(servers: &[String]) -> Result<()> {
    if servers.len() > 16 {
        bail!("DNS accepts at most 16 servers");
    }
    for server in servers {
        let mut address: std::net::IpAddr = server.parse()?;
        if let std::net::IpAddr::V6(v6) = address
            && let Some(v4) = v6.to_ipv4_mapped()
        {
            address = v4.into();
        }
        if address.is_loopback()
            || address.is_unspecified()
            || address.is_multicast()
            || address == std::net::IpAddr::V4(std::net::Ipv4Addr::BROADCAST)
        {
            bail!(
                "system DNS must use a public or LAN unicast resolver; this app does not own a localhost DNS listener"
            );
        }
    }
    Ok(())
}

/// Keep search domains and all unrelated DNS fields; restore uses the exact original dictionary.
pub fn dns_configuration(original: &NetworkValue, servers: &[String]) -> Result<NetworkValue> {
    if original.protocol != "DNS" || servers.is_empty() {
        bail!("DNS requires at least one literal IP address");
    }
    validate_dns_servers(servers)?;
    let mut configuration = original.configuration.clone().unwrap_or_else(|| json!({}));
    configuration
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("DNS configuration is not a dictionary"))?
        .insert("ServerAddresses".into(), json!(servers));
    Ok(NetworkValue {
        configuration: Some(configuration),
        ..original.clone()
    })
}

#[cfg(test)]
#[derive(Default)]
pub struct SimulatedNetwork {
    pub values: BTreeMap<(String, String), Option<Value>>,
    pub writes: usize,
}
#[cfg(test)]
impl NetworkBackend for SimulatedNetwork {
    fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue> {
        Ok(NetworkValue {
            service: service.into(),
            protocol: protocol.into(),
            configuration: self.values.get(&(service.into(), protocol.into())).cloned().flatten(),
        })
    }
    fn write(&mut self, value: &NetworkValue) -> Result<()> {
        self.writes += 1;
        self.values.insert(
            (value.service.clone(), value.protocol.clone()),
            value.configuration.clone(),
        );
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[path = "platform_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacNetwork;

#[cfg(test)]
mod tests {
    use super::*;
    fn value(protocol: &str, configuration: Value) -> NetworkValue {
        NetworkValue {
            service: "fixture-service".into(),
            protocol: protocol.into(),
            configuration: Some(configuration),
        }
    }
    #[test]
    fn manual_proxy_disables_pac_and_discovery_then_restores_only_owned_fields() {
        let original = value(
            "Proxies",
            json!({"ProxyAutoConfigEnable":1,"ProxyAutoConfigURLString":"https://fixture.invalid/old.pac","ProxyAutoDiscoveryEnable":1,"HTTPEnable":0}),
        );
        let mut tx = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        tx.backend.write(&original).unwrap();
        let enabled = proxy_configuration(&original, 7890).unwrap();
        assert_eq!(enabled.configuration.as_ref().unwrap()["ProxyAutoConfigEnable"], 0);
        assert_eq!(enabled.configuration.as_ref().unwrap()["ProxyAutoDiscoveryEnable"], 0);
        tx.apply(vec![enabled.clone()]).unwrap();
        let mut external = enabled;
        external.configuration.as_mut().unwrap()["ProxyAutoConfigURLString"] =
            json!("https://fixture.invalid/external.pac");
        tx.backend.write(&external).unwrap();
        tx.restore().unwrap();
        let restored = tx
            .backend
            .read("fixture-service", "Proxies")
            .unwrap()
            .configuration
            .unwrap();
        assert_eq!(restored["ProxyAutoConfigEnable"], 1);
        assert_eq!(restored["ProxyAutoDiscoveryEnable"], 1);
        assert_eq!(
            restored["ProxyAutoConfigURLString"],
            "https://fixture.invalid/external.pac"
        );
        assert_eq!(restored["HTTPEnable"], 0);
        assert!(restored.get("HTTPPort").is_none());
    }
    #[test]
    fn integration_has_zero_writes() {
        let mut tx = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::IntegrationTest,
            "native-mac:fixture".into(),
        )
        .unwrap();
        assert!(
            tx.apply(vec![value("DNS", json!({"ServerAddresses":["127.0.0.1"]}))])
                .is_err()
        );
        assert!(tx.restore().is_err());
        assert_eq!(tx.backend.writes, 0);
    }
    #[test]
    fn preserves_external_edits_and_restores_dns() {
        let mut tx = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        let dns = value("DNS", json!({"ServerAddresses":["127.0.0.1"]}));
        let proxy = value("Proxies", json!({"HTTPEnable":1}));
        tx.apply(vec![dns, proxy.clone()]).unwrap();
        tx.backend.write(&value("Proxies", json!({"HTTPEnable":0}))).unwrap();
        assert_eq!(tx.restore().unwrap().len(), 1);
        assert_eq!(tx.backend.read("fixture-service", "DNS").unwrap().configuration, None);
        assert_eq!(
            tx.backend.read("fixture-service", "Proxies").unwrap().configuration,
            Some(json!({"HTTPEnable":0}))
        );
    }
    #[test]
    fn restores_owned_keys_while_preserving_external_dns_domain() {
        let mut tx = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        tx.backend
            .write(&value(
                "DNS",
                json!({"SearchDomains":["old.example"],"ServerAddresses":["192.0.2.1"]}),
            ))
            .unwrap();
        tx.apply(vec![value(
            "DNS",
            json!({"SearchDomains":["old.example"],"ServerAddresses":["127.0.0.1"]}),
        )])
        .unwrap();
        tx.backend
            .write(&value(
                "DNS",
                json!({"SearchDomains":["external.example"],"ServerAddresses":["127.0.0.1"]}),
            ))
            .unwrap();
        tx.restore().unwrap();
        assert_eq!(
            tx.backend.read("fixture-service", "DNS").unwrap().configuration,
            Some(json!({"SearchDomains":["external.example"],"ServerAddresses":["192.0.2.1"]}))
        );
    }
    struct FailingNetwork {
        inner: SimulatedNetwork,
        fail_at: usize,
    }
    impl NetworkBackend for FailingNetwork {
        fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue> {
            self.inner.read(service, protocol)
        }
        fn write(&mut self, value: &NetworkValue) -> Result<()> {
            if self.inner.writes + 1 == self.fail_at {
                self.inner.writes += 1;
                bail!("fixture write failure");
            }
            self.inner.write(value)
        }
    }
    #[test]
    fn failed_dns_transaction_restores_prior_proxy() {
        let mut tx = NetworkTransaction::new(
            FailingNetwork {
                inner: SimulatedNetwork::default(),
                fail_at: 2,
            },
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        assert!(
            tx.apply(vec![
                value("Proxies", json!({"HTTPEnable":1})),
                value("DNS", json!({"ServerAddresses":["127.0.0.1"]}))
            ])
            .is_err()
        );
        assert!(tx.ownership.changes.is_empty());
        assert_eq!(
            tx.backend.read("fixture-service", "Proxies").unwrap().configuration,
            None
        );
    }
    #[test]
    fn durable_checkpoint_precedes_write_and_restarts_restore() {
        let mut tx = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        let mut journal = vec![];
        tx.apply_with_checkpoint(
            vec![value("DNS", json!({"ServerAddresses":["127.0.0.1"]}))],
            |snapshot| {
                journal = serde_json::to_vec(snapshot)?;
                Ok(())
            },
        )
        .unwrap();
        let mut restarted =
            NetworkTransaction::new(tx.backend, NetworkMode::Interactive, "native-mac:fixture".into()).unwrap();
        restarted
            .adopt_snapshot(serde_json::from_slice(&journal).unwrap())
            .unwrap();
        restarted.restore().unwrap();
        assert_eq!(
            restarted.backend.read("fixture-service", "DNS").unwrap().configuration,
            None
        );
        let mut denied = NetworkTransaction::new(
            SimulatedNetwork::default(),
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        assert!(
            denied
                .apply_with_checkpoint(vec![value("DNS", json!({}))], |_| bail!("fixture disk failure"))
                .is_err()
        );
        assert_eq!(denied.backend.writes, 0);
    }
}

#[cfg(test)]
mod dns_restore_regressions {
    use super::*;
    struct PartialApply {
        prefs: SimulatedNetwork,
        active: Option<Value>,
        fail_restore: bool,
        removed: bool,
    }
    impl NetworkBackend for PartialApply {
        fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue> {
            if self.removed {
                return Err(MissingNetworkService.into());
            }
            self.prefs.read(service, protocol)
        }
        fn write(&mut self, value: &NetworkValue) -> Result<()> {
            self.prefs.write(value)?;
            if self.fail_restore
                && value
                    .configuration
                    .as_ref()
                    .is_none_or(|v| v["ServerAddresses"] == json!(["192.0.2.1"]))
            {
                self.fail_restore = false;
                bail!("fixture SC commit succeeded but apply failed");
            }
            self.active = value.configuration.clone();
            Ok(())
        }
    }
    #[test]
    fn failed_apply_is_retried_even_when_preferences_already_show_original_dns() {
        for original in [
            None,
            Some(json!({"ServerAddresses":["192.0.2.1"],"SearchDomains":["corp.example"]})),
        ] {
            let before = NetworkValue {
                service: "fixture".into(),
                protocol: "DNS".into(),
                configuration: original.clone(),
            };
            let mut backend = PartialApply {
                prefs: SimulatedNetwork::default(),
                active: original.clone(),
                fail_restore: false,
                removed: false,
            };
            backend.prefs.write(&before).unwrap();
            let desired = dns_configuration(&before, &["192.0.2.53".into()]).unwrap();
            let mut tx =
                NetworkTransaction::new(backend, NetworkMode::Interactive, "native-mac:fixture".into()).unwrap();
            tx.apply(vec![desired.clone()]).unwrap();
            tx.backend.fail_restore = true;
            assert!(tx.restore().is_err());
            assert_eq!(tx.backend.active, desired.configuration);
            assert_eq!(tx.backend.read("fixture", "DNS").unwrap(), before);
            let persisted: OwnershipSnapshot =
                serde_json::from_slice(&serde_json::to_vec(&tx.ownership).unwrap()).unwrap();
            let mut reopened =
                NetworkTransaction::new(tx.backend, NetworkMode::Interactive, "native-mac:fixture".into()).unwrap();
            reopened.adopt_snapshot(persisted).unwrap();
            reopened.restore().unwrap();
            assert_eq!(reopened.backend.active, original);
            assert!(reopened.ownership.changes.is_empty());
        }
    }
    #[test]
    fn removed_service_is_not_recreated_during_restore() {
        let backend = PartialApply {
            prefs: SimulatedNetwork::default(),
            active: None,
            fail_restore: false,
            removed: false,
        };
        let mut tx = NetworkTransaction::new(backend, NetworkMode::Interactive, "native-mac:fixture".into()).unwrap();
        let before = tx.backend.read("fixture", "DNS").unwrap();
        tx.apply(vec![dns_configuration(&before, &["192.0.2.53".into()]).unwrap()])
            .unwrap();
        tx.backend.removed = true;
        let writes = tx.backend.prefs.writes;
        tx.restore().unwrap();
        assert!(tx.ownership.changes.is_empty());
        assert_eq!(tx.backend.prefs.writes, writes);
    }
    #[test]
    fn unavailable_localhost_resolvers_are_rejected_before_write() {
        for value in [
            "127.0.0.1",
            "::1",
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "ff02::1",
            "255.255.255.255",
            "::ffff:127.0.0.1",
        ] {
            assert!(validate_dns_servers(&[value.into()]).is_err());
        }
        validate_dns_servers(&["192.168.1.1".into(), "2001:db8::53".into()]).unwrap();
    }
}
