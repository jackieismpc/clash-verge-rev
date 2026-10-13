//! Read-only, bounded legacy import. Original bytes are preserved before conversion.
use crate::config::{Profile, parse_profile_yaml};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_FILE: usize = 16 * 1024 * 1024;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportReport {
    pub profiles: Vec<Profile>,
    pub selected_profile_id: Option<String>,
    pub warnings: Vec<String>,
    pub fallback_path: PathBuf,
}
#[derive(Deserialize)]
struct Index {
    current: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    uid: Option<String>,
    name: Option<String>,
    file: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
    updated: Option<u64>,
    extra: Option<BTreeMap<String, u64>>,
    home: Option<String>,
    option: Option<serde_json::Value>,
    selected: Option<Vec<Selection>>,
}
#[derive(Deserialize)]
struct Selection {
    name: Option<String>,
    now: Option<String>,
}
fn safe_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains('\\')
        || name.contains(':')
        || name.contains('\0')
        || Path::new(name).components().any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Unsafe legacy archive path");
    }
    Ok(())
}
fn read_file(path: &Path) -> Result<Vec<u8>> {
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > MAX_TOTAL as u64 {
        bail!("Legacy source must be a bounded regular file");
    }
    let mut out = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_TOTAL as u64 + 1)
        .read_to_end(&mut out)?;
    if out.len() > MAX_TOTAL {
        bail!("Legacy source exceeds size limit");
    }
    Ok(out)
}
fn zip_files(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    if archive.len() > 1024 {
        bail!("Too many ZIP entries");
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name()?.into_owned();
        safe_name(name.trim_end_matches('/'))?;
        if entry.enclosed_name().is_none() || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("ZIP path escape or symlink is forbidden");
        }
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_FILE as u64 || entry.compressed_size() > MAX_TOTAL as u64 {
            bail!("ZIP entry exceeds size limit");
        }
        let mut decoded = Vec::new();
        entry.by_ref().take(MAX_FILE as u64 + 1).read_to_end(&mut decoded)?;
        if decoded.len() > MAX_FILE || decoded.len() as u64 != entry.size() {
            bail!("Invalid ZIP expansion");
        }
        total = total.checked_add(decoded.len()).context("ZIP overflow")?;
        if total > MAX_TOTAL {
            bail!("ZIP expansion exceeds total limit");
        }
        if files.insert(name, decoded).is_some() {
            bail!("Duplicate ZIP path");
        }
    }
    Ok(files)
}
pub fn inspect_legacy(source: &Path, fallback_root: &Path) -> Result<LegacyImportReport> {
    if std::fs::symlink_metadata(source)?.file_type().is_symlink() {
        bail!("Legacy source must not be a symlink");
    }
    let source = source.canonicalize().context("Legacy source is unavailable")?;
    let original_metadata = std::fs::symlink_metadata(source.as_path())?;
    if !fallback_root.is_dir() || std::fs::symlink_metadata(fallback_root)?.file_type().is_symlink() {
        bail!("Fallback root must be a private real directory");
    }
    let fallback_path = fallback_root.join(format!(
        "legacy-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        std::process::id()
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&fallback_path)?;
    let mut files = BTreeMap::new();
    if original_metadata.is_dir() {
        files.insert("profiles.yaml".into(), read_file(&source.join("profiles.yaml"))?);
        let profiles = source.join("profiles");
        if std::fs::symlink_metadata(&profiles)?.file_type().is_symlink() {
            bail!("Legacy profiles directory must not be a symlink");
        }
        for entry in std::fs::read_dir(profiles)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid legacy filename"))?;
            safe_name(&name)?;
            if files.len() > 1024 {
                bail!("Too many legacy files");
            }
            let bytes = read_file(&entry.path())?;
            if bytes.len() > MAX_FILE || files.values().map(Vec::len).sum::<usize>() + bytes.len() > MAX_TOTAL {
                bail!("Legacy directory exceeds size limit");
            }
            files.insert(format!("profiles/{name}"), bytes);
        }
    } else {
        let bytes = read_file(&source)?;
        let mut original = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(fallback_path.join("original.zip"))?;
        original.write_all(&bytes)?;
        original.sync_all()?;
        files = zip_files(&bytes)?;
        if files.contains_key("original.zip") {
            bail!("Reserved legacy archive filename");
        }
    }
    if files.values().map(Vec::len).sum::<usize>() > MAX_TOTAL {
        bail!("Legacy import exceeds total size limit");
    }
    for (name, bytes) in &files {
        let path = fallback_path.join(name);
        if let Some(parent) = path.parent()
            && parent != fallback_path
            && !parent.exists()
        {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::File::open(&fallback_path)?.sync_all()?;
    let index: Index =
        serde_yaml_ng::from_slice(files.get("profiles.yaml").context("Legacy profiles.yaml is missing")?)?;
    let mut report = LegacyImportReport {
        profiles: vec![],
        selected_profile_id: None,
        warnings: vec![],
        fallback_path,
    };
    for (position, item) in index.items.into_iter().enumerate() {
        let kind = item.kind.as_deref().unwrap_or("local");
        if kind == "script" {
            report.warnings.push(
                "Custom JavaScript was preserved without execution. Migrate its changes to YAML overrides.".into(),
            );
            continue;
        }
        if !["local", "remote"].contains(&kind) {
            report.warnings.push(format!(
                "Legacy {kind} overlay was preserved; review and migrate it to YAML overrides."
            ));
            continue;
        }
        let file = item.file.context("Legacy profile filename is missing")?;
        safe_name(&file)?;
        let yaml = String::from_utf8(
            files
                .get(&format!("profiles/{file}"))
                .context("Legacy profile file is missing")?
                .clone(),
        )?;
        parse_profile_yaml(&yaml)?;
        let id = item.uid.unwrap_or_else(|| format!("legacy-{position}"));
        let selected_nodes = item
            .selected
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| Some((s.name?, s.now?)))
            .collect();
        let interval = item
            .option
            .as_ref()
            .and_then(|o| o.get("update_interval"))
            .and_then(serde_json::Value::as_u64);
        let interval = if kind == "remote" {
            Some(
                if item
                    .option
                    .as_ref()
                    .and_then(|o| o.get("allow_auto_update"))
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
                {
                    0
                } else {
                    interval.unwrap_or(0)
                },
            )
        } else {
            None
        };
        report.profiles.push(Profile {
            id: id.clone(),
            name: item.name.unwrap_or_else(|| id.clone()),
            kind: kind.into(),
            yaml,
            url: item.url,
            updated_at: item.updated.unwrap_or(0),
            subscription_info: item.extra,
            update_interval_minutes: interval,
            home: item.home,
            selected_nodes,
        });
        if index.current.as_ref() == Some(&id) {
            report.selected_profile_id = Some(id);
        }
    }
    if report.profiles.is_empty() {
        bail!(
            "No supported legacy profiles; originals were preserved at {}",
            report.fallback_path.display()
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<PathBuf> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.work/migration-tests");
        std::fs::create_dir_all(&root)?;
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(path)
    }
    #[test]
    fn legacy_directory_preserves_scripts_and_metadata() -> Result<()> {
        let root = fixture()?;
        let source = root.join("source");
        let fallback = root.join("fallback");
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&fallback)?;
        std::fs::create_dir(source.join("profiles"))?;
        std::fs::write(
            source.join("profiles.yaml"),
            "current: p\nitems:\n- {uid: p, name: Sample, type: remote, file: p.yaml, url: 'https://example.invalid/sub', option: {update_interval: 60}, selected: [{name: Group, now: Sample}]}\n- {uid: script, type: script, file: script.js}\n- {uid: disabled, type: remote, file: p.yaml, url: 'https://example.invalid/sub'}\n- {uid: optedout, type: remote, file: p.yaml, url: 'https://example.invalid/sub', option: {update_interval: 60, allow_auto_update: false}}\n",
        )?;
        std::fs::write(source.join("profiles/p.yaml"), "proxies: []\nrules: [MATCH,DIRECT]\n")?;
        let script = b"function main(config) { return config; }";
        std::fs::write(source.join("profiles/script.js"), script)?;
        let report = inspect_legacy(&source, &fallback)?;
        assert_eq!(report.profiles.len(), 3);
        assert_eq!(report.selected_profile_id.as_deref(), Some("p"));
        assert_eq!(report.profiles[0].update_interval_minutes, Some(60));
        assert_eq!(report.profiles[1].update_interval_minutes, Some(0));
        assert_eq!(report.profiles[2].update_interval_minutes, Some(0));
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(std::fs::read(source.join("profiles/script.js"))?, script);
        assert_eq!(std::fs::read(report.fallback_path.join("profiles/script.js"))?, script);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn zip_traversal_and_symlinks_are_rejected() -> Result<()> {
        for name in ["../escape", "/absolute", "profiles/../escape"] {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(b"sample")?;
            let bytes = writer.finish()?.into_inner();
            assert!(zip_files(&bytes).is_err());
        }
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.add_symlink("profiles/link", "../escape", zip::write::SimpleFileOptions::default())?;
        assert!(zip_files(&writer.finish()?.into_inner()).is_err());
        Ok(())
    }
    #[test]
    fn unsafe_archive_paths_and_source_symlinks_are_rejected() -> Result<()> {
        for name in [
            "../escape",
            "/absolute",
            "C:/absolute",
            "profiles/../escape",
            "profiles\\escape",
        ] {
            assert!(safe_name(name).is_err());
        }
        assert!(zip_files(b"broken ZIP").is_err());
        let root = fixture()?;
        let source = root.join("source");
        std::fs::create_dir(&source)?;
        let link = root.join("link");
        std::os::unix::fs::symlink(&source, &link)?;
        assert!(inspect_legacy(&link, &root).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
