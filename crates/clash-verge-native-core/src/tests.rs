use super::*;
use std::path::PathBuf;
use std::{
    ffi::{CStr, CString},
    io::Write,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
            mixed_port: Some(19882),
        }
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
    assert_eq!(event["coreGeneration"], 2);
    assert_eq!(event["revision"], 4);
    assert_eq!(event["data"]["value"]["up"], 30);
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
        options: dir.options(),
        store,
        runtime_state,
        controller: Controller::new(socket, "secret".into()).unwrap(),
        secret: "secret".into(),
        shared,
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
    let dir = TestDir::new();
    let config = CString::new(json!({"version":1,"dataDir":dir.0,"mixedPort":19884}).to_string()).unwrap();
    unsafe {
        assert!(ffi::cvn_create(std::ptr::null()).is_null());
        let error = ffi::cvn_last_error();
        assert!(!error.is_null());
        assert!(CStr::from_ptr(error).to_str().unwrap().contains("null string"));
        ffi::cvn_free_string(error);
        let handle = ffi::cvn_create(config.as_ptr());
        assert!(!handle.is_null());
        for _ in 0..100 {
            let data = ffi::cvn_snapshot(handle);
            let value: Value = serde_json::from_str(CStr::from_ptr(data).to_str().unwrap()).unwrap();
            assert_eq!(value["version"], 1);
            ffi::cvn_free_string(data);
        }
        assert_eq!(ffi::cvn_submit(handle, std::ptr::null()), 1);
        ffi::cvn_stop(handle);
        let request =
            CString::new(json!({"version":1,"id":"late","command":"snapshot","args":{}}).to_string()).unwrap();
        assert_eq!(ffi::cvn_submit(handle, request.as_ptr()), 3);
        ffi::cvn_destroy(handle);
        ffi::cvn_destroy(std::ptr::null_mut());
        ffi::cvn_free_string(std::ptr::null_mut());
        let reopened = ffi::cvn_create(config.as_ptr());
        assert!(!reopened.is_null());
        ffi::cvn_destroy(reopened);
    }
}
