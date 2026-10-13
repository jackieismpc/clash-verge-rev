//! macOS networking ownership transactions. No constructor performs OS writes.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OwnershipSnapshot {
    pub owner: String,
    pub changes: Vec<OwnedChange>,
}

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
                changes.push(OwnedChange { before, applied: value });
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
        for change in self.ownership.changes.drain(..).rev() {
            match self.backend.read(&change.applied.service, &change.applied.protocol) {
                Ok(current) => {
                    let (desired, externally_changed) = restoration_value(&current, &change);
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
                                retained.push(change);
                            }
                        }
                    }
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

/// Keep search domains and all unrelated DNS fields; restore uses the exact original dictionary.
pub fn dns_configuration(original: &NetworkValue, servers: &[String]) -> Result<NetworkValue> {
    if original.protocol != "DNS" || servers.is_empty() {
        bail!("DNS requires at least one literal IP address");
    }
    for server in servers {
        server.parse::<std::net::IpAddr>()?;
    }
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

/// Closed helper protocol: no executable, shell command, arbitrary path or shared service channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperRequest {
    EnableTun { generation: u64 },
    DisableTun { generation: u64 },
}
pub trait TunHelper {
    fn ready(&self) -> bool;
    fn request(&mut self, owner: &str, request: HelperRequest) -> Result<()>;
}
pub struct TunSession<H> {
    pub helper: H,
    mode: NetworkMode,
    owner: String,
    active: Option<u64>,
}
impl<H: TunHelper> TunSession<H> {
    pub fn new(helper: H, mode: NetworkMode, owner: String) -> Result<Self> {
        if !owner.starts_with("native-mac:") || owner.len() < 16 {
            bail!("independent native owner identity required");
        }
        Ok(Self {
            helper,
            mode,
            owner,
            active: None,
        })
    }
    pub fn available(&self) -> bool {
        self.mode == NetworkMode::Interactive && self.helper.ready()
    }
    pub fn set(&mut self, enabled: bool, generation: u64) -> Result<()> {
        if self.mode == NetworkMode::IntegrationTest {
            bail!("integrationTest forbids helper writes");
        }
        if !self.helper.ready() {
            bail!("TUN unavailable: independently authorized native helper is not installed");
        }
        if let Some(active) = self.active
            && generation < active
        {
            bail!("stale TUN generation");
        }
        // Disable only the owned generation, never a later or unknown session.
        if !enabled && self.active != Some(generation) {
            bail!("TUN generation is not owned by this session");
        }
        self.helper.request(
            &self.owner,
            if enabled {
                HelperRequest::EnableTun { generation }
            } else {
                HelperRequest::DisableTun { generation }
            },
        )?;
        self.active = enabled.then_some(generation);
        Ok(())
    }
}
pub struct UnavailableNativeHelper;
impl TunHelper for UnavailableNativeHelper {
    fn ready(&self) -> bool {
        false
    }
    fn request(&mut self, _: &str, _: HelperRequest) -> Result<()> {
        bail!("native helper requires signing and explicit macOS administrator authorization")
    }
}

#[derive(Default)]
pub struct SimulatedNetwork {
    pub values: BTreeMap<(String, String), Option<Value>>,
    pub writes: usize,
}
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
    struct MockHelper {
        requests: Vec<HelperRequest>,
        fail: bool,
    }
    impl TunHelper for MockHelper {
        fn ready(&self) -> bool {
            true
        }
        fn request(&mut self, _: &str, request: HelperRequest) -> Result<()> {
            if self.fail {
                bail!("fixture failure");
            }
            self.requests.push(request);
            Ok(())
        }
    }
    #[test]
    fn tun_guard_generation_and_capability() {
        let mut session = TunSession::new(
            MockHelper {
                requests: vec![],
                fail: false,
            },
            NetworkMode::IntegrationTest,
            "native-mac:fixture".into(),
        )
        .unwrap();
        assert!(!session.available());
        assert!(session.set(true, 1).is_err());
        assert!(session.helper.requests.is_empty());
        let mut session = TunSession::new(
            MockHelper {
                requests: vec![],
                fail: false,
            },
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        session.helper.fail = true;
        assert!(session.set(true, 2).is_err());
        assert!(session.active.is_none());
        session.helper.fail = false;
        session.set(true, 2).unwrap();
        assert!(session.set(false, 1).is_err());
        session.helper.fail = true;
        assert!(session.set(false, 2).is_err());
        assert_eq!(session.active, Some(2));
        session.helper.fail = false;
        session.set(false, 2).unwrap();
        let native = TunSession::new(
            UnavailableNativeHelper,
            NetworkMode::Interactive,
            "native-mac:fixture".into(),
        )
        .unwrap();
        assert!(!native.available());
    }
}
