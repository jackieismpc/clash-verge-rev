//! Closed, independently named native helper protocol. Root paths are derived, never accepted from clients.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub const HELPER_ID: &str = "io.github.jackieismpc.clash-native.helper";
pub const HELPER_SOCKET: &str = "/var/run/io.github.jackieismpc.clash-native.helper.sock";
pub const HELPER_DATA: &str = "/var/db/io.github.jackieismpc.clash-native.helper";
pub const MAX_HELPER_BYTES: usize = 4 * 1024 * 1024;
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HelperCommand {
    Status,
    Start {
        owner: String,
        generation: u64,
        mixed_port: u16,
        secret: String,
        runtime_yaml: String,
        #[serde(default)]
        provider_files: BTreeMap<String, String>,
    },
    Stop {
        owner: String,
        generation: u64,
    },
    Reload {
        owner: String,
        generation: u64,
        runtime_yaml: String,
        #[serde(default)]
        provider_files: BTreeMap<String, String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatus {
    pub available: bool,
    pub running: bool,
    pub owned_uid: Option<u32>,
    pub owner: Option<String>,
    pub generation: Option<u64>,
    pub pid: Option<u32>,
    pub private_socket: Option<String>,
    pub message: String,
}
#[derive(Serialize, Deserialize)]
pub struct HelperReply {
    pub status: Option<HelperStatus>,
    pub error: Option<String>,
}

/// Every command is blocked in integrationTest, including contact with a daily/system helper.
pub fn request(command: &HelperCommand, integration_test: bool) -> Result<HelperStatus> {
    if integration_test {
        bail!("integrationTest forbids native helper communication");
    }
    let body = serde_json::to_vec(command)?;
    if body.len() > MAX_HELPER_BYTES {
        bail!("helper request exceeds limit");
    }
    let mut stream = UnixStream::connect(HELPER_SOCKET).context("native helper is not registered or approved")?;
    verify_root_peer(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(&body)?;
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 64 * 1024 {
        bail!("invalid helper reply length");
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    let reply: HelperReply = serde_json::from_slice(&body)?;
    if let Some(error) = reply.error {
        bail!("{error}");
    }
    reply.status.ok_or_else(|| anyhow::anyhow!("missing helper status"))
}

pub fn verify_owned_stop(status: &HelperStatus, uid: u32, owner: &str, generation: u64) -> Result<()> {
    if !status.running
        || status.owned_uid != Some(uid)
        || status.owner.as_deref() != Some(owner)
        || status.generation != Some(generation)
    {
        bail!("TUN stop does not match owned UID/owner/generation");
    }
    Ok(())
}

fn trusted_helper_uid(uid: u32) -> bool {
    uid == 0
}
fn verify_root_peer(stream: &UnixStream) -> Result<()> {
    use std::os::fd::AsRawFd;
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if !trusted_helper_uid(uid) {
        bail!("native helper server is not root");
    }
    Ok(())
}

pub struct PreparedConfig {
    pub yaml: String,
    pub files: Vec<(PathBuf, String)>,
}
fn object(value: &Value) -> Result<&serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("expected configuration dictionary"))
}
fn check_no_paths(value: &Value) -> Result<()> {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "path"
                        | "certificate"
                        | "private-key"
                        | "ca"
                        | "ca-str"
                        | "tls-client-cert"
                        | "tls-client-key"
                        | "client-cert"
                        | "client-key"
                        | "script"
                        | "command"
                        | "plugin"
                        | "plugin-opts"
                        | "dialer-proxy"
                ) {
                    bail!("privileged proxy configuration forbids field {key}");
                }
                check_no_paths(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                check_no_paths(value)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn validate_proxy(proxy: &Value) -> Result<()> {
    let values = object(proxy)?;
    let allowed = [
        "name",
        "type",
        "server",
        "port",
        "password",
        "username",
        "uuid",
        "alterId",
        "cipher",
        "udp",
        "tls",
        "servername",
        "sni",
        "skip-cert-verify",
        "alpn",
        "network",
        "ws-opts",
        "grpc-opts",
        "h2-opts",
        "http-opts",
        "reality-opts",
        "fingerprint",
        "packet-encoding",
        "flow",
        "client-fingerprint",
        "udp-over-tcp",
        "up",
        "down",
        "obfs",
        "obfs-password",
        "auth",
        "auth-str",
        "fast-open",
        "disable-sni",
        "reduce-rtt",
        "request-timeout",
        "heartbeat-interval",
        "max-udp-relay-packet-size",
        "udp-relay-mode",
        "congestion-controller",
        "ip",
        "token",
        "tfo",
        "ip-version",
        "interface-name",
        "routing-mark",
        "mptcp",
        "smux",
    ];
    for key in values.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("unsupported privileged proxy field {key}");
        }
    }
    let kind = values.get("type").and_then(Value::as_str).unwrap_or_default();
    if ![
        "ss",
        "vmess",
        "vless",
        "trojan",
        "hysteria",
        "hysteria2",
        "tuic",
        "http",
        "socks5",
    ]
    .contains(&kind)
    {
        bail!("unsupported privileged proxy protocol {kind}");
    }
    let mut safe = proxy.clone();
    // These path fields are HTTP request targets, not filesystem paths.
    for key in ["ws-opts", "http-opts", "h2-opts"] {
        if let Some(options) = safe.get_mut(key).and_then(Value::as_object_mut) {
            options.remove("path");
        }
    }
    check_no_paths(&safe)
}

/// Rebuild a bounded runtime. Every provider and all core caches live under the root-owned UID directory.
/// File providers arrive as bounded text; original user paths are never opened by root.
pub fn prepare_config(
    runtime_yaml: &str,
    provider_files: &BTreeMap<String, String>,
    directory: &Path,
    mixed_port: u16,
    secret: &str,
) -> Result<PreparedConfig> {
    if mixed_port == 0 || secret.len() < 24 || secret.len() > 256 || runtime_yaml.len() > MAX_HELPER_BYTES {
        bail!("invalid helper runtime parameters");
    }
    let input: Value = serde_yaml_ng::from_str(runtime_yaml)?;
    let input = object(&input)?;
    let mut output = serde_json::Map::new();
    let proxies = input.get("proxies").cloned().unwrap_or_else(|| json!([]));
    for proxy in proxies
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("proxies must be an array"))?
    {
        validate_proxy(proxy)?;
    }
    output.insert("proxies".into(), proxies);
    let groups = input.get("proxy-groups").cloned().unwrap_or_else(|| json!([]));
    check_no_paths(&groups)?;
    output.insert("proxy-groups".into(), groups);
    let mut files = vec![];
    for category in ["proxy-providers", "rule-providers"] {
        let mut providers = serde_json::Map::new();
        if let Some(value) = input.get(category) {
            for (index, (name, provider)) in object(value)?.iter().enumerate() {
                let provider = object(provider)?;
                let kind = provider.get("type").and_then(Value::as_str).unwrap_or_default();
                if kind != "http" && kind != "file" {
                    bail!("unsupported provider type {kind}");
                }
                let path = directory.join(format!("{category}-{index}.yaml"));
                let mut normalized = serde_json::Map::new();
                for key in [
                    "type",
                    "url",
                    "interval",
                    "behavior",
                    "format",
                    "health-check",
                    "header",
                    "filter",
                    "exclude-filter",
                    "exclude-type",
                ] {
                    if let Some(value) = provider.get(key) {
                        check_no_paths(value)?;
                        normalized.insert(key.into(), value.clone());
                    }
                }
                // HTTP provider entries are parsed by mihomo. Do not accept overrides capable of adding file or executable settings.
                if let Some(overrides) = provider.get("override") {
                    check_no_paths(overrides)?;
                    normalized.insert("override".into(), overrides.clone());
                }
                if kind == "http" {
                    let url = normalized
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("HTTP provider requires URL"))?;
                    let url = reqwest::Url::parse(url)?;
                    if !matches!(url.scheme(), "https" | "http") {
                        bail!("provider URL must be HTTP(S)");
                    }
                } else {
                    let key = format!("{category}:{name}");
                    let content = provider_files
                        .get(&key)
                        .ok_or_else(|| anyhow::anyhow!("file provider {name} requires inline bounded content"))?;
                    if content.len() > 1024 * 1024 {
                        bail!("file provider exceeds limit");
                    }
                    if category == "proxy-providers" {
                        let parsed: Value = serde_yaml_ng::from_str(content)?;
                        for proxy in parsed
                            .get("proxies")
                            .and_then(Value::as_array)
                            .ok_or_else(|| anyhow::anyhow!("file proxy provider requires proxies"))?
                        {
                            validate_proxy(proxy)?;
                        }
                    }
                    files.push((path.clone(), content.clone()));
                    normalized.remove("url");
                }
                normalized.insert("path".into(), json!(path));
                providers.insert(name.clone(), Value::Object(normalized));
            }
        }
        output.insert(category.into(), Value::Object(providers));
    }
    for key in [
        "rules",
        "mode",
        "log-level",
        "ipv6",
        "find-process-mode",
        "global-client-fingerprint",
        "tcp-concurrent",
        "unified-delay",
    ] {
        if let Some(value) = input.get(key) {
            output.insert(key.into(), value.clone());
        }
    }
    output.insert("profile".into(), json!({"store-selected":false,"store-fake-ip":false}));
    output.insert("mixed-port".into(), json!(mixed_port));
    output.insert("bind-address".into(), json!("127.0.0.1"));
    output.insert("allow-lan".into(), json!(false));
    output.insert(
        "external-controller-unix".into(),
        json!(directory.join("privileged/engine.sock")),
    );
    output.insert("secret".into(), json!(secret));
    output.insert("tun".into(), json!({"enable":true,"stack":"mixed","device":"utun","auto-route":true,"auto-detect-interface":true,"dns-hijack":["any:53"]}));
    let mut dns = json!({"enable":true,"ipv6":false,"enhanced-mode":"fake-ip","fake-ip-range":"198.18.0.1/16","nameserver":["https://1.1.1.1/dns-query","https://8.8.8.8/dns-query"]});
    if let Some(source) = input.get("dns").and_then(Value::as_object) {
        for key in [
            "nameserver",
            "default-nameserver",
            "fallback",
            "proxy-server-nameserver",
            "direct-nameserver",
            "nameserver-policy",
            "fallback-filter",
            "fake-ip-filter",
            "fake-ip-filter-mode",
            "respect-rules",
            "use-hosts",
            "use-system-hosts",
        ] {
            if let Some(value) = source.get(key) {
                check_no_paths(value)?;
                dns.as_object_mut().unwrap().insert(key.into(), value.clone());
            }
        }
    }
    output.insert("dns".into(), dns);
    Ok(PreparedConfig {
        yaml: serde_yaml_ng::to_string(&Value::Object(output))?,
        files,
    })
}

#[path = "helper_controller.rs"]
mod helper_controller;

#[cfg(target_os = "macos")]
#[path = "helper_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::serve;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_peer_must_be_root() {
        assert!(trusted_helper_uid(0));
        assert!(!trusted_helper_uid(501));
        assert!(!trusted_helper_uid(u32::MAX));
    }
    #[test]
    fn integration_never_contacts_helper() {
        assert!(request(&HelperCommand::Status, true).is_err());
    }
    #[test]
    fn stop_requires_exact_session_ownership() {
        let status = HelperStatus {
            available: true,
            running: true,
            owned_uid: Some(501),
            owner: Some("native-mac:fixture".into()),
            generation: Some(8),
            pid: Some(42),
            private_socket: Some("/fixture/core.sock".into()),
            message: String::new(),
        };
        assert!(verify_owned_stop(&status, 501, "native-mac:fixture", 8).is_ok());
        assert!(verify_owned_stop(&status, 502, "native-mac:fixture", 8).is_err());
        assert!(verify_owned_stop(&status, 501, "native-mac:other", 8).is_err());
        assert!(verify_owned_stop(&status, 501, "native-mac:fixture", 7).is_err());
    }
    #[test]
    fn wire_rejects_arbitrary_commands_and_paths() {
        assert!(serde_json::from_str::<HelperCommand>(r#"{"operation":"start","owner":"native-mac:fixture","generation":1,"mixedPort":7890,"secret":"fixture","runtimeYaml":"{}","command":"/bin/sh"}"#).is_err());
        assert!(serde_json::from_str::<HelperCommand>(r#"{"operation":"execute","path":"/bin/sh"}"#).is_err());
    }
    #[test]
    fn root_provider_paths_are_rebuilt() {
        let mut files = BTreeMap::new();
        files.insert("rule-providers:local".into(), "payload:\n - example.com\n".into());
        let input = "proxy-providers:\n  http:\n    type: http\n    url: https://example.com/proxies.yaml\n    path: /etc/owned-by-user\nrule-providers:\n  local:\n    type: file\n    behavior: domain\n    path: /etc/passwd\nrules:\n - RULE-SET,local,DIRECT\n - MATCH,DIRECT\n";
        let config = prepare_config(
            input,
            &files,
            Path::new("/fixture/root-owned"),
            7890,
            "fixture-secret-32-characters-long",
        )
        .unwrap();
        assert!(!config.yaml.contains("/etc/"));
        assert!(config.yaml.contains("RULE-SET"));
        assert!(config.yaml.contains("https://example.com/proxies.yaml"));
        assert_eq!(config.files.len(), 1);
    }
    #[test]
    fn reject_root_file_and_executable_inputs() {
        for yaml in [
            "proxies: [{name: p, type: ss, plugin: bad}]",
            "proxies: [{name: p, type: vless, ws-opts: {tls-client-key: /secret}}]",
            "proxy-providers: {p: {type: http, url: file:///etc/passwd}}",
        ] {
            assert!(
                prepare_config(
                    yaml,
                    &BTreeMap::new(),
                    Path::new("/fixture/root-owned"),
                    7890,
                    "fixture-secret-32-characters-long"
                )
                .is_err()
            );
        }
    }
}
