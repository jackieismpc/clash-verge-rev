use crate::{config::ExecutionMode, helper, platform::validate_dns_servers};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct NetworkClient {
    owner: String,
    mode: ExecutionMode,
    pub proxy_on: bool,
    pub dns_servers: Vec<String>,
    recovery: bool,
    pub core_pid: u32,
}
impl NetworkClient {
    pub fn new(directory: &Path, mode: ExecutionMode) -> Result<Self> {
        let digest = Sha256::digest(directory.canonicalize()?.as_os_str().as_encoded_bytes());
        let owner = format!(
            "native-mac:{}",
            digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        );
        Ok(Self {
            owner,
            mode,
            proxy_on: false,
            dns_servers: vec![],
            recovery: mode == ExecutionMode::Interactive && legacy_recovery_required(directory)?,
            core_pid: 0,
        })
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn state(&self) -> Value {
        json!({"systemProxy":if self.mode==ExecutionMode::IntegrationTest {"unavailable"} else if self.recovery {"failed"} else if self.proxy_on {"on"} else {"off"},"dns":if self.recovery {"failed"} else if self.dns_servers.is_empty(){"off"}else{"on"},"tun":"off","recoveryRequired":self.recovery})
    }
    fn accept(&mut self, status: helper::HelperStatus) -> Result<()> {
        let network = status.network.context("helper omitted network state")?;
        if let Some(owner) = network["owner"].as_str()
            && (owner != self.owner || network["guiPid"].as_u64() != Some(std::process::id() as u64))
        {
            self.proxy_on = false;
            self.dns_servers.clear();
            self.recovery = false;
            bail!("system network is owned by another native instance");
        }
        self.proxy_on = network["systemProxy"] == "on";
        self.dns_servers = serde_json::from_value(network["dnsServers"].clone()).unwrap_or_default();
        self.recovery = network["recoveryRequired"] == true;
        Ok(())
    }
    fn set(&mut self, proxy: bool, dns_servers: Vec<String>, mixed_port: u16) -> Result<()> {
        if self.mode == ExecutionMode::IntegrationTest {
            bail!("integrationTest forbids system networking");
        }
        validate_dns_servers(&dns_servers)?;
        if !proxy && dns_servers.is_empty() {
            return self.restore();
        }
        let outcome = helper::request(
            &helper::HelperCommand::NetworkSet {
                owner: self.owner.clone(),
                mixed_port,
                core_pid: self.core_pid,
                proxy,
                dns_servers,
            },
            false,
        );
        match outcome {
            Ok(status) => self.accept(status),
            Err(error) => {
                self.recovery = true;
                Err(error)
            }
        }
    }
    pub fn set_proxy(&mut self, enabled: bool, port: u16) -> Result<()> {
        self.set(enabled, if enabled { self.dns_servers.clone() } else { vec![] }, port)
    }
    pub fn set_dns(&mut self, servers: Vec<String>, port: u16) -> Result<()> {
        self.set(self.proxy_on, servers, port)
    }
    pub fn refresh(&mut self) -> Result<()> {
        if !self.proxy_on && self.dns_servers.is_empty() && !self.recovery {
            return Ok(());
        }
        let status = helper::request(
            &helper::HelperCommand::Status,
            self.mode == ExecutionMode::IntegrationTest,
        )?;
        self.accept(status)
    }
    pub fn restore(&mut self) -> Result<()> {
        if !self.proxy_on && self.dns_servers.is_empty() && !self.recovery {
            return Ok(());
        }
        match helper::request(
            &helper::HelperCommand::NetworkRestore {
                owner: self.owner.clone(),
            },
            self.mode == ExecutionMode::IntegrationTest,
        ) {
            Ok(status) => self.accept(status),
            Err(error) => {
                self.recovery = true;
                Err(error)
            }
        }
    }
}

fn legacy_recovery_required(directory: &Path) -> Result<bool> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let path = directory.join("network-ownership.json");
    if !path.try_exists()? {
        return Ok(false);
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.len() > 65536
    {
        bail!("invalid legacy network recovery record");
    }
    let mut bytes = vec![];
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        bail!("legacy network recovery record exceeds limit");
    }
    let snapshot: crate::platform::OwnershipSnapshot = serde_json::from_slice(&bytes)?;
    Ok(!snapshot.changes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_ownership_requires_recovery_without_writing_os_or_losing_record() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.work/legacy-network")
            .join(crate::unique_id());
        std::fs::create_dir_all(&directory).unwrap();
        let journal = directory.join("network-ownership.json");
        let original = json!({"owner":"native-mac:fixture","changes":[{"before":{"service":"fixture","protocol":"DNS","configuration":null},"applied":{"service":"fixture","protocol":"DNS","configuration":{"ServerAddresses":["192.0.2.53"]}}}]}).to_string();
        std::fs::write(&journal, &original).unwrap();
        let network = NetworkClient::new(&directory, ExecutionMode::Interactive).unwrap();
        assert_eq!(network.state()["recoveryRequired"], true);
        assert_eq!(std::fs::read_to_string(&journal).unwrap(), original);
        let integration = NetworkClient::new(&directory, ExecutionMode::IntegrationTest).unwrap();
        assert_eq!(integration.state()["systemProxy"], "unavailable");
        assert_eq!(std::fs::read_to_string(journal).unwrap(), original);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
