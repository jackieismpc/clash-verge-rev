use super::{HELPER_DATA, HelperStatus};
use crate::{network::NetworkSession, platform::NetworkMode};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct ProcessIdentity {
    pid: u32,
    pub uid: u32,
    start_seconds: u64,
    start_microseconds: u64,
}
impl ProcessIdentity {
    pub fn capture(pid: u32) -> Result<(Self, u32)> {
        if pid == 0 || pid > i32::MAX as u32 {
            bail!("invalid owned process ID");
        }
        let mut value = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let length = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                value.as_mut_ptr().cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if length != std::mem::size_of::<libc::proc_bsdinfo>() as i32 {
            bail!("owned process no longer exists");
        }
        let value = unsafe { value.assume_init() };
        if value.pbi_status == libc::SZOMB {
            bail!("owned process has exited");
        }
        Ok((
            Self {
                pid,
                uid: value.pbi_uid,
                start_seconds: value.pbi_start_tvsec,
                start_microseconds: value.pbi_start_tvusec,
            },
            value.pbi_ppid,
        ))
    }
    pub fn alive(&self) -> bool {
        Self::capture(self.pid).is_ok_and(|(current, _)| current == *self)
    }
    pub fn terminate(&self) -> Result<()> {
        if !self.alive() {
            return Ok(());
        }
        if unsafe { libc::kill(self.pid as i32, libc::SIGTERM) } != 0 && self.alive() {
            return Err(std::io::Error::last_os_error().into());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if self.alive() {
            bail!("owned engine did not exit; recovery identity retained");
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
pub(crate) struct LeaseRecord {
    pub gui_token: [u32; 8],
    pub core: ProcessIdentity,
}
pub(crate) struct NetworkLease<B = crate::platform::MacNetwork> {
    pub uid: u32,
    pub owner: String,
    pub record: LeaseRecord,
    pub session: NetworkSession<B>,
}
impl<B: crate::network::NetworkAccess> NetworkLease<B> {
    pub fn state(&self) -> Value {
        let mut value = self.session.state();
        value["dnsServers"] = json!(self.session.dns_servers);
        value["owner"] = json!(self.owner);
        value["guiPid"] = json!(self.record.gui_token[5]);
        value
    }
    pub fn restore(&mut self) -> Result<()> {
        self.session.restore()
    }
}
pub(crate) fn monitor<B: crate::network::NetworkAccess>(
    lease: &mut NetworkLease<B>,
    gui_alive: bool,
    core_alive: bool,
    stop: impl FnOnce(&ProcessIdentity) -> Result<()>,
) -> Result<bool> {
    if gui_alive && core_alive {
        return Ok(false);
    }
    let restore = lease.restore();
    let terminate = if !gui_alive && core_alive {
        stop(&lease.record.core)
    } else {
        Ok(())
    };
    restore?;
    terminate?;
    Ok(true)
}
pub(crate) fn directory(uid: u32) -> PathBuf {
    Path::new(HELPER_DATA).join(uid.to_string())
}
pub(crate) fn valid_owner(owner: &str) -> bool {
    owner.starts_with("native-mac:")
        && (16..=128).contains(&owner.len())
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b"-:_".contains(&b))
}
pub(crate) fn safe_json(path: &Path, maximum: u64) -> Result<Value> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.len() > maximum
    {
        bail!("invalid root-owned recovery record");
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut bytes = vec![];
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        bail!("recovery record exceeds limit");
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn recover_directory(directory: &Path) -> Result<()> {
    let network = directory.join("network-ownership.json");
    if network.exists() {
        let value = safe_json(&network, 65536)?;
        let owner = value["owner"]
            .as_str()
            .context("network recovery owner missing")?
            .to_owned();
        if !valid_owner(&owner) {
            bail!("invalid network recovery owner");
        }
        let mut session = NetworkSession::open_owned(directory, NetworkMode::Interactive, owner)?;
        session.restore()?;
    }
    for filename in ["engine-owner.json", "network-process.json"] {
        let path = directory.join(filename);
        if path.exists() {
            let record: LeaseRecord = serde_json::from_value(safe_json(&path, 4096)?)?;
            // An orphaned privileged engine owns routes; its exact process incarnation must exit.
            if record.core.uid == 0 || !super::macos::gui_alive(&record.gui_token) {
                record.core.terminate()?;
            }
            fs::remove_file(path)?;
        }
    }
    Ok(())
}
pub(crate) fn recover_all() -> Result<()> {
    for entry in fs::read_dir(HELPER_DATA)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().is_none_or(|name| name.parse::<u32>().is_err()) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            bail!("invalid helper recovery directory");
        }
        recover_directory(&entry.path())?;
    }
    Ok(())
}
pub(crate) fn attach_status(mut status: HelperStatus, network: Option<&NetworkLease>) -> HelperStatus {
    status.network = Some(
        network
            .map(NetworkLease::state)
            .unwrap_or_else(|| json!({"systemProxy":"off","dns":"off","dnsServers":[],"recoveryRequired":false})),
    );
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        network::NetworkAccess,
        platform::{NetworkBackend, NetworkValue, SimulatedNetwork},
    };
    #[derive(Default)]
    struct Backend(SimulatedNetwork);
    impl NetworkBackend for Backend {
        fn read(&mut self, s: &str, p: &str) -> Result<NetworkValue> {
            self.0.read(s, p)
        }
        fn write(&mut self, v: &NetworkValue) -> Result<()> {
            self.0.write(v)
        }
    }
    impl NetworkAccess for Backend {
        fn authorize_interactive(&mut self) -> Result<()> {
            Ok(())
        }
        fn services(&mut self) -> Result<Vec<String>> {
            Ok(vec!["fixture".into()])
        }
    }
    #[test]
    fn lost_gui_or_core_restores_dhcp_without_touching_other_dns_fields() {
        for (gui_alive, core_alive) in [(false, true), (true, false), (false, false)] {
            let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.work/lease-tests")
                .join(format!("{}-{gui_alive}-{core_alive}", std::process::id()));
            fs::create_dir_all(&directory).unwrap();
            let mut backend = Backend::default();
            backend.0.values.insert(
                ("fixture".into(), "DNS".into()),
                Some(json!({"SearchDomains":["corp.example"]})),
            );
            let mut session = NetworkSession::fixture(backend, directory.join("network.json"));
            session.configure(true, vec!["192.0.2.53".into()], 7890).unwrap();
            let mut lease = NetworkLease {
                uid: 501,
                owner: "native-mac:fixture".into(),
                record: LeaseRecord {
                    gui_token: [0; 8],
                    core: ProcessIdentity {
                        pid: 1,
                        uid: 501,
                        start_seconds: 1,
                        start_microseconds: 2,
                    },
                },
                session,
            };
            let mut stopped = false;
            assert!(
                monitor(&mut lease, gui_alive, core_alive, |_| {
                    stopped = true;
                    Ok(())
                })
                .unwrap()
            );
            assert_eq!(stopped, !gui_alive && core_alive);
            assert_eq!(
                lease
                    .session
                    .transaction
                    .backend
                    .read("fixture", "DNS")
                    .unwrap()
                    .configuration,
                Some(json!({"SearchDomains":["corp.example"]}))
            );
            assert!(lease.session.transaction.ownership.changes.is_empty());
            fs::remove_dir_all(directory).unwrap();
        }
    }
    #[test]
    fn current_gui_and_engine_do_not_release_their_lease() {
        let session = NetworkSession::fixture(Backend::default(), "not-opened".into());
        let mut lease = NetworkLease {
            uid: 501,
            owner: "native-mac:fixture".into(),
            record: LeaseRecord {
                gui_token: [0; 8],
                core: ProcessIdentity {
                    pid: 1,
                    uid: 501,
                    start_seconds: 1,
                    start_microseconds: 2,
                },
            },
            session,
        };
        assert!(!monitor(&mut lease, true, true, |_| panic!("must not stop live owner")).unwrap());
    }
    #[test]
    fn pid_reuse_does_not_signal_the_new_process() {
        let mut child = std::process::Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let (identity, _) = ProcessIdentity::capture(child.id()).unwrap();
        assert!(identity.alive());
        let mut old = identity;
        old.start_seconds = old.start_seconds.saturating_sub(1);
        assert!(!old.alive());
        old.terminate().unwrap();
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
