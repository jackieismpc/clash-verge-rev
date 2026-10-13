use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use serde_yaml_ng::{Mapping, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub version: u32,
    pub data_dir: PathBuf,
    pub mihomo_path: Option<PathBuf>,
    pub mixed_port: Option<u16>,
}

impl Options {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported contract version");
        }
        if !self.data_dir.is_absolute() {
            bail!("dataDir must be absolute");
        }
        if self.mihomo_path.as_ref().is_some_and(|p| !p.is_absolute()) {
            bail!("mihomoPath must be absolute");
        }
        if self.mixed_port == Some(0) {
            bail!("mixedPort must be nonzero");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub yaml: String,
    pub url: Option<String>,
    pub updated_at: u64,
    pub subscription_info: Option<BTreeMap<String, u64>>,
    pub update_interval_minutes: Option<u64>,
    pub home: Option<String>,
    #[serde(default)]
    pub selected_nodes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub mixed_port: u16,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub version: u32,
    pub revision: u64,
    pub profiles: Vec<Profile>,
    pub selected_profile_id: Option<String>,
    pub overrides: String,
    pub settings: Settings,
}

impl Store {
    pub fn new(mixed_port: u16) -> Self {
        Self {
            version: 1,
            revision: 0,
            profiles: vec![],
            selected_profile_id: None,
            overrides: "{}\n".into(),
            settings: Settings {
                mixed_port,
                mode: "rule".into(),
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported store version");
        }
        if self.settings.mixed_port == 0 {
            bail!("mixedPort must be nonzero");
        }
        if !["rule", "global", "direct"].contains(&self.settings.mode.as_str()) {
            bail!("invalid mode");
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.profiles {
            if !ids.insert(&p.id) || p.id.is_empty() {
                bail!("duplicate or empty profile ID");
            }
            parse_yaml(&p.yaml)?;
            if !["local", "remote"].contains(&p.kind.as_str()) {
                bail!("invalid profile kind");
            }
        }
        if self.selected_profile_id.as_ref().is_some_and(|id| !ids.contains(id)) {
            bail!("selected profile is missing");
        }
        parse_yaml(&self.overrides)?;
        Ok(())
    }
    pub fn profile(&self, id: &str) -> Result<&Profile> {
        self.profiles.iter().find(|p| p.id == id).context("profile not found")
    }
    pub fn profile_mut(&mut self, id: &str) -> Result<&mut Profile> {
        self.profiles
            .iter_mut()
            .find(|p| p.id == id)
            .context("profile not found")
    }
    pub fn runtime_yaml(&self, socket: &Path, secret: &str) -> Result<String> {
        let mut yaml = match &self.selected_profile_id {
            Some(id) => parse_yaml(&self.profile(id)?.yaml)?,
            None => parse_yaml("proxies: []\nproxy-groups: []\nrules: [\"MATCH,DIRECT\"]\n")?,
        };
        merge(&mut yaml, parse_yaml(&self.overrides)?);
        let root = yaml.as_mapping_mut().context("profile root must be a mapping")?;
        // Engine protocol fields are intentionally left intact. Only endpoints and
        // privileged effects owned by this client are reconciled here.
        root.insert("mixed-port".into(), self.settings.mixed_port.into());
        for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
            root.insert(key.into(), 0.into());
        }
        root.insert("allow-lan".into(), false.into());
        root.insert("bind-address".into(), "127.0.0.1".into());
        root.insert("mode".into(), self.settings.mode.clone().into());
        root.remove("external-controller");
        root.remove("external-controller-tls");
        root.remove("external-controller-pipe");
        root.insert(
            "external-controller-unix".into(),
            socket.to_string_lossy().to_string().into(),
        );
        root.insert("secret".into(), secret.into());
        let tun = root
            .entry(Value::from("tun"))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        if !tun.is_mapping() {
            *tun = Value::Mapping(Mapping::new());
        }
        tun.as_mapping_mut()
            .context("invalid tun mapping")?
            .insert("enable".into(), false.into());
        if let Some(dns) = root.get_mut("dns").and_then(Value::as_mapping_mut) {
            // The unprivileged client cannot own a DNS listener or OS DNS settings.
            dns.remove("listen");
        }
        serde_yaml_ng::to_string(&yaml).context("serialize runtime YAML")
    }
    pub fn snapshot(&self, runtime: Json) -> Json {
        let mut data = serde_json::to_value(self).unwrap_or(Json::Null);
        data["runtime"] = runtime;
        data["capabilities"] = serde_json::json!({
            "profiles":true,"subscriptions":true,"localOverrides":true,"proxySelection":true,
            "unixController":true,"webSocketStreams":true,"mediaUnlock":true,"localBackup":true,
            "systemProxy":false,"tun":false,"helper":false,"scriptProfiles":false,
            "webdavBackup":false,"autoUpdate":false
        });
        data
    }
}

pub fn parse_yaml(text: &str) -> Result<Value> {
    if text.len() > MAX_DOCUMENT_BYTES {
        bail!("YAML exceeds 16 MiB limit");
    }
    let yaml: Value = serde_yaml_ng::from_str(text.trim_start_matches('\u{feff}')).context("invalid YAML")?;
    if !yaml.is_mapping() {
        bail!("YAML root must be a mapping");
    }
    Ok(yaml)
}

fn merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Mapping(target), Value::Mapping(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, source) => *target = source,
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug)]
pub struct CommittedWriteError(pub std::io::Error);
impl std::fmt::Display for CommittedWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file replaced but directory sync failed: {}", self.0)
    }
}
impl std::error::Error for CommittedWriteError {}

pub async fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension("pending");
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .await
        .with_context(|| format!("create staging file {}", temp.display()))?;
    let outcome = async {
        file.write_all(data).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp, path).await?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(CommittedWriteError)?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_preserves_protocol_fields_and_merges_without_privilege() {
        let mut store = Store::new(19881);
        store.profiles.push(Profile { id:"p".into(),name:"p".into(),kind:"local".into(),
            yaml:"proxies:\n  - {name: node, type: vless, reality-opts: {public-key: keep}, unknown-future-field: 42}\ndns: {enable: true, nameserver: [https://dns.example/dns-query], listen: 0.0.0.0:53}\ntun: {enable: true, stack: mixed}\n".into(),
            url:None,updated_at:0,subscription_info:None,update_interval_minutes:None,home:None,selected_nodes:BTreeMap::new() });
        store.selected_profile_id = Some("p".into());
        store.overrides = "dns: {ipv6: false}\n".into();
        let yaml = parse_yaml(&store.runtime_yaml(Path::new("/tmp/owned.sock"), "secret").unwrap()).unwrap();
        assert_eq!(yaml["proxies"][0]["unknown-future-field"].as_u64(), Some(42));
        assert_eq!(yaml["proxies"][0]["reality-opts"]["public-key"].as_str(), Some("keep"));
        assert_eq!(yaml["tun"]["enable"].as_bool(), Some(false));
        assert!(yaml["dns"].as_mapping().unwrap().get("listen").is_none());
        assert_eq!(
            yaml["dns"]["nameserver"][0].as_str(),
            Some("https://dns.example/dns-query")
        );
    }
}
