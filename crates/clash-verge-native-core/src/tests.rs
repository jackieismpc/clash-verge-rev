use super::*;
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
static FFI_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".work")
            .join("core-tests")
            .join(unique_id());
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn options(&self) -> Options {
        Options {
            version: 1,
            data_dir: self.0.clone(),
            mihomo_path: None,
            host_path: None,
            mixed_port: Some(19882),
            execution_mode: ExecutionMode::Interactive,
            integration_root: None,
        }
    }
    fn integration() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".work/integration");
        let path = root.join(unique_id());
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn integration_options(&self) -> Options {
        let mut options = self.options();
        options.execution_mode = ExecutionMode::IntegrationTest;
        options.integration_root = Some(self.0.parent().unwrap().to_path_buf());
        options
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(id: &str, command: &str, args: Value) -> Request {
    Request {
        version: 1,
        id: id.into(),
        command: command.into(),
        args,
        core_generation: None,
        revision: None,
    }
}

#[test]
fn uri_import_uses_the_transaction_and_rejects_ambiguous_sources() {
    let dir = TestDir::new();
    let client = Client::open(dir.options()).unwrap();
    let uri = "vless://00000000-0000-0000-0000-000000000000@[2001:db8::1]:443?security=reality&pbk=sample&sid=00&type=ws&path=%2Fproxy#Sample";
    assert_eq!(client.submit(request("uri", "profile.import", json!({"uri": uri}))), 0);
    assert_eq!(result(&client, "uri")["ok"], true);
    let snapshot = client.snapshot();
    let yaml = parse_yaml(snapshot["data"]["profiles"][0]["yaml"].as_str().unwrap()).unwrap();
    assert_eq!(yaml["proxies"][0]["server"].as_str(), Some("2001:db8::1"));
    assert_eq!(
        yaml["proxies"][0]["reality-opts"]["public-key"].as_str(),
        Some("sample")
    );
    assert_eq!(yaml["proxy-groups"][0]["proxies"][0].as_str(), Some("Sample"));
    assert_eq!(yaml["rules"][0].as_str(), Some("MATCH,Imported"));
    assert_eq!(
        client.submit(request(
            "ambiguous",
            "profile.import",
            json!({"uri": uri, "yaml": "{}"})
        )),
        0
    );
    assert_eq!(result(&client, "ambiguous")["ok"], false);
    assert_eq!(client.snapshot()["data"]["revision"], snapshot["data"]["revision"]);
    drop(client);
    let reopened = Client::open(dir.options()).unwrap();
    assert_eq!(reopened.snapshot()["data"]["profiles"], snapshot["data"]["profiles"]);
}

#[test]
fn script_import_is_explicit_and_preserves_the_source_file() {
    let dir = TestDir::new();
    let source = dir.0.join("source.js");
    let code = "function main(config) { throw new Error('never execute'); }";
    std::fs::write(&source, code).unwrap();
    let client = Client::open(dir.options()).unwrap();
    for (id, args) in [
        ("code", json!({"yaml": code})),
        ("file", json!({"yaml": "{}", "filename": "source.JS"})),
        ("descriptor", json!({"yaml": "type: script\nfile: source.js\n"})),
        ("kind", json!({"yaml": "{}", "sourceKind": "script"})),
    ] {
        assert_eq!(client.submit(request(id, "profile.import", args)), 0);
        let event = result(&client, id);
        assert_eq!(event["ok"], false);
        assert!(
            event["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Keep the original file")
        );
    }
    assert_eq!(std::fs::read_to_string(source).unwrap(), code);
    assert!(client.snapshot()["data"]["profiles"].as_array().unwrap().is_empty());
    assert!(config::parse_profile_yaml("description: 'function main is just a description'\n").is_ok());
}

#[test]
fn integration_mode_rejects_foreign_directories_and_links() {
    let dir = TestDir::integration();
    let mut options = dir.integration_options();
    options.validate().unwrap();
    options.integration_root = None;
    assert!(options.validate().is_err());
    options = dir.integration_options();
    let outside = TestDir::new();
    options.data_dir = outside.0.clone();
    assert!(options.validate().is_err());
    options = dir.integration_options();
    std::os::unix::fs::symlink(&outside.0, dir.0.join("providers")).unwrap();
    assert!(options.validate().is_err());
}

#[test]
fn test_profile_cannot_reenable_system_effects_or_escape_provider_cache() {
    let dir = TestDir::integration();
    let client = Client::open(dir.integration_options()).unwrap();
    for command in [
        "systemProxy.set",
        "systemDns.set",
        "tun.set",
        "helper.install",
        "login.set",
    ] {
        assert_eq!(client.submit(request(command, command, json!({}))), 0);
        let event = result(&client, command);
        assert_eq!(event["ok"], false);
        assert!(
            event["error"]["message"]
                .as_str()
                .unwrap_or_else(|| panic!("unexpected fixture envelope: {event}"))
                .contains("integrationTest forbids")
        );
    }
    assert_eq!(client.submit(request("bad", "profile.import", json!({"yaml":"rule-providers: {bad: {type: http, path: ../foreign.yaml, url: https://example.invalid/rules}}\n"}))), 0);
    assert_eq!(result(&client, "bad")["ok"], false);
    assert!(client.snapshot()["data"]["profiles"].as_array().unwrap().is_empty());
    import(&client);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(dir.0.join("state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn isolated_runtime_preserves_protocol_fields_and_owns_all_endpoints() {
    let dir = TestDir::integration();
    let mut store = Store::new(19992);
    store.profiles.push(Profile {
        id: "p".into(), name: "fixture".into(), kind: "local".into(), url: None, updated_at: 0,
        subscription_info: None, update_interval_minutes: None, home: None, selected_nodes: BTreeMap::new(),
        yaml: "proxies: [{name: node, type: vless, unknown-future-field: keep}]\ntun: {enable: true, auto-route: true}\nntp: {enable: true, write-to-system: true}\ndns: {enable: true, listen: 0.0.0.0:53}\nlisteners: [{type: tun}]\nexternal-ui: /foreign/ui\nrule-providers: {remote: {type: http, path: cache/shared.yaml, url: https://example.invalid/rules}}\n".into(),
    });
    store.selected_profile_id = Some("p".into());
    store.overrides =
        "allow-lan: true\nmixed-port: 7897\nexternal-controller: 0.0.0.0:9090\ntun: {enable: true}\n".into();
    let text = store
        .owned_runtime_yaml(&dir.0.join("c.sock"), "owned", &dir.integration_options())
        .unwrap();
    let yaml = parse_yaml(&text).unwrap();
    assert_eq!(yaml["mixed-port"].as_u64(), Some(19992));
    assert_eq!(yaml["allow-lan"].as_bool(), Some(false));
    assert_eq!(yaml["tun"]["enable"].as_bool(), Some(false));
    assert_eq!(yaml["ntp"]["write-to-system"].as_bool(), Some(false));
    assert!(yaml["dns"].as_mapping().unwrap().get("listen").is_none());
    assert!(yaml.as_mapping().unwrap().get("external-ui").is_none());
    assert!(yaml.as_mapping().unwrap().get("listeners").is_none());
    assert_eq!(yaml["proxies"][0]["unknown-future-field"].as_str(), Some("keep"));
    assert_eq!(
        yaml["rule-providers"]["remote"]["path"].as_str(),
        Some("providers/rule-providers/p0.yaml")
    );
}

#[test]
fn occupied_udp_port_is_rejected_without_starting_or_stopping_a_process() {
    let dir = TestDir::integration();
    let tcp = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = tcp.local_addr().unwrap().port();
    let udp = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    drop(tcp);
    let mut options = dir.integration_options();
    options.mixed_port = Some(port);
    options.mihomo_path = Some(PathBuf::from("/usr/bin/false"));
    let client = Client::open(options).unwrap();
    assert_eq!(client.submit(request("start", "runtime.start", json!({}))), 0);
    let event = result(&client, "start");
    assert_eq!(event["ok"], false);
    assert!(
        event["error"]["message"]
            .as_str()
            .unwrap()
            .contains("UDP is already occupied")
    );
    assert_eq!(client.snapshot()["data"]["runtime"]["status"], "stopped");
    assert_eq!(udp.local_addr().unwrap().port(), port);
}
fn result(client: &Client, id: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = client.next_event()
            && event["kind"] == "result"
            && event["id"] == id
        {
            return event;
        }
        assert!(std::time::Instant::now() < deadline, "missing result {id}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn import(client: &Client) -> String {
    assert_eq!(
        client.submit(request(
            "import",
            "profile.import",
            json!({"name":"fixture","yaml":"proxies: []\nrules: [\"MATCH,DIRECT\"]\n"})
        )),
        0
    );
    let event = result(client, "import");
    assert_eq!(event["ok"], true, "{event}");
    event["data"]["id"].as_str().unwrap().into()
}

#[test]
fn profiles_persist_and_failed_transaction_preserves_committed_snapshot() {
    let dir = TestDir::new();
    let client = Client::open(dir.options()).unwrap();
    let id = import(&client);
    let before = client.snapshot();
    std::fs::create_dir(dir.0.join("state.pending")).unwrap();
    assert_eq!(
        client.submit(request(
            "rename",
            "profile.rename",
            json!({"id":id,"name":"must not commit"})
        )),
        0
    );
    assert_eq!(result(&client, "rename")["ok"], false);
    assert_eq!(client.snapshot(), before);
    std::fs::remove_dir(dir.0.join("state.pending")).unwrap();
    drop(client);
    let reopened = Client::open(dir.options()).unwrap();
    assert_eq!(reopened.snapshot()["data"]["profiles"][0]["name"], "fixture");
    assert_eq!(reopened.snapshot()["data"]["revision"], 1);
}

#[test]
fn second_owner_and_stale_generation_are_rejected() {
    let dir = TestDir::new();
    let client = Client::open(dir.options()).unwrap();
    assert!(Client::open(dir.options()).is_err());
    let mut old = request("old", "settings.patch", json!({"mode":"direct"}));
    old.core_generation = Some(7);
    assert_eq!(client.submit(old), 0);
    let event = result(&client, "old");
    assert_eq!(event["error"]["code"], "staleGeneration");
    assert_eq!(client.snapshot()["data"]["settings"]["mode"], "rule");
}

#[test]
fn cancelling_subscription_io_does_not_commit_or_block_following_command() {
    let dir = TestDir::new();
    let client = Client::open(dir.options()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (connected_tx, connected_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let fixture = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n")
            .unwrap();
        connected_tx.send(()).unwrap();
        let _ = finish_rx.recv_timeout(Duration::from_secs(5));
    });
    assert_eq!(
        client.submit(request(
            "fetch",
            "profile.import",
            json!({"url":format!("http://{address}/subscription")})
        )),
        0
    );
    connected_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(client.cancel("fetch"));
    let event = result(&client, "fetch");
    assert_eq!(event["error"]["code"], "cancelled");
    assert!(client.snapshot()["data"]["profiles"].as_array().unwrap().is_empty());
    assert_eq!(client.submit(request("after", "snapshot", json!({}))), 0);
    assert_eq!(result(&client, "after")["ok"], true);
    finish_tx.send(()).unwrap();
    fixture.join().unwrap();
}

#[test]
fn old_stream_generation_never_enters_new_snapshot_queue() {
    let shared = Shared {
        snapshot: Mutex::new(json!({"revision":4,"runtime":{"status":"running","coreGeneration":2}})),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    };
    shared.stream(1, "traffic", Ok(json!({"up":20,"down":40})));
    assert!(lock(&shared.events).is_empty());
    shared.stream(2, "traffic", Ok(json!({"up":30,"down":50})));
    let event = lock(&shared.events).pop_front().unwrap();
    assert_eq!(event.value["coreGeneration"], 2);
    assert_eq!(event.value["revision"], 4);
    assert_eq!(event.value["data"]["value"]["up"], 30);
}

#[tokio::test]
async fn unix_http_reload_rolls_back_after_persistence_failure() {
    let dir = TestDir::new();
    let socket = dir.0.join("c.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let fixture = tokio::spawn(async move {
        let mut bodies = vec![];
        for _ in 0..2 {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            loop {
                let mut buffer = [0_u8; 4096];
                let count = connection.read(&mut buffer).await.unwrap();
                bytes.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|number| number.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        bodies.push(serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length]).unwrap());
                        break;
                    }
                }
                assert_ne!(count, 0);
            }
            connection
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        }
        bodies
    });
    let store = Store::new(19883);
    let mut runtime_state = stopped("owner", 1, &socket, 19883);
    runtime_state["status"] = "running".into();
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(store.snapshot(runtime_state.clone())),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    });
    let mut worker = Worker {
        network: network_client::NetworkClient::new(&dir.0, ExecutionMode::Interactive).unwrap(),
        options: dir.options(),
        store,
        runtime_state,
        controller: Controller::new(socket, "secret".into()).unwrap(),
        secret: "secret".into(),
        shared,
        scheduler: Scheduler::default(),
        subscription: None,
    };
    std::fs::create_dir(dir.0.join("state.pending")).unwrap();
    let mut candidate = worker.store.clone();
    candidate.settings.mode = "direct".into();
    assert!(
        worker
            .commit(candidate, "commit", &Cancellation::default())
            .await
            .is_err()
    );
    assert_eq!(worker.store.settings.mode, "rule");
    assert_eq!(worker.store.revision, 0);
    assert!(worker.is_running());
    let bodies = fixture.await.unwrap();
    assert!(bodies[0]["path"].as_str().unwrap().ends_with("runtime-staged.yaml"));
    assert!(bodies[1]["path"].as_str().unwrap().ends_with("runtime-rollback.yaml"));
    let rollback = std::fs::read_to_string(dir.0.join("runtime-rollback.yaml")).unwrap();
    assert_eq!(parse_yaml(&rollback).unwrap()["mode"].as_str(), Some("rule"));
}

#[test]
fn ffi_allocations_null_inputs_shutdown_and_directory_release() {
    let _serial = lock(&FFI_TEST_LOCK);
    let dir = TestDir::new();
    let config = json!({"version":1,"dataDir":dir.0,"mixedPort":19884}).to_string();
    unsafe {
        assert!(ffi::cvn_create(std::ptr::null(), 1).is_null());
        let error = ffi::cvn_last_error();
        assert_eq!(error.status, 0);
        ffi::cvn_free_buffer(error);
        let handle = ffi::cvn_create(config.as_ptr(), config.len() as u64);
        assert!(!handle.is_null());
        for _ in 0..100 {
            let data = ffi::cvn_snapshot(handle);
            assert_eq!(data.status, 0);
            let value: Value =
                serde_json::from_slice(std::slice::from_raw_parts(data.data, data.length as usize)).unwrap();
            assert_eq!(value["version"], 1);
            assert_eq!(ffi::cvn_free_buffer(data), 0);
            assert_eq!(ffi::cvn_free_buffer(data), 0);
        }
        assert_eq!(ffi::cvn_submit(handle, std::ptr::null(), 1), 1);
        assert_eq!(ffi::cvn_stop(handle), 0);
        let request = json!({"version":1,"id":"late","command":"snapshot","args":{}}).to_string();
        assert_eq!(ffi::cvn_submit(handle, request.as_ptr(), request.len() as u64), 3);
        while ffi::cvn_destroy(handle) == 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(ffi::cvn_snapshot(handle).status, 1);
        assert_eq!(ffi::cvn_destroy(handle), 0);
        let reopened = ffi::cvn_create(config.as_ptr(), config.len() as u64);
        assert!(!reopened.is_null());
        while ffi::cvn_destroy(reopened) == 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[tokio::test]
async fn delay_batch_runs_concurrently_and_publishes_each_result() {
    let dir = TestDir::new();
    let socket = dir.0.join("c.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(6));
    let fixture = tokio::spawn(async move {
        let mut handlers = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let (mut connection, _) = listener.accept().await.unwrap();
            let barrier = Arc::clone(&barrier);
            handlers.spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0_u8; 4096];
                    let count = connection.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") { break; }
                }
                let header = String::from_utf8(bytes).unwrap();
                assert!(header.contains("%20node/delay?timeout=5000&url=http%3A%2F%2Ffixture%2Fpath%3Fx%3D1"));
                barrier.wait().await;
                connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\nConnection: close\r\n\r\n{\"delay\":42}").await.unwrap();
            });
        }
        while let Some(handler) = handlers.join_next().await {
            handler.unwrap();
        }
    });
    let store = Store::new(19885);
    let mut runtime_state = stopped("owner", 1, &socket, 19885);
    runtime_state["status"] = "running".into();
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(store.snapshot(runtime_state.clone())),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    });
    let mut worker = Worker {
        network: network_client::NetworkClient::new(&dir.0, ExecutionMode::Interactive).unwrap(),
        options: dir.options(),
        store,
        runtime_state,
        controller: Controller::new(socket, "secret".into()).unwrap(),
        secret: "secret".into(),
        shared: Arc::clone(&shared),
        scheduler: Scheduler::default(),
        subscription: None,
    };
    let names: Vec<String> = (0..6).map(|index| format!("{index} node")).collect();
    let request = request(
        "delay",
        "proxy.delay",
        json!({"names":names,"url":"http://fixture/path?x=1"}),
    );
    let results = tokio::time::timeout(
        Duration::from_secs(3),
        worker.execute(&request, &Cancellation::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 6);
    assert!(results.as_array().unwrap().iter().all(|value| value["delay"] == 42));
    assert!(
        results
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value["requestId"] == "delay")
    );
    assert_eq!(lock(&shared.events).len(), 6);
    assert!(
        lock(&shared.events)
            .iter()
            .all(|event| event.value["data"]["stream"] == "delay" && event.value["timestamp"].is_number())
    );
    fixture.await.unwrap();
}

#[tokio::test]
async fn slow_delay_does_not_block_snapshot_or_owned_stop() {
    let dir = TestDir::new();
    let socket = dir.0.join("c.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (connected, ready) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0_u8; 4096];
        let _ = stream.read(&mut header).await.unwrap();
        connected.send(()).unwrap();
        let mut remainder = Vec::new();
        stream.read_to_end(&mut remainder).await.unwrap();
    });
    let store = Store::new(19886);
    let mut runtime_state = stopped("owner", 1, &socket, 19886);
    runtime_state["status"] = "running".into();
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(store.snapshot(runtime_state.clone())),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    });
    let mut worker = Worker {
        network: network_client::NetworkClient::new(&dir.0, ExecutionMode::Interactive).unwrap(),
        options: dir.options(),
        store,
        runtime_state,
        controller: Controller::new(socket, "secret".into()).unwrap(),
        secret: "secret".into(),
        shared: Arc::clone(&shared),
        scheduler: Scheduler::default(),
        subscription: None,
    };
    for id in ["delay", "read", "stop"] {
        lock(&shared.pending).insert(
            id.into(),
            Pending {
                token: Arc::new(Cancellation::default()),
                bytes: 100,
                response_bytes: 1024,
            },
        );
    }
    let (sender, receiver) = mpsc::channel(8);
    let owner = tokio::spawn(async move { worker.serve(receiver).await });
    sender
        .send(Queued {
            request: request("delay", "proxy.delay", json!({"names":["slow"]})),
            cancellation: Arc::new(Cancellation::default()),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    sender
        .send(Queued {
            request: request("read", "snapshot", json!({})),
            cancellation: Arc::new(Cancellation::default()),
        })
        .await
        .unwrap();
    sender
        .send(Queued {
            request: request("stop", "runtime.stop", json!({})),
            cancellation: Arc::new(Cancellation::default()),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while lock(&shared.events)
            .iter()
            .filter(|event| event.value["kind"] == "result")
            .count()
            < 3
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    {
        let events = lock(&shared.events);
        assert!(
            events.iter().any(|event| event.value["id"] == "read"
                && event.value["kind"] == "result"
                && event.value["ok"] == true)
        );
        assert!(
            events.iter().any(|event| event.value["id"] == "stop"
                && event.value["kind"] == "result"
                && event.value["ok"] == true)
        );
        assert!(
            events
                .iter()
                .any(|event| event.value["id"] == "delay" && event.value["error"]["code"] == "cancelled")
        );
    }
    shared.shutdown.notify_one();
    owner.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), fixture)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn scheduled_subscription_preserves_local_rules_and_can_be_disabled() {
    use std::io::Read;
    let dir = TestDir::new();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let url = format!("http://{}/subscription", listener.local_addr().unwrap());
    let mut store = Store::new(19882);
    store.profiles.push(Profile {
        id: "remote".into(),
        name: "fixture".into(),
        kind: "remote".into(),
        url: Some(url),
        updated_at: 1,
        subscription_info: None,
        update_interval_minutes: Some(1),
        home: None,
        selected_nodes: BTreeMap::new(),
        yaml: "proxies: []\nrules: [MATCH,DIRECT]\n".into(),
    });
    store.selected_profile_id = Some("remote".into());
    std::fs::write(dir.0.join("state.json"), serde_json::to_vec(&store).unwrap()).unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        let yaml = "proxies: []\nrules: [\"DOMAIN,remote.example,DIRECT\",\"MATCH,DIRECT\"]\n";
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nProfile-Update-Interval: 999999999\r\nConnection: close\r\n\r\n{yaml}", yaml.len()).unwrap();
    });
    let client = Client::open(dir.options()).unwrap();
    assert_eq!(
        client.submit(request(
            "rules",
            "rules.overrides.set",
            json!({"prepend":["DOMAIN,local.example,REJECT"],"append":[],"delete":[]})
        )),
        0
    );
    assert_eq!(result(&client, "rules")["ok"], true);
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while client.snapshot()["data"]["profiles"][0]["updatedAt"] == 1 {
        assert!(std::time::Instant::now() < deadline, "scheduled update not committed");
        std::thread::sleep(Duration::from_millis(20));
    }
    server.join().unwrap();
    let current = client.snapshot()["data"].clone();
    assert!(
        current["profiles"][0]["yaml"]
            .as_str()
            .unwrap()
            .contains("remote.example")
    );
    assert_eq!(current["profiles"][0]["updateIntervalMinutes"], 1);
    assert_eq!(current["ruleOverrides"]["prepend"][0], "DOMAIN,local.example,REJECT");
    assert_eq!(
        client.submit(request(
            "disable",
            "subscription.configure",
            json!({"id":"remote","intervalMinutes":0})
        )),
        0
    );
    assert_eq!(result(&client, "disable")["ok"], true);
    drop(client);
    let client = Client::open(dir.options()).unwrap();
    assert_eq!(client.snapshot()["data"]["profiles"][0]["updateIntervalMinutes"], 0);
    assert_eq!(client.snapshot()["data"]["ruleOverrides"], current["ruleOverrides"]);
}

#[test]
fn legacy_zip_import_and_corrupt_backup_leave_original_data_intact() {
    let dir = TestDir::new();
    let zip_path = dir.0.join("legacy.zip");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in [
        (
            "profiles.yaml",
            "current: old\nitems:\n  - {uid: old, name: Original, file: old.yaml, type: local}\n  - {uid: script, file: custom.js, type: script}\n",
        ),
        ("profiles/old.yaml", "proxies: []\nrules: [\"MATCH,DIRECT\"]\n"),
        ("profiles/custom.js", "throw new Error('must never execute');"),
    ] {
        archive.start_file(name, options).unwrap();
        archive.write_all(bytes.as_bytes()).unwrap();
    }
    let original = archive.finish().unwrap().into_inner();
    std::fs::write(&zip_path, &original).unwrap();
    let client = Client::open(dir.options()).unwrap();
    assert_eq!(
        client.submit(request("legacy", "legacy.import", json!({"path":zip_path}))),
        0
    );
    let event = result(&client, "legacy");
    assert_eq!(event["ok"], true, "{event}");
    let before = client.snapshot();
    assert_eq!(before["data"]["selectedProfileId"], "old");
    assert_eq!(before["data"]["profiles"][0]["name"], "Original");
    assert_eq!(before["data"]["migrationWarnings"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read(&zip_path).unwrap(), original);
    assert_eq!(
        client.submit(request("duplicate", "legacy.import", json!({"path":zip_path}))),
        0
    );
    assert_eq!(result(&client, "duplicate")["ok"], false);
    assert_eq!(client.snapshot()["data"]["revision"], before["data"]["revision"]);
    std::fs::create_dir_all(dir.0.join("backups")).unwrap();
    std::fs::write(dir.0.join("backups/broken.json"), "{invalid").unwrap();
    assert_eq!(
        client.submit(request("broken", "backup.restore", json!({"id":"broken.json"}))),
        0
    );
    assert_eq!(result(&client, "broken")["ok"], false);
    assert_eq!(client.snapshot()["data"], before["data"]);
    drop(client);
    let client = Client::open(dir.options()).unwrap();
    assert_eq!(client.snapshot()["data"]["profiles"], before["data"]["profiles"]);
}

#[test]
fn isolated_commands_forbid_activation_and_global_recovery() {
    let dir = TestDir::integration();
    let client = Client::open(dir.integration_options()).unwrap();
    assert_eq!(client.snapshot()["data"]["capabilities"]["systemProxy"], false);
    for (i, command) in [
        "core.update.check",
        "core.update.stage",
        "core.update.activate",
        "core.update.reset",
        "network.restore",
        "helper.status",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("forbidden-{i}");
        assert_eq!(client.submit(request(&id, command, json!({}))), 0);
        let event = result(&client, &id);
        assert_eq!(event["ok"], false);
        assert!(
            event["error"]["message"]
                .as_str()
                .unwrap()
                .contains("integrationTest forbids")
        );
    }
}

#[tokio::test]
async fn core_activation_rolls_back_failed_persistence_and_restores_bundled_baseline() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let dir = TestDir::new();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let binary = workspace.join(".work/engines/mihomo");
    assert!(
        binary.is_file(),
        "prepare pinned assets with scripts/native-core-assets.sh --offline"
    );
    std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory = dir.0.join(".work/core-updates/fixture");
    std::fs::create_dir_all(&directory).unwrap();
    let candidate = directory.join("mihomo");
    std::fs::copy(&binary, &candidate).unwrap();
    let bytes = std::fs::read(&candidate).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let report = json!({"latest":{"channel":"stable","version":"v1.19.32","packageUrl":"https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-darwin-arm64-go122-v1.19.32.gz"},"candidatePath":candidate,"binarySha256":hash});
    std::fs::write(directory.join("candidate.json"), serde_json::to_vec(&report).unwrap()).unwrap();
    let probe = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    let udp = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    let socket = dir.0.join("c.sock");
    let store = Store::new(port);
    let state = stopped("fixture", 0, &socket, port);
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(store.snapshot(state.clone())),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    });
    let mut options = dir.options();
    options.mihomo_path = Some(binary);
    options.mixed_port = Some(port);
    let mut worker = Worker {
        network: network_client::NetworkClient::new(&dir.0, ExecutionMode::Interactive).unwrap(),
        options,
        store,
        runtime_state: state,
        controller: Controller::new(socket.clone(), "fixture-secret".into()).unwrap(),
        secret: "fixture-secret".into(),
        shared,
        scheduler: Scheduler::default(),
        subscription: None,
    };
    drop((probe, udp));
    worker.start_core("start", &Cancellation::default()).await.unwrap();
    worker
        .activate_core(Some(&candidate), "activate", &Cancellation::default())
        .await
        .unwrap();
    assert!(worker.is_running());
    assert_eq!(worker.store.active_core, Some(candidate.clone()));
    let revision = worker.store.revision;
    std::fs::create_dir(dir.0.join("state.pending")).unwrap();
    assert!(
        worker
            .activate_core(None, "reset-fails", &Cancellation::default())
            .await
            .is_err()
    );
    assert!(worker.is_running());
    assert_eq!(worker.store.active_core, Some(candidate));
    assert_eq!(worker.store.revision, revision);
    std::fs::remove_dir(dir.0.join("state.pending")).unwrap();
    worker
        .activate_core(None, "reset", &Cancellation::default())
        .await
        .unwrap();
    assert!(worker.is_running());
    assert!(worker.store.active_core.is_none());
    assert_eq!(worker.store.revision, revision + 1);
    worker.stop_core("stop").await.unwrap();
    assert!(!socket.exists());
    assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
}

#[test]
fn ffi_spans_ownership_and_concurrent_destroy_are_checked() {
    let _serial = lock(&FFI_TEST_LOCK);
    fn assert_thread_safe<T: Send + Sync>() {}
    assert_thread_safe::<Client>();
    assert_eq!(std::mem::size_of::<ffi::CvnBuffer>(), 32);
    assert_eq!(std::mem::offset_of!(ffi::CvnBuffer, status), 24);
    let dir = TestDir::new();
    let options = json!({"version":1,"dataDir":dir.0}).to_string();
    unsafe {
        assert!(ffi::cvn_create(std::ptr::without_provenance(1), 65537).is_null());
        let handle = ffi::cvn_create(options.as_ptr(), options.len() as u64);
        assert!(!handle.is_null());
        let invalid = [0xffu8];
        assert_eq!(ffi::cvn_submit(handle, invalid.as_ptr(), 1), 1);
        let buffer = ffi::cvn_snapshot(handle);
        assert_eq!(buffer.status, 0);
        assert_eq!(
            ffi::cvn_free_buffer(ffi::CvnBuffer {
                length: buffer.length + 1,
                ..buffer
            }),
            1
        );
        assert_eq!(ffi::cvn_free_buffer(buffer), 0);
        assert_eq!(ffi::cvn_free_buffer(buffer), 0);
        let request = json!({"version":1,"id":"中文\0id","command":"snapshot","args":{}}).to_string();
        assert_eq!(ffi::cvn_submit(handle, request.as_ptr(), request.len() as u64), 0);
        let address = handle as usize;
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        let buffer = ffi::cvn_snapshot(std::ptr::without_provenance_mut(address));
                        assert!([0, 1].contains(&buffer.status));
                        ffi::cvn_free_buffer(buffer);
                    }
                })
            })
            .collect();
        assert_eq!(ffi::cvn_stop(handle), 0);
        let status = ffi::cvn_destroy(handle);
        assert!([0, 2].contains(&status));
        for thread in threads {
            thread.join().unwrap();
        }
        while ffi::cvn_destroy(handle) == 2 {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(ffi::cvn_snapshot(handle).status, 1);
    }
}

#[test]
fn supervisor_reaps_owned_mihomo_after_parent_sigkill() {
    use std::io::{BufRead, BufReader};
    let dir = TestDir::integration();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let supervisor = workspace.join(".work/target/aarch64-apple-darwin/debug/clash-native-helper");
    assert!(supervisor.is_file(), "build the native helper before checks");
    let fixture = dir.0.join("parent-fixture");
    let compiler = std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/supervisor_parent.rs"))
        .arg("-o")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(compiler.status.success());
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let socket = dir.0.join("c.sock");
    let config = dir.0.join("runtime.yaml");
    std::fs::create_dir_all(dir.0.join("tmp")).unwrap();
    std::fs::write(&config,format!("mixed-port: {port}\nbind-address: 127.0.0.1\nallow-lan: false\nexternal-controller-unix: {}\ntun: {{enable: false}}\ndns: {{enable: false}}\nproxies: []\nrules: ['MATCH,DIRECT']\n",socket.display())).unwrap();
    let spec = serde_json::to_string(&crate::supervisor::Specification {
        binary: workspace.join(".work/engines/mihomo"),
        home: dir.0.clone(),
        config,
        integration: true,
    })
    .unwrap();
    let mut parent = std::process::Command::new(fixture)
        .arg(supervisor)
        .arg(spec)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let host_pid: u32 = line.trim().parse().unwrap();
    let ready = std::time::Instant::now() + Duration::from_secs(8);
    while !socket.exists() {
        if std::time::Instant::now() >= ready {
            let _ = parent.kill();
            let _ = parent.wait();
            panic!("owned fixture controller never became ready");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while socket.exists() {
        assert!(std::time::Instant::now() < deadline, "socket survived killed parent");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
    // Read-only process check; never terminate by process name.
    assert!(
        unsafe { libc::kill(host_pid as i32, 0) } != 0
            || crate::helper::network::ProcessIdentity::capture(host_pid).is_err()
    );
}

#[test]
fn ffi_full_output_pool_preserves_accepted_result_for_retry() {
    let _serial = lock(&FFI_TEST_LOCK);
    let directory = TestDir::new();
    let options = json!({"version":1,"dataDir":directory.0}).to_string();
    unsafe {
        let handle = ffi::cvn_create(options.as_ptr(), options.len() as u64);
        assert!(!handle.is_null());
        let mut buffers: Vec<_> = (0..64).map(|_| ffi::cvn_snapshot(handle)).collect();
        assert!(buffers.iter().all(|buffer| buffer.status == 0));
        let request = request("retained-result", "snapshot", json!({}));
        let input = serde_json::to_vec(&request).unwrap();
        assert_eq!(ffi::cvn_submit(handle, input.as_ptr(), input.len() as u64), 0);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let event = ffi::cvn_next_event(handle);
            if event.status == 2 {
                break;
            }
            assert_eq!(event.status, 4);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(ffi::cvn_free_buffer(buffers.pop().unwrap()), 0);
        loop {
            let event = ffi::cvn_next_event(handle);
            assert!([0, 4].contains(&event.status));
            if event.status == 4 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let value: Value =
                serde_json::from_slice(std::slice::from_raw_parts(event.data, event.length as usize)).unwrap();
            ffi::cvn_free_buffer(event);
            if value["kind"] != "result" {
                continue;
            }
            assert_eq!(value["id"], "retained-result");
            assert_eq!(value["ok"], true);
            break;
        }
        for buffer in buffers {
            ffi::cvn_free_buffer(buffer);
        }
        assert_eq!(ffi::cvn_next_event(handle).status, 4);
        while ffi::cvn_destroy(handle) == 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn event_bytes_are_bounded_and_metrics_coalesce_without_losing_results() {
    let shared = Shared {
        snapshot: Mutex::new(json!({"revision":1,"runtime":{"status":"running","coreGeneration":1}})),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    };
    for up in 0..1000 {
        shared.stream(1, "traffic", Ok(json!({"up":up})));
    }
    assert_eq!(lock(&shared.events).len(), 1);
    assert_eq!(lock(&shared.events)[0].value["data"]["value"]["up"], 999);
    for _ in 0..1000 {
        shared.stream(1, "logs", Ok(json!({"payload":"x".repeat(32768)})));
    }
    assert!(lock(&shared.events).iter().map(|event| event.bytes).sum::<usize>() <= 4 * 1024 * 1024);
    lock(&shared.pending).insert(
        "accepted".into(),
        Pending {
            token: Arc::new(Cancellation::default()),
            bytes: 10,
            response_bytes: 1024,
        },
    );
    shared.complete("accepted", Ok(json!({"value":"finished"})), None);
    shared.complete("accepted", Err(anyhow::anyhow!("duplicate finish")), None);
    assert_eq!(
        lock(&shared.events)
            .iter()
            .filter(|event| event.value["kind"] == "result")
            .count(),
        1
    );
}

#[test]
fn queued_byte_backpressure_and_worker_failure_complete_only_accepted_requests() {
    let dir = TestDir::new();
    let shared = Arc::new(Shared {
        snapshot: Mutex::new(json!({"revision":0,"runtime":{"status":"stopped","coreGeneration":0}})),
        events: Mutex::new(VecDeque::new()),
        pending: Mutex::new(HashMap::new()),
        stopping: AtomicBool::new(false),
        shutdown: Notify::new(),
    });
    let (sender, _receiver) = mpsc::channel(128);
    let (_complete, completion) = std::sync::mpsc::channel();
    drop(_complete);
    let client = Client {
        sender,
        shared: Arc::clone(&shared),
        runtime: None,
        completion: Mutex::new(completion),
        _directory_lock: File::create(dir.0.join("fixture-lock")).unwrap(),
    };
    assert_eq!(client.submit(request("accepted", "snapshot", json!({}))), 0);
    lock(&shared.pending).get_mut("accepted").unwrap().bytes = MAX_PENDING_BYTES;
    assert_eq!(client.submit(request("rejected", "snapshot", json!({}))), 2);
    let token = lock(&shared.pending)["accepted"].token.clone();
    shared.fail_pending();
    assert!(token.check().is_err());
    assert!(lock(&shared.pending).is_empty());
    let result = client.next_event().unwrap();
    assert_eq!(result["id"], "accepted");
    assert_eq!(result["ok"], false);
    assert!(client.next_event().is_none());
    assert_eq!(client.submit(request("after-failure", "snapshot", json!({}))), 3);
}
