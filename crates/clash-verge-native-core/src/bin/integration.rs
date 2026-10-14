use anyhow::{Context, Result, bail};
use clash_verge_native_core::{
    Client, Request,
    config::{ExecutionMode, MAX_DOCUMENT_BYTES, Options},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    net::{Ipv4Addr, TcpListener, UdpSocket},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command};

const GROUP: &str = "Integration-Test-Only";
const DEFAULT_URL: &str = "https://www.gstatic.com/generate_204";

struct Arguments {
    profile: PathBuf,
    mihomo: PathBuf,
    all: bool,
    url: String,
}
#[derive(Clone)]
struct Node {
    id: String,
    protocol: String,
    value: Value,
}
struct Bridge<'a> {
    client: &'a Client,
    directory: &'a Path,
    next: u64,
    progress: usize,
    owned_pid: Option<u64>,
}

fn arguments(args: impl IntoIterator<Item = String>) -> Result<Option<Arguments>> {
    let mut args = args.into_iter();
    let (mut profile, mut mihomo, mut all, mut url) = (None, None, false, DEFAULT_URL.to_string());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--profile" => profile = Some(PathBuf::from(args.next().context("missing profile path")?)),
            "--mihomo" => mihomo = Some(PathBuf::from(args.next().context("missing mihomo path")?)),
            "--all-nodes" => all = true,
            "--url" => url = args.next().context("missing HTTPS URL")?,
            _ => bail!("unknown argument"),
        }
    }
    if profile.is_none() && mihomo.is_none() {
        return Ok(None);
    }
    let profile = profile.context("--profile is required")?;
    let mihomo = mihomo.context("--mihomo is required")?;
    if !profile.is_absolute() || !mihomo.is_absolute() {
        bail!("source and binary paths must be absolute");
    }
    let parsed = reqwest::Url::parse(&url).context("invalid URL")?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        bail!("URL must use HTTPS without embedded credentials");
    }
    Ok(Some(Arguments {
        profile,
        mihomo,
        all,
        url,
    }))
}

fn select_nodes(document: &str, all: bool) -> Result<Vec<Node>> {
    let root: serde_yaml_ng::Value = serde_yaml_ng::from_str(document).context("invalid source YAML")?;
    let proxies = root
        .get("proxies")
        .and_then(serde_yaml_ng::Value::as_sequence)
        .context("source needs explicit proxies; remote providers are not fetched by this runner")?;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut nodes = Vec::new();
    for proxy in proxies {
        let mut value = serde_json::to_value(proxy)?;
        let kind = value["type"].as_str().unwrap_or("").to_ascii_lowercase();
        if !["vless", "ss", "hysteria2", "tuic"].contains(&kind.as_str()) {
            continue;
        }
        let count = counts.entry(kind.clone()).or_default();
        if !all && *count >= 2 {
            continue;
        }
        // A nested dialer can route through another node and cannot prove this target's chain.
        if value.get("dialer-proxy").is_some() {
            continue;
        }
        *count += 1;
        let id = format!("node-{:03}", nodes.len() + 1);
        value["name"] = id.clone().into();
        nodes.push(Node {
            id,
            protocol: kind,
            value,
        });
        if nodes.len() > 4096 {
            bail!("source exceeds runner node limit");
        }
    }
    if nodes.is_empty() {
        bail!("source has no supported standalone nodes");
    }
    Ok(nodes)
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    Ok(())
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn create_run() -> Result<(PathBuf, PathBuf)> {
    let workspace = std::env::current_dir()?.canonicalize()?;
    let root = workspace.join(".work/integration");
    private_directory(&root)?;
    let root = root.canonicalize()?;
    for _ in 0..20 {
        let mut bytes = [0_u8; 4];
        fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let directory = root.join(format!("i-{:08x}", u32::from_ne_bytes(bytes)));
        match fs::create_dir(&directory) {
            Ok(()) => {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
                return Ok((root, directory));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("unable to reserve private run directory")
}
fn reserve_port() -> Result<(u16, TcpListener, UdpSocket)> {
    for _ in 0..100 {
        let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = tcp.local_addr()?.port();
        if let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)) {
            return Ok((port, tcp, udp));
        }
    }
    bail!("unable to reserve loopback TCP and UDP port")
}

impl Bridge<'_> {
    fn submit(&mut self, command: &str, args: Value, generation: Option<u64>) -> Result<String> {
        self.next += 1;
        let id = format!("integration-{}", self.next);
        let status = self.client.submit(Request {
            version: 1,
            id: id.clone(),
            command: command.into(),
            args,
            core_generation: generation,
            revision: None,
        });
        if status != 0 {
            bail!("native request queue rejected command");
        }
        Ok(id)
    }
    fn event(&mut self) -> Result<Option<Value>> {
        let event = self.client.next_event();
        if let Some(event) = &event {
            use std::io::Write;
            let mut log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(self.directory.join("events.private.jsonl"))?;
            writeln!(log, "{}", serde_json::to_string(event)?)?;
            if event["data"]["stream"] == "delay" {
                self.progress += 1;
            }
        }
        Ok(event)
    }
    async fn wait(&mut self, id: &str) -> Result<Value> {
        let start = Instant::now();
        loop {
            while let Some(event) = self.event()? {
                if event["id"] == id && event["kind"] == "result" {
                    return Ok(event);
                }
            }
            if start.elapsed() > Duration::from_secs(45) {
                self.client.cancel(id);
                bail!("native command timed out");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    async fn call(&mut self, command: &str, args: Value) -> Result<Value> {
        let id = self.submit(command, args, None)?;
        let event = self.wait(&id).await?;
        if event["ok"] != true {
            bail!("native command failed (details in private event log)");
        }
        Ok(event["data"].clone())
    }
    async fn connections(&mut self) -> Result<Value> {
        self.call("core.request", json!({"method":"GET", "path":"/connections"}))
            .await
    }
}

fn chain_matches(connection: &Value, target: &str, nodes: &[Node]) -> bool {
    let Some(chains) = connection["chains"].as_array() else {
        return false;
    };
    chains.iter().any(|chain| chain == target)
        && chains.iter().any(|chain| chain == GROUP)
        && !chains.iter().any(|chain| {
            chain == "DIRECT" || chain == "REJECT" || nodes.iter().any(|node| node.id != target && chain == &node.id)
        })
}

async fn https_probe(bridge: &mut Bridge<'_>, node: &Node, nodes: &[Node], port: u16, url: &str) -> Result<Value> {
    bridge
        .call(
            "core.request",
            json!({"method":"PUT", "path":format!("/proxies/{GROUP}"), "body":{"name":node.id}}),
        )
        .await?;
    bridge
        .call("core.request", json!({"method":"DELETE", "path":"/connections"}))
        .await?;
    let mut command = Command::new("/usr/bin/curl");
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--proxy",
        &format!("http://127.0.0.1:{port}"),
        "--noproxy",
        "",
        "--connect-timeout",
        "10",
        "--max-time",
        "20",
        "--output",
        "/dev/null",
        "--write-out",
        "%{json}",
        url,
    ]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_ascii_lowercase().ends_with("_proxy") {
            command.env_remove(key);
        }
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().context("start system HTTPS client")?;
    let mut stdout = child.stdout.take().context("missing curl stdout")?;
    let mut stderr = child.stderr.take().context("missing curl stderr")?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let mut chain_seen = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Ok(connections) = bridge.connections().await
            && let Some(items) = connections["connections"].as_array()
        {
            chain_seen |= items
                .iter()
                .any(|connection| chain_matches(connection, &node.id, nodes));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let output = stdout_task.await??;
    let errors = stderr_task.await??;
    private_write(
        &bridge.directory.join(format!("{}.curl.private.stderr", node.id)),
        &errors,
    )?;
    let timings: Value = serde_json::from_slice(&output).unwrap_or(json!({}));
    private_write(
        &bridge.directory.join(format!("{}.curl.private.json", node.id)),
        &output,
    )?;
    let tls_seconds = timings["time_appconnect"].as_f64().unwrap_or(0.0);
    let status_code = timings["http_code"]
        .as_u64()
        .or_else(|| timings["response_code"].as_u64())
        .unwrap_or(0);
    let tls_verified = status.success() && timings["ssl_verify_result"].as_u64() == Some(0) && tls_seconds > 0.0;
    let success = status.success() && (200..400).contains(&status_code) && tls_verified && chain_seen;
    Ok(
        json!({"node":node.id,"protocol":node.protocol,"tlsVerified":tls_verified,"tlsMilliseconds":tls_seconds*1000.0,
        "firstByteMilliseconds":timings["time_starttransfer"].as_f64().map(|v|v*1000.0),"httpStatus":status_code,"actualTargetChain":chain_seen,"httpsSuccess":success}),
    )
}

async fn snapshot_network(directory: &Path, label: &str) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut values = BTreeMap::new();
    for (name, binary, args) in [
        ("proxy", "/usr/sbin/scutil", vec!["--proxy"]),
        ("dns", "/usr/sbin/scutil", vec!["--dns"]),
        ("routes", "/usr/sbin/netstat", vec!["-rn"]),
        ("defaultRoute", "/sbin/route", vec!["-n", "get", "default"]),
        ("dailyInstances", "/bin/ps", vec!["-axo", "pid=,ppid=,comm="]),
    ] {
        let output = Command::new(binary).args(args).output().await?;
        let mut bytes = output.stdout;
        if name == "dailyInstances" {
            bytes = String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower.contains("mihomo") || lower.contains("clash")
                })
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes();
        }
        private_write(&directory.join(format!("{label}-{name}.private.txt")), &bytes)?;
        private_write(
            &directory.join(format!("{label}-{name}.private.stderr")),
            &output.stderr,
        )?;
        values.insert(name.to_string(), bytes);
        values.insert(
            format!("{name}Readable"),
            output.status.success().to_string().into_bytes(),
        );
    }
    Ok(values)
}

async fn workflow(bridge: &mut Bridge<'_>, nodes: &[Node], port: u16, url: &str) -> Result<Value> {
    let yaml = serde_yaml_ng::to_string(
        &json!({"proxies":nodes.iter().map(|n|n.value.clone()).collect::<Vec<_>>(),
        "proxy-groups":[{"name":GROUP,"type":"select","proxies":nodes.iter().map(|n|n.id.clone()).collect::<Vec<_>>()}],
        "rules":[format!("MATCH,{GROUP}")], "mode":"rule"}),
    )?;
    private_write(&bridge.directory.join("source-selected.private.yaml"), yaml.as_bytes())?;
    bridge
        .call(
            "profile.import",
            json!({"name":"Private integration fixture", "yaml":yaml}),
        )
        .await?;
    bridge.call("runtime.start", json!({})).await?;
    bridge.owned_pid = bridge.client.snapshot()["data"]["runtime"]["pid"].as_u64();
    let single = bridge
        .call("proxy.delay", json!({"names":[nodes[0].id],"url":url}))
        .await?;
    let mut delays = BTreeMap::new();
    for chunk in nodes.chunks(4) {
        let results = bridge
            .call(
                "proxy.delay",
                json!({"names":chunk.iter().map(|n|n.id.clone()).collect::<Vec<_>>(),"url":url}),
            )
            .await?;
        for result in results.as_array().context("invalid delay result")? {
            if let Some(name) = result["name"].as_str() {
                delays.insert(name.to_string(), result["delay"].clone());
            }
        }
    }
    let id = bridge.submit("proxy.delay", json!({"names":[nodes[0].id],"url":url}), None)?;
    let cancel_accepted = bridge.client.cancel(&id);
    let cancelled = bridge.wait(&id).await?;
    let cancellation_confirmed = cancelled["ok"] == false && cancelled["error"]["code"] == "cancelled";
    let mut results = Vec::new();
    for node in nodes {
        let mut result = https_probe(bridge, node, nodes, port, url).await?;
        result["delayMilliseconds"] = delays.get(&node.id).cloned().unwrap_or(Value::Null);
        results.push(result);
        println!(
            "{} {}: delay={} ms TLS={} chain={} HTTPS={}",
            node.id,
            node.protocol,
            delays.get(&node.id).unwrap_or(&Value::Null),
            results.last().unwrap()["tlsVerified"],
            results.last().unwrap()["actualTargetChain"],
            results.last().unwrap()["httpsSuccess"]
        );
    }
    let generation = bridge.client.snapshot()["data"]["runtime"]["coreGeneration"]
        .as_u64()
        .context("missing generation")?;
    bridge.call("runtime.stop", json!({})).await?;
    let id = bridge.submit(
        "proxy.delay",
        json!({"names":[nodes[0].id],"url":url}),
        Some(generation),
    )?;
    let stale = bridge.wait(&id).await?;
    Ok(
        json!({"nodes":results,"singleDelayMilliseconds":single[0]["delay"],"delayProgressEvents":bridge.progress,"delayProgressComplete":bridge.progress == nodes.len() + 1,"cancelAccepted":cancel_accepted,"cancellationConfirmed":cancellation_confirmed,
        "staleAfterStopRejected":stale["ok"] == false && stale["error"]["code"] == "staleGeneration"}),
    )
}

fn permissions_private(path: &Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(false);
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Ok(false);
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            if !permissions_private(&entry?.path())? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn pid_gone(pid: Option<u64>) -> bool {
    pid.is_none_or(|pid| unsafe { libc::kill(pid as libc::pid_t, 0) } == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
}

fn execute(args: Arguments) -> Result<bool> {
    let metadata = fs::metadata(&args.profile).context("source is unavailable")?;
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        bail!("source must be a regular YAML file at most 16 MiB");
    }
    let document = fs::read_to_string(&args.profile).context("unable to read source")?;
    let nodes = select_nodes(&document, args.all)?;
    let (root, directory) = create_run()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let before = runtime.block_on(snapshot_network(&directory, "before"))?;
    let (port, tcp, udp) = reserve_port()?;
    let client = Client::open(Options {
        version: 1,
        data_dir: directory.clone(),
        mihomo_path: Some(args.mihomo),
        host_path: Some({
            let host = std::env::current_exe()?.with_file_name("clash-native-helper");
            if !host.is_file() {
                bail!("build the native supervisor with scripts/native-integration.sh");
            }
            host
        }),
        mixed_port: Some(port),
        execution_mode: ExecutionMode::IntegrationTest,
        integration_root: Some(root),
    })?;
    drop((tcp, udp));
    let mut bridge = Bridge {
        client: &client,
        directory: &directory,
        next: 0,
        progress: 0,
        owned_pid: None,
    };
    let mut report = runtime.block_on(async {
        tokio::select! {
            value = workflow(&mut bridge, &nodes, port, &args.url) => match value { Ok(value) => value, Err(error) => {
                let _ = private_write(&directory.join("failure.private.txt"), format!("{error:#}").as_bytes()); json!({"workflowFailed":true})
            } },
            signal = tokio::signal::ctrl_c() => { let _ = signal; json!({"interrupted":true}) }
        }
    });
    let pid = bridge
        .owned_pid
        .or_else(|| client.snapshot()["data"]["runtime"]["pid"].as_u64());
    // Stop cancels every pending native request. Drop joins the owned worker outside Tokio.
    client.stop();
    drop(client);
    let after = runtime.block_on(snapshot_network(&directory, "after"))?;
    let changes = before
        .iter()
        .map(|(key, value)| (key.clone(), json!({"unchanged":after.get(key)==Some(value)})))
        .collect::<serde_json::Map<_, _>>();
    report["networkSnapshots"] = changes.into();
    report["sourceUnchanged"] = fs::read_to_string(&args.profile)
        .is_ok_and(|current| current == document)
        .into();
    report["ownedCleanup"] = json!({"pidGone":pid_gone(pid),"socketGone":!directory.join("c.sock").exists(),
        "tcpReleased":TcpListener::bind((Ipv4Addr::LOCALHOST,port)).is_ok(),"udpReleased":UdpSocket::bind((Ipv4Addr::LOCALHOST,port)).is_ok()});
    let tracked = std::process::Command::new("/usr/bin/git")
        .args(["ls-files", ".work"])
        .output()?;
    let ignored = std::process::Command::new("/usr/bin/git")
        .arg("check-ignore")
        .arg(&directory)
        .output()?;
    report["artifacts"] = json!({"privatePermissions":permissions_private(&directory)?, "workIgnored":ignored.status.success(), "noTrackedWork":tracked.status.success()&&tracked.stdout.is_empty()});
    private_write(&directory.join("summary.json"), &serde_json::to_vec_pretty(&report)?)?;
    let success = report["nodes"]
        .as_array()
        .is_some_and(|items| !items.is_empty() && items.iter().all(|v| v["httpsSuccess"] == true))
        && report["sourceUnchanged"] == true
        && report["delayProgressComplete"] == true
        && report["cancellationConfirmed"] == true
        && report["staleAfterStopRejected"] == true
        && report["ownedCleanup"]
            .as_object()
            .is_some_and(|values| values.values().all(|v| v == true))
        && report["artifacts"]
            .as_object()
            .is_some_and(|values| values.values().all(|v| v == true));
    println!(
        "Anonymous report: {} (passed={success}); snapshot changes are recorded only, never restored",
        directory.join("summary.json").display()
    );
    Ok(success)
}

fn main() {
    let result = arguments(std::env::args().skip(1)).and_then(|args| match args {
        None => { println!("Usage: integration --profile /absolute/source.yaml --mihomo /absolute/mihomo [--all-nodes] [--url https://...]\nNo source supplied: no core started and no network requests made."); Ok(true) },
        Some(args) => execute(args),
    });
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(_) => {
            eprintln!("Integration runner failed; sensitive details are withheld from console.");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_has_no_side_effects_and_requires_explicit_paths() {
        assert!(arguments(Vec::<String>::new()).unwrap().is_none());
        assert!(arguments(["--profile".into(), "relative.yaml".into()]).is_err());
    }
    #[test]
    fn selection_is_bounded_anonymous_and_preserves_protocol_fields() {
        let source = json!({"proxies":(0..5).flat_map(|i| ["vless","ss","hysteria2","tuic"].map(move |kind|json!({"name":format!("sample-{i}"),"type":kind,"server":"example.invalid","port":443,"password":"synthetic"}))).collect::<Vec<_>>()});
        let yaml = serde_yaml_ng::to_string(&source).unwrap();
        let selected = select_nodes(&yaml, false).unwrap();
        assert_eq!(selected.len(), 8);
        assert_eq!(select_nodes(&yaml, true).unwrap().len(), 20);
        assert_eq!(selected[0].value["server"], "example.invalid");
        assert_eq!(selected[0].value["name"], "node-001");
    }
    #[test]
    fn actual_target_chain_rejects_direct_or_another_node() {
        let nodes = vec![
            Node {
                id: "node-001".into(),
                protocol: "ss".into(),
                value: json!({}),
            },
            Node {
                id: "node-002".into(),
                protocol: "ss".into(),
                value: json!({}),
            },
        ];
        assert!(chain_matches(&json!({"chains":["node-001",GROUP]}), "node-001", &nodes));
        assert!(!chain_matches(
            &json!({"chains":["node-001",GROUP,"DIRECT"]}),
            "node-001",
            &nodes
        ));
        assert!(!chain_matches(
            &json!({"chains":["node-001",GROUP,"node-002"]}),
            "node-001",
            &nodes
        ));
        assert!(!chain_matches(&json!({"chains":["node-001"]}), "node-001", &nodes));
    }
}
