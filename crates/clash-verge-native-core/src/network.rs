use crate::platform::{
    MacNetwork, NetworkBackend, NetworkMode, NetworkTransaction, OwnershipSnapshot, dns_configuration,
    proxy_configuration,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub trait NetworkAccess: NetworkBackend {
    fn authorize_interactive(&mut self) -> Result<()>;
    fn services(&mut self) -> Result<Vec<String>>;
}
impl NetworkAccess for MacNetwork {
    fn authorize_interactive(&mut self) -> Result<()> {
        MacNetwork::authorize_interactive(self)
    }
    fn services(&mut self) -> Result<Vec<String>> {
        MacNetwork::services(self)
    }
}
pub struct NetworkSession<B = MacNetwork> {
    pub transaction: NetworkTransaction<B>,
    journal: PathBuf,
    pub proxy_on: bool,
    pub dns_servers: Vec<String>,
    mode: NetworkMode,
    recovery_required: bool,
}
impl NetworkSession<MacNetwork> {
    pub fn open_owned(directory: &Path, mode: NetworkMode, owner: String) -> Result<Self> {
        let mut transaction = NetworkTransaction::new(MacNetwork::new(mode), mode, owner)?;
        let journal = directory.join("network-ownership.json");
        if journal.exists() {
            let metadata = std::fs::symlink_metadata(&journal)?;
            if metadata.file_type().is_symlink() || metadata.len() > 65536 {
                bail!("invalid network ownership journal");
            }
            let snapshot: OwnershipSnapshot = serde_json::from_slice(&std::fs::read(&journal)?)?;
            if !snapshot.changes.is_empty() || snapshot.owner == transaction.ownership.owner {
                transaction.adopt_snapshot(snapshot)?;
            }
        }
        Ok(Self {
            transaction,
            journal,
            proxy_on: false,
            dns_servers: vec![],
            mode,
            recovery_required: false,
        })
    }
}
impl<B: NetworkAccess> NetworkSession<B> {
    #[cfg(test)]
    pub(crate) fn fixture(backend: B, journal: PathBuf) -> Self {
        Self {
            transaction: NetworkTransaction::new(backend, NetworkMode::Interactive, "native-mac:fixture".into())
                .unwrap(),
            journal,
            proxy_on: false,
            dns_servers: vec![],
            mode: NetworkMode::Interactive,
            recovery_required: false,
        }
    }
    pub fn state(&self) -> Value {
        let recovery = self.recovery_required
            || (!self.transaction.ownership.changes.is_empty() && !self.proxy_on && self.dns_servers.is_empty());
        json!({"systemProxy":if self.mode==NetworkMode::IntegrationTest {"unavailable"} else if recovery {"failed"} else if self.proxy_on {"on"} else {"off"},"dns":if recovery {"failed"} else if self.dns_servers.is_empty() {"off"} else {"on"},"tun":"off","recoveryRequired":recovery})
    }
    pub fn configure(&mut self, proxy: bool, dns: Vec<String>, port: u16) -> Result<()> {
        self.reconcile(proxy, dns, port)
    }
    fn reconcile(&mut self, proxy: bool, dns: Vec<String>, port: u16) -> Result<()> {
        let journal = self.journal.clone();
        self.reconcile_with_checkpoint(proxy, dns, port, &mut |snapshot| persist(&journal, snapshot))
    }
    fn reconcile_with_checkpoint(
        &mut self,
        proxy: bool,
        dns: Vec<String>,
        port: u16,
        checkpoint: &mut impl FnMut(&OwnershipSnapshot) -> Result<()>,
    ) -> Result<()> {
        if self.mode == NetworkMode::IntegrationTest {
            bail!("integrationTest forbids OS networking");
        }
        if proxy && port == 0 {
            bail!("invalid proxy target");
        }
        crate::platform::validate_dns_servers(&dns)?;
        self.transaction.backend.authorize_interactive()?;
        if !self.transaction.ownership.changes.is_empty() || self.recovery_required {
            self.restore_with_checkpoint(checkpoint)?;
        }
        // A previous successful state cannot survive restoration or a failed new transaction.
        self.proxy_on = false;
        self.dns_servers.clear();
        let mut desired = vec![];
        for service in self.transaction.backend.services()? {
            if proxy {
                let original = self.transaction.backend.read(&service, "Proxies")?;
                desired.push(proxy_configuration(&original, port)?);
            }
            if !dns.is_empty() {
                let original = self.transaction.backend.read(&service, "DNS")?;
                desired.push(dns_configuration(&original, &dns)?);
            }
        }
        if desired.is_empty() && (proxy || !dns.is_empty()) {
            bail!("no enabled network services found");
        }
        let outcome = self.transaction.apply_with_checkpoint(desired, &mut *checkpoint);
        let saved = checkpoint(&self.transaction.ownership);
        if let Err(error) = saved {
            let recovered = self.transaction.restore();
            let recovery_saved = checkpoint(&self.transaction.ownership);
            self.recovery_required =
                recovered.is_err() || recovery_saved.is_err() || !self.transaction.ownership.changes.is_empty();
            return Err(error.context(format!("network checkpoint failed; apply: {outcome:?}; rollback: {recovered:?}; recovery journal: {recovery_saved:?}")));
        }
        if let Err(error) = outcome {
            self.recovery_required = !self.transaction.ownership.changes.is_empty();
            return Err(error);
        }
        self.recovery_required = false;
        self.proxy_on = proxy;
        self.dns_servers = dns;
        Ok(())
    }
    pub fn restore(&mut self) -> Result<()> {
        let journal = self.journal.clone();
        self.restore_with_checkpoint(&mut |snapshot| persist(&journal, snapshot))
    }
    fn restore_with_checkpoint(&mut self, checkpoint: &mut impl FnMut(&OwnershipSnapshot) -> Result<()>) -> Result<()> {
        if self.mode == NetworkMode::IntegrationTest {
            if self.transaction.ownership.changes.is_empty() {
                return Ok(());
            }
            bail!("integrationTest forbids OS networking");
        }
        self.proxy_on = false;
        self.dns_servers.clear();
        let restored = if self.transaction.ownership.changes.is_empty() {
            Ok(vec![])
        } else {
            self.transaction.restore()
        };
        let saved = checkpoint(&self.transaction.ownership);
        self.recovery_required = restored.is_err() || saved.is_err() || !self.transaction.ownership.changes.is_empty();
        if let Err(error) = saved {
            return Err(error.context(format!("network recovery checkpoint failed; restoration: {restored:?}")));
        }
        restored.context("could not restore owned network settings")?;
        Ok(())
    }
}
fn persist(path: &Path, snapshot: &OwnershipSnapshot) -> Result<()> {
    let temporary = path.with_extension("pending");
    use std::os::unix::fs::MetadataExt;
    if let Ok(metadata) = std::fs::symlink_metadata(&temporary) {
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            bail!("unsafe stale network checkpoint");
        }
        std::fs::remove_file(&temporary)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .custom_flags(libc::O_NOFOLLOW)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let outcome = (|| {
        let bytes = serde_json::to_vec_pretty(snapshot)?;
        if bytes.len() > 65536 {
            bail!("network ownership journal exceeds 64 KiB");
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::File::open(path.parent().context("journal parent missing")?)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{NetworkValue, SimulatedNetwork};
    #[derive(Default)]
    struct MockNetwork {
        inner: SimulatedNetwork,
        fail_dns: bool,
        restore_conflict: bool,
    }
    impl NetworkBackend for MockNetwork {
        fn read(&mut self, service: &str, protocol: &str) -> Result<NetworkValue> {
            self.inner.read(service, protocol)
        }
        fn write(&mut self, value: &NetworkValue) -> Result<()> {
            if value.protocol == "DNS" && self.fail_dns {
                self.fail_dns = false;
                bail!("fixture DNS write failure");
            }
            self.inner.write(value)
        }
        fn compare_and_write(&mut self, expected: &NetworkValue, desired: &NetworkValue) -> Result<bool> {
            if self.restore_conflict && desired.configuration.is_none() {
                self.restore_conflict = false;
                self.inner.write(&NetworkValue {
                    configuration: Some(json!({"ServerAddresses":["192.0.2.99"]})),
                    ..expected.clone()
                })?;
                return Ok(false);
            }
            if self.read(&expected.service, &expected.protocol)? != *expected {
                return Ok(false);
            }
            self.write(desired)?;
            Ok(true)
        }
    }
    impl NetworkAccess for MockNetwork {
        fn authorize_interactive(&mut self) -> Result<()> {
            Ok(())
        }
        fn services(&mut self) -> Result<Vec<String>> {
            Ok(vec!["fixture-service".into()])
        }
    }
    fn session() -> NetworkSession<MockNetwork> {
        NetworkSession {
            transaction: NetworkTransaction::new(
                MockNetwork::default(),
                NetworkMode::Interactive,
                "native-mac:fixture".into(),
            )
            .unwrap(),
            journal: "/fixture/not-opened".into(),
            proxy_on: false,
            dns_servers: vec![],
            mode: NetworkMode::Interactive,
            recovery_required: false,
        }
    }
    #[test]
    fn crashed_checkpoint_is_replaced_but_symlink_checkpoint_is_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.work/network-checkpoints")
            .join(crate::unique_id());
        std::fs::create_dir_all(&directory).unwrap();
        let journal = directory.join("network-ownership.json");
        let temporary = journal.with_extension("pending");
        std::fs::write(&temporary, b"interrupted").unwrap();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).unwrap();
        let snapshot = OwnershipSnapshot {
            owner: "native-mac:fixture".into(),
            changes: vec![],
        };
        persist(&journal, &snapshot).unwrap();
        assert!(!temporary.exists());
        let outside = directory.join("preserved");
        std::fs::write(&outside, b"preserved").unwrap();
        symlink(&outside, &temporary).unwrap();
        assert!(persist(&journal, &snapshot).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"preserved");
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn dns_failure_after_proxy_restoration_does_not_report_previous_on() {
        let mut session = session();
        session
            .reconcile_with_checkpoint(true, vec![], 7890, &mut |_| Ok(()))
            .unwrap();
        assert!(session.proxy_on);
        session.transaction.backend.fail_dns = true;
        assert!(
            session
                .reconcile_with_checkpoint(true, vec!["192.0.2.53".into()], 7890, &mut |_| Ok(()))
                .is_err()
        );
        assert!(!session.proxy_on);
        assert!(session.dns_servers.is_empty());
        assert_eq!(session.state()["systemProxy"], "off");
        assert_eq!(
            session
                .transaction
                .backend
                .read("fixture-service", "Proxies")
                .unwrap()
                .configuration,
            None
        );
    }
    #[test]
    fn checkpoint_failure_before_dns_write_rolls_back_proxy() {
        let mut session = session();
        let mut checkpoints = 0;
        let outcome = session.reconcile_with_checkpoint(true, vec!["192.0.2.53".into()], 7890, &mut |_| {
            checkpoints += 1;
            if checkpoints == 2 {
                bail!("fixture disk failure");
            }
            Ok(())
        });
        assert!(outcome.is_err());
        assert!(!session.proxy_on);
        assert!(session.dns_servers.is_empty());
        assert_eq!(
            session
                .transaction
                .backend
                .read("fixture-service", "Proxies")
                .unwrap()
                .configuration,
            None
        );
        assert!(session.transaction.ownership.changes.is_empty());
    }
    #[test]
    fn final_checkpoint_failure_rolls_back_completed_apply() {
        let mut session = session();
        let mut checkpoints = 0;
        assert!(
            session
                .reconcile_with_checkpoint(true, vec!["192.0.2.53".into()], 7890, &mut |_| {
                    checkpoints += 1;
                    if checkpoints == 3 {
                        bail!("fixture final journal failure");
                    }
                    Ok(())
                })
                .is_err()
        );
        assert!(!session.proxy_on);
        assert!(session.dns_servers.is_empty());
        for protocol in ["Proxies", "DNS"] {
            assert_eq!(
                session
                    .transaction
                    .backend
                    .read("fixture-service", protocol)
                    .unwrap()
                    .configuration,
                None
            );
        }
    }
    #[test]
    fn recovery_checkpoint_failure_reports_failed_even_after_os_restore() {
        let mut session = session();
        session
            .reconcile_with_checkpoint(true, vec![], 7890, &mut |_| Ok(()))
            .unwrap();
        assert!(
            session
                .restore_with_checkpoint(&mut |_| bail!("fixture recovery journal failure"))
                .is_err()
        );
        assert!(!session.proxy_on);
        assert_eq!(session.state()["systemProxy"], "failed");
        assert_eq!(session.state()["recoveryRequired"], true);
        assert_eq!(
            session
                .transaction
                .backend
                .read("fixture-service", "Proxies")
                .unwrap()
                .configuration,
            None
        );
    }
    #[test]
    fn restore_cas_conflict_retains_journal_and_preserves_external_dns() {
        let mut session = session();
        session
            .reconcile_with_checkpoint(false, vec!["192.0.2.53".into()], 7890, &mut |_| Ok(()))
            .unwrap();
        session.transaction.backend.restore_conflict = true;
        assert!(session.restore_with_checkpoint(&mut |_| Ok(())).is_err());
        assert!(session.dns_servers.is_empty());
        assert_eq!(session.state()["dns"], "failed");
        assert!(!session.transaction.ownership.changes.is_empty());
        session.restore_with_checkpoint(&mut |_| Ok(())).unwrap();
        assert_eq!(
            session
                .transaction
                .backend
                .read("fixture-service", "DNS")
                .unwrap()
                .configuration,
            Some(json!({"ServerAddresses":["192.0.2.99"]}))
        );
        assert_eq!(session.state()["dns"], "off");
    }
}
