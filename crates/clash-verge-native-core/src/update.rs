use anyhow::{Context, Result, bail};
pub use clash_verge_core_update::{check_latest, stage_latest};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
/// Resolve only a published candidate owned by this private data directory.
pub fn validate_owned_candidate(data_dir: &Path, candidate: &Path) -> Result<PathBuf> {
    let directory = data_dir.canonicalize()?.join(".work/core-updates");
    if directory.canonicalize()? != directory {
        bail!("Update directory must not redirect through a symlink");
    }
    if candidate.file_name().and_then(|n| n.to_str()) != Some("mihomo") {
        bail!("Invalid owned candidate filename");
    }
    let parent = candidate.parent().context("Missing candidate directory")?;
    if parent.parent() != Some(directory.as_path())
        || parent.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        bail!("Candidate is outside the private update directory");
    }
    for path in [parent, candidate] {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() || path.canonicalize()? != path {
            bail!("Candidate must not redirect through a symlink");
        }
    }
    if std::fs::metadata(candidate)?.len() > MAX_BINARY_BYTES {
        bail!("Candidate exceeds size limit");
    }
    let report_path = parent.join("candidate.json");
    let metadata = std::fs::symlink_metadata(&report_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
        bail!("Invalid candidate report");
    }
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report_path)?)?;
    let latest = &report["latest"];
    let version = latest["version"].as_str().context("Missing candidate version")?;
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        bail!("Invalid candidate version");
    }
    let channel = latest["channel"].as_str().context("Missing candidate channel")?;
    let base = match channel {
        "stable" => format!("https://github.com/MetaCubeX/mihomo/releases/download/{version}"),
        "alpha" => "https://github.com/MetaCubeX/mihomo/releases/download/Prerelease-Alpha".to_owned(),
        _ => bail!("Invalid candidate channel"),
    };
    if latest["packageUrl"].as_str() != Some(format!("{base}/mihomo-darwin-arm64-go122-{version}.gz").as_str()) {
        bail!("Candidate source is not the official asset");
    }
    let bytes = std::fs::read(candidate)?;
    if bytes.len() < 32
        || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe]
        || bytes[4..8] != [0x0c, 0, 0, 1]
        || bytes[12..16] != [2, 0, 0, 0]
    {
        bail!("Candidate is not arm64 Mach-O");
    }
    if report["candidatePath"].as_str() != candidate.to_str()
        || report["binarySha256"].as_str() != Some(digest(&bytes).as_str())
    {
        bail!("Candidate metadata or digest does not match");
    }
    Ok(candidate.to_owned())
}
