//! Explicit, candidate-only mihomo update preparation for Apple Silicon macOS.
use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const STABLE_VERSION_URL: &str = "https://github.com/MetaCubeX/mihomo/releases/latest/download/version.txt";
const ALPHA_BASE_URL: &str = "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha";
const RELEASE_BASE_URL: &str = "https://github.com/MetaCubeX/mihomo/releases/download";
const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestReport {
    pub channel: &'static str,
    pub version: String,
    pub version_url: String,
    pub package_url: String,
    pub asset: &'static str,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedReport {
    pub latest: LatestReport,
    pub candidate_path: PathBuf,
    pub archive_sha256: String,
    pub binary_sha256: String,
    pub binary_bytes: u64,
    pub activated: bool,
}
fn usable_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version.as_bytes()[0].is_ascii_alphanumeric()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn package_url(alpha: bool, version: &str) -> Result<String> {
    if !usable_version(version) {
        bail!("Invalid mihomo version token");
    }
    let base = if alpha {
        ALPHA_BASE_URL.to_owned()
    } else {
        format!("{RELEASE_BASE_URL}/{version}")
    };
    Ok(format!("{base}/mihomo-darwin-arm64-go122-{version}.gz"))
}
fn client(timeout: u64) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .timeout(Duration::from_secs(timeout))
        .build()?)
}
async fn fetch(url: &str, timeout: u64, maximum: usize) -> Result<Vec<u8>> {
    let mut response = client(timeout)?.get(url).send().await?.error_for_status()?;
    if response.content_length().is_some_and(|n| n > maximum as u64) {
        bail!("Mihomo response exceeds size limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            bail!("Mihomo response exceeds size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn check_latest(alpha: bool) -> Result<LatestReport> {
    let version_url = if alpha {
        format!("{ALPHA_BASE_URL}/version.txt")
    } else {
        STABLE_VERSION_URL.to_owned()
    };
    let bytes = fetch(&version_url, 20, 4096)
        .await
        .context("Failed to check the official mihomo version")?;
    let version = std::str::from_utf8(&bytes)
        .context("Mihomo version response is not UTF-8")?
        .trim()
        .to_owned();
    let package_url = package_url(alpha, &version)?;
    Ok(LatestReport {
        channel: if alpha { "alpha" } else { "stable" },
        version,
        version_url,
        package_url,
        asset: "mihomo-darwin-arm64-go122",
    })
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn decode_archive_with_limit(archive: &[u8], maximum: u64) -> Result<Vec<u8>> {
    if archive.len() > MAX_ARCHIVE_BYTES {
        bail!("Mihomo archive exceeds 64 MiB limit");
    }
    let mut bytes = Vec::new();
    GzDecoder::new(archive)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .context("Invalid mihomo gzip archive")?;
    if bytes.len() as u64 > maximum {
        bail!("Mihomo binary exceeds size limit");
    }
    validate_binary(&bytes)?;
    Ok(bytes)
}
fn validate_binary(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 32
        || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || bytes[4..8] != [0x0c, 0, 0, 1]
        || bytes[12..16] != [2, 0, 0, 0]
    {
        bail!("Mihomo candidate is not an arm64 Mach-O executable");
    }
    Ok(())
}
fn private_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
struct OwnedStaging {
    path: PathBuf,
    published: bool,
}
impl Drop for OwnedStaging {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
fn update_directory(root: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().context("Repository root is unavailable")?;
    let work = root.join(".work");
    ensure_directory(&work)?;
    let directory = work.join("core-updates");
    ensure_directory(&directory)?;
    if directory.canonicalize()? != directory {
        bail!("Mihomo update directory must not redirect through a symlink");
    }
    Ok(directory)
}
fn ensure_directory(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(()),
        Ok(_) => bail!("Mihomo update directory must be a real directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new().mode(0o700).create(path)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
fn staging(directory: &Path) -> Result<OwnedStaging> {
    for _ in 0..32 {
        let id = GENERATION.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = directory.join(format!(".stage-{}-{nanos}-{id}", std::process::id()));
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(OwnedStaging { path, published: false }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("Failed to reserve an owned mihomo staging directory")
}
fn run_bounded(mut command: Command, timeout: Duration) -> Result<()> {
    command.env_clear().stdin(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().context("Failed to run mihomo candidate validation")?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => bail!("Mihomo candidate validation failed"),
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Mihomo candidate validation timed out");
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        }
    }
}
fn version_matches(output: &str, expected: &str) -> bool {
    let words: Vec<_> = output.split_whitespace().collect();
    words.len() >= 5
        && words[0] == "Mihomo"
        && words[1] == "Meta"
        && words[2] == expected
        && words[3] == "darwin"
        && words[4] == "arm64"
}
fn stage_verified(root: &Path, latest: LatestReport, archive: Vec<u8>) -> Result<StagedReport> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        bail!("Mihomo staging requires Apple Silicon macOS");
    }
    let directory = update_directory(root)?;
    let mut staged = staging(&directory)?;
    let binary = decode_archive_with_limit(&archive, MAX_BINARY_BYTES)?;
    let binary_path = staged.path.join("mihomo");
    private_file(&binary_path, &binary, 0o700)?;
    let mut sign = Command::new("/usr/bin/codesign");
    sign.args(["--force", "--sign", "-"])
        .arg(&binary_path)
        .stdout(Stdio::null());
    run_bounded(sign, Duration::from_secs(30)).context("Failed to ad-hoc sign the mihomo candidate")?;
    let mut verify = Command::new("/usr/bin/codesign");
    verify
        .args(["--verify", "--strict"])
        .arg(&binary_path)
        .stdout(Stdio::null());
    run_bounded(verify, Duration::from_secs(30)).context("Mihomo candidate signature verification failed")?;
    let output_path = staged.path.join("version-output");
    let output_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output_path)?;
    let mut command = Command::new(&binary_path);
    command.arg("-v").current_dir(&staged.path).stdout(output_file);
    run_bounded(command, Duration::from_secs(20))?;
    if std::fs::metadata(&output_path)?.len() > 65536 {
        bail!("Mihomo candidate version output exceeds size limit");
    }
    let output = std::fs::read_to_string(&output_path)?;
    if !version_matches(&output, &latest.version) {
        bail!("Mihomo candidate reports an unexpected version or platform");
    }
    std::fs::remove_file(output_path)?;
    if std::fs::metadata(&binary_path)?.len() > MAX_BINARY_BYTES {
        bail!("Signed mihomo candidate exceeds size limit");
    }
    let signed = std::fs::read(&binary_path)?;
    validate_binary(&signed)?;
    File::open(&binary_path)?.sync_all()?;
    let suffix = staged
        .path
        .file_name()
        .context("Missing staging identity")?
        .to_string_lossy()
        .trim_start_matches('.')
        .to_owned();
    let destination = directory.join(format!("{}-{}-{suffix}", latest.channel, latest.version));
    let report = StagedReport {
        latest,
        candidate_path: destination.join("mihomo"),
        archive_sha256: digest(&archive),
        binary_sha256: digest(&signed),
        binary_bytes: signed.len() as u64,
        activated: false,
    };
    private_file(
        &staged.path.join("candidate.json"),
        &serde_json::to_vec_pretty(&report)?,
        0o600,
    )?;
    std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o700))?;
    File::open(&staged.path)?.sync_all()?;
    if destination.exists() {
        bail!("Mihomo candidate destination is already occupied");
    }
    std::fs::rename(&staged.path, &destination).context("Failed to atomically publish the mihomo candidate")?;
    staged.published = true;
    File::open(directory)?.sync_all()?;
    Ok(report)
}
pub async fn stage_latest(root: &Path, alpha: bool) -> Result<StagedReport> {
    // Validate containment before any download; the fixed engine and artifact lock are never opened.
    update_directory(root)?;
    let latest = check_latest(alpha).await?;
    let archive = fetch(&latest.package_url, 300, MAX_ARCHIVE_BYTES)
        .await
        .context("Failed to download the official mihomo candidate")?;
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || stage_verified(&root, latest, archive)).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_assets_and_versions_are_pinned() {
        assert_eq!(
            package_url(false, "v1.19.32").unwrap(),
            "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-darwin-arm64-go122-v1.19.32.gz"
        );
        assert!(
            package_url(true, "alpha-example")
                .unwrap()
                .contains("Prerelease-Alpha/mihomo-darwin-arm64-go122-alpha-example.gz")
        );
        for v in ["", "..", "v1/other", "v1?secret", "<html>", "v1 extra", &"v".repeat(65)] {
            assert!(package_url(false, v).is_err());
        }
        assert!(version_matches(
            "Mihomo Meta v1.19.32 darwin arm64 with go1.22",
            "v1.19.32"
        ));
        assert!(!version_matches("Mihomo Meta v1.19.31 darwin arm64", "v1.19.32"));
        assert!(!version_matches("Mihomo Meta v1.19.32 linux arm64", "v1.19.32"));
    }
    #[test]
    fn malformed_archives_wrong_architecture_and_expansion_are_rejected() -> Result<()> {
        assert!(decode_archive_with_limit(b"not gzip", 128).is_err());
        let mut bytes = vec![0; 64];
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(&[0x0c, 0, 0, 1]);
        bytes[12..16].copy_from_slice(&[2, 0, 0, 0]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&bytes)?;
        let archive = gz.finish()?;
        assert_eq!(decode_archive_with_limit(&archive, 64)?, bytes);
        assert!(decode_archive_with_limit(&archive, 63).is_err());
        bytes[4] = 7;
        assert!(validate_binary(&bytes).is_err());
        Ok(())
    }
    #[test]
    fn invalid_candidate_never_replaces_existing_files() -> Result<()> {
        let tests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.work/core-updates/tests");
        std::fs::create_dir_all(&tests)?;
        let owned = staging(&tests.canonicalize()?)?;
        let directory = update_directory(&owned.path)?;
        let existing = directory.join("existing-candidate");
        private_file(&existing, b"previous verified core", 0o600)?;
        let latest = LatestReport {
            channel: "stable",
            version: "v1.19.32".into(),
            version_url: STABLE_VERSION_URL.into(),
            package_url: package_url(false, "v1.19.32")?,
            asset: "mihomo-darwin-arm64-go122",
        };
        assert!(stage_verified(&owned.path, latest, b"invalid gzip".to_vec()).is_err());
        assert_eq!(std::fs::read(&existing)?, b"previous verified core");
        assert_eq!(std::fs::read_dir(&directory)?.count(), 1);
        Ok(())
    }
    #[test]
    fn owned_cleanup_preserves_other_files_and_rejects_symlinks() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.work/core-updates/tests");
        std::fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let owned = staging(&root)?;
        let owned_path = owned.path.clone();
        private_file(&owned_path.join("owned"), b"sample", 0o600)?;
        let unrelated = root.join(format!("unrelated-{}", std::process::id()));
        private_file(&unrelated, b"keep", 0o600)?;
        drop(owned);
        assert!(!owned_path.exists());
        assert_eq!(std::fs::read(&unrelated)?, b"keep");
        std::fs::remove_file(unrelated)?;
        let owned = staging(&root)?;
        let link = owned.path.join("redirect");
        std::os::unix::fs::symlink(&root, &link)?;
        assert!(ensure_directory(&link).is_err());
        Ok(())
    }
}
