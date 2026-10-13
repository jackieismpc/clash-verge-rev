use crate::config::{ExecutionMode, Options};
use anyhow::{Context, Result, bail};
use serde_yaml_ng::{Mapping, Value};
use std::path::{Component, Path};

pub fn validate(options: &Options) -> Result<()> {
    if options.execution_mode != ExecutionMode::IntegrationTest {
        return Ok(());
    }
    let root = options
        .integration_root
        .as_ref()
        .context("integrationRoot is required in test mode")?;
    if !root.is_absolute()
        || root.file_name().is_none_or(|name| name != "integration")
        || root
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| name != ".work")
        || root.canonicalize().context("integrationRoot must already exist")? != *root
    {
        bail!("integrationRoot must be a real .work/integration directory");
    }
    let relative = options
        .data_dir
        .strip_prefix(root)
        .context("test dataDir must be inside integrationRoot")?;
    if relative.as_os_str().is_empty() || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        bail!("test dataDir must identify a private run below integrationRoot");
    }
    reject_links(&options.data_dir, root)?;
    Ok(())
}

fn reject_links(path: &Path, root: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for part in path.strip_prefix(root)?.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => bail!("test paths cannot contain symbolic links"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                bail!("test directories cannot contain symbolic links");
            }
            if entry.file_type()?.is_dir() {
                reject_links(&entry.path(), root)?;
            }
        }
    }
    Ok(())
}

pub fn normalize(root: &mut Mapping, options: &Options) -> Result<()> {
    for key in [
        "listeners",
        "tunnels",
        "external-ui",
        "external-ui-url",
        "external-ui-name",
        "external-controller-cors",
    ] {
        root.remove(key);
    }
    root.insert(
        "tun".into(),
        serde_yaml_ng::from_str("enable: false\nauto-route: false\nstrict-route: false\n")?,
    );
    root.insert(
        "ntp".into(),
        serde_yaml_ng::from_str("enable: false\nwrite-to-system: false\n")?,
    );
    for key in ["rule-providers", "proxy-providers"] {
        let Some(providers) = root.get_mut(key).and_then(Value::as_mapping_mut) else {
            continue;
        };
        for (index, value) in providers.values_mut().enumerate() {
            normalize_provider(value, options, key, index)?;
        }
    }
    Ok(())
}

fn normalize_provider(value: &mut Value, options: &Options, category: &str, index: usize) -> Result<()> {
    let provider = value.as_mapping_mut().context("provider must be a mapping")?;
    if let Some(path) = provider.get("path") {
        let path = Path::new(path.as_str().context("provider path must be a string")?);
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
        {
            bail!("test provider paths must be relative without traversal");
        }
        if provider.get("type").and_then(Value::as_str) == Some("file") {
            let owned = options.data_dir.join(path);
            let root = options.integration_root.as_ref().context("missing integrationRoot")?;
            reject_links(&owned, root)?;
            if owned
                .canonicalize()
                .context("local test provider must exist")?
                .starts_with(&options.data_dir)
            {
                return Ok(());
            }
            bail!("local test provider is outside the owned run");
        }
    }
    if provider.get("type").and_then(Value::as_str) == Some("file") {
        bail!("local test provider requires an owned path");
    }
    let format = provider.get("format").and_then(Value::as_str).unwrap_or("yaml");
    let extension = if format == "mrs" {
        "mrs"
    } else if format == "text" {
        "txt"
    } else {
        "yaml"
    };
    provider.insert(
        "path".into(),
        format!("providers/{category}/p{index}.{extension}").into(),
    );
    Ok(())
}
