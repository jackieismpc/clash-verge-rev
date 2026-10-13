use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
mod audit;
mod updates;

const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactLock {
    target: String,
    cores: BTreeMap<String, Artifact>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    version: String,
    url: String,
    sha256: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Verified {
    version: String,
    archive_sha256: String,
    binary_sha256: String,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate(lock: &ArtifactLock) -> Result<&Artifact> {
    if lock.target != "aarch64-apple-darwin" {
        bail!("asset lock must target Apple Silicon macOS");
    }
    let artifact = lock
        .cores
        .get("stable")
        .context("stable core is missing from artifact lock")?;
    if artifact.version.is_empty()
        || artifact.sha256.len() != 64
        || !artifact
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("pinned artifact must declare a version and lowercase SHA256");
    }
    let url = reqwest::Url::parse(&artifact.url)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.path().starts_with("/MetaCubeX/mihomo/releases/download/")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("pinned core must use the official HTTPS release URL");
    }
    Ok(artifact)
}

fn decompress(archive: &[u8]) -> Result<Vec<u8>> {
    let mut binary = Vec::new();
    GzDecoder::new(archive)
        .take(MAX_BINARY_BYTES + 1)
        .read_to_end(&mut binary)
        .context("invalid core gzip archive")?;
    if binary.len() as u64 > MAX_BINARY_BYTES {
        bail!("core binary exceeds 128 MiB limit");
    }
    // arm64 Mach-O: 64-bit little-endian magic and ARM64 CPU type.
    if binary.len() < 8 || binary[..4] != [0xcf, 0xfa, 0xed, 0xfe] || binary[4..8] != [0x0c, 0x00, 0x00, 0x01] {
        bail!("core archive does not contain an arm64 Mach-O executable");
    }
    Ok(binary)
}

async fn publish(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;
    let temporary = path.with_extension("partial");
    let outcome = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(bytes).await?;
        if executable {
            file.set_permissions(std::fs::Permissions::from_mode(0o755)).await?;
        }
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    outcome
}

async fn prepare(root: &Path, offline: bool) -> Result<()> {
    let lock: ArtifactLock =
        serde_json::from_slice(&tokio::fs::read(root.join("resources/core-artifacts.json")).await?)?;
    let artifact = validate(&lock)?;
    let directory = root.join(".work/engines");
    tokio::fs::create_dir_all(&directory).await?;
    // Never race another downloader's staging files or replace its output.
    let owner = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("assets.lock"))?;
    owner
        .try_lock()
        .context("another native asset preparation owns this directory")?;
    for name in [
        "mihomo.partial",
        "verified.partial",
        &format!("{}.partial", artifact.sha256),
    ] {
        match tokio::fs::remove_file(directory.join(name)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let archive_path = directory.join(format!("{}.gz", artifact.sha256));
    let cached = match tokio::fs::metadata(&archive_path).await {
        Ok(metadata) if metadata.len() <= MAX_ARCHIVE_BYTES as u64 => {
            let bytes = tokio::fs::read(&archive_path).await?;
            (sha256(&bytes) == artifact.sha256).then_some(bytes)
        }
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let needs_publication = cached.is_none();
    let archive = if let Some(bytes) = cached {
        bytes
    } else {
        if offline {
            bail!("verified pinned core archive is unavailable offline");
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()?;
        let mut response = client.get(&artifact.url).send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_ARCHIVE_BYTES {
                bail!("core archive exceeds 64 MiB limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        if sha256(&bytes) != artifact.sha256 {
            bail!("pinned core archive SHA256 mismatch");
        }
        bytes
    };
    let binary = decompress(&archive)?;
    if needs_publication {
        publish(&archive_path, &archive, false).await?;
    }
    let binary_path = directory.join("mihomo");
    let verified = Verified {
        version: artifact.version.clone(),
        archive_sha256: artifact.sha256.clone(),
        binary_sha256: sha256(&binary),
    };
    publish(&binary_path, &binary, true).await?;
    publish(
        &directory.join("verified.json"),
        &serde_json::to_vec_pretty(&verified)?,
        false,
    )
    .await?;
    println!("Prepared verified {} at {}", artifact.version, binary_path.display());
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        bail!("native assets require Apple Silicon macOS");
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("missing repository root")?
        .to_path_buf();
    match arguments.as_slice() {
        [] => prepare(&root, false).await,
        [flag] if flag == "--offline" => prepare(&root, true).await,
        [flag] if flag == "--audit-source-tree" => audit::check(&root),
        [flag] if flag == "--check-core-update" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&updates::check_latest(false).await?)?
            );
            Ok(())
        }
        [flag, alpha] if flag == "--check-core-update" && alpha == "--alpha" => {
            println!("{}", serde_json::to_string_pretty(&updates::check_latest(true).await?)?);
            Ok(())
        }
        [flag] if flag == "--stage-core-update" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&updates::stage_latest(&root, false).await?)?
            );
            Ok(())
        }
        [flag, alpha] if flag == "--stage-core-update" && alpha == "--alpha" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&updates::stage_latest(&root, true).await?)?
            );
            Ok(())
        }
        _ => bail!(
            "usage: native-core-assets [--offline | --audit-source-tree | --check-core-update [--alpha] | --stage-core-update [--alpha]]"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn pinned_identity_and_archive_content_are_verified() -> Result<()> {
        let artifact = Artifact {
            version: "v1".into(),
            url: "https://github.com/MetaCubeX/mihomo/releases/download/v1/core.gz".into(),
            sha256: "0".repeat(64),
        };
        let mut lock = ArtifactLock {
            target: "aarch64-apple-darwin".into(),
            cores: BTreeMap::from([("stable".into(), artifact)]),
        };
        assert!(validate(&lock).is_ok());
        lock.cores.get_mut("stable").context("missing artifact")?.url = "http://example.com/core.gz".into();
        assert!(validate(&lock).is_err());
        let mut archive = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        archive.write_all(b"not a native executable")?;
        assert!(decompress(&archive.finish()?).is_err());
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_cache_cannot_replace_an_existing_core() -> Result<()> {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .context("missing root")?
            .to_path_buf();
        let root = repository.join(".work/assets-tests").join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let directory = root.join(".work/engines");
        tokio::fs::create_dir_all(root.join("resources")).await?;
        tokio::fs::create_dir_all(&directory).await?;
        let digest = "0".repeat(64);
        let lock = serde_json::json!({"target":"aarch64-apple-darwin","cores":{"stable":{"version":"fixture","url":"https://github.com/MetaCubeX/mihomo/releases/download/fixture/core.gz","sha256":digest}}});
        tokio::fs::write(root.join("resources/core-artifacts.json"), serde_json::to_vec(&lock)?).await?;
        tokio::fs::write(directory.join(format!("{digest}.gz")), b"corrupted archive").await?;
        tokio::fs::write(directory.join("mihomo"), b"previous verified core").await?;
        tokio::fs::write(directory.join("verified.json"), b"previous metadata").await?;
        let outcome = prepare(&root, true).await;
        assert!(outcome.is_err());
        assert_eq!(
            tokio::fs::read(directory.join("mihomo")).await?,
            b"previous verified core"
        );
        assert_eq!(
            tokio::fs::read(directory.join("verified.json")).await?,
            b"previous metadata"
        );
        tokio::fs::remove_dir_all(root).await?;
        Ok(())
    }
}
