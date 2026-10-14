use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Specification {
    pub binary: PathBuf,
    pub home: PathBuf,
    pub config: PathBuf,
    pub integration: bool,
}
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn terminate(_: i32) {
    STOP.store(true, Ordering::Release);
}
pub fn supervise(input: &str) -> Result<()> {
    if unsafe { libc::geteuid() } == 0 {
        bail!("unprivileged engine supervisor must not run as root");
    }
    if input.len() > 65536 {
        bail!("supervisor specification exceeds limit");
    }
    let spec: Specification = serde_json::from_str(input)?;
    if !spec.binary.is_absolute() || !spec.home.is_absolute() || !spec.config.is_absolute() || !spec.home.is_dir() {
        bail!("supervisor paths must be absolute");
    }
    unsafe {
        libc::signal(libc::SIGTERM, terminate as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, terminate as *const () as libc::sighandler_t);
    }
    std::thread::spawn(|| {
        let mut buffer = [0; 256];
        loop {
            match std::io::stdin().read(&mut buffer) {
                Ok(0) | Err(_) => {
                    STOP.store(true, Ordering::Release);
                    break;
                }
                Ok(_) => (),
            }
        }
    });
    let mut command = Command::new(&spec.binary);
    command
        .args(["-d"])
        .arg(&spec.home)
        .arg("-f")
        .arg(&spec.config)
        .current_dir(&spec.home)
        .stdin(Stdio::null());
    if spec.integration {
        command
            .env_clear()
            .env("HOME", &spec.home)
            .env("TMPDIR", spec.home.join("tmp"));
        unsafe {
            libc::umask(0o077);
        }
    }
    let socket = spec.home.join("c.sock");
    if socket.exists() {
        bail!("owned controller path is already occupied");
    }
    let mut child = command.spawn().context("start owned engine")?;
    while !STOP.load(Ordering::Acquire) {
        if let Some(status) = child.try_wait()? {
            if let Ok(metadata) = std::fs::symlink_metadata(&socket)
                && metadata.file_type().is_socket()
                && metadata.uid() == unsafe { libc::geteuid() }
            {
                let _ = std::fs::remove_file(&socket);
            }
            if status.success() {
                return Ok(());
            }
            bail!("owned engine exited ({status})");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // The pipe belongs to the GUI process; EOF also covers SIGKILL and crashes.
    if child.try_wait()?.is_none() {
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait()?.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    child.wait()?;
    if let Ok(metadata) = std::fs::symlink_metadata(&socket)
        && metadata.file_type().is_socket()
        && metadata.uid() == unsafe { libc::geteuid() }
    {
        std::fs::remove_file(&socket)?;
    }
    Ok(())
}
