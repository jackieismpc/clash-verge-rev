//! Native business ownership extracted from the legacy configuration and profile
//! semantics, without importing the Tauri application library.
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("clash-verge-native-core supports Apple Silicon macOS only");

pub mod config;
pub mod controller;
mod ffi;
pub mod helper;
mod isolation;
mod logs;
mod migration;
mod network;
mod network_client;
pub mod platform;
mod rules;
mod runtime_helper;
mod subscriptions;
mod supervisor;
mod update;
mod uri;

use anyhow::{Context, Result, bail};
use config::{
    ExecutionMode, MAX_DOCUMENT_BYTES, Options, Profile, Store, atomic_write, now, parse_profile_yaml, parse_yaml,
};
use controller::{Controller, percent_encode, public_request_allowed, stopped};
use futures_util::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::File,
    io::Read,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subscriptions::{ScheduleProfile, Scheduler, UpdateTicket};
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
};

const MAX_PENDING: usize = 128;
const MAX_EVENTS: usize = 384;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}
fn unique_id() -> String {
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub command: String,
    #[serde(default = "empty_object")]
    pub args: Value,
    pub core_generation: Option<u64>,
    pub revision: Option<u64>,
}
fn json_size(value: &impl Serialize) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(value.len());
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    if serde_json::to_writer(&mut counter, value).is_err() {
        usize::MAX
    } else {
        counter.0
    }
}
fn empty_object() -> Value {
    json!({})
}

#[derive(Default)]
struct Cancellation {
    flag: AtomicBool,
    notify: Notify,
}
impl Cancellation {
    fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        self.notify.notify_one();
    }
    fn check(&self) -> Result<()> {
        if self.flag.load(Ordering::Acquire) {
            bail!("request cancelled");
        }
        Ok(())
    }
    async fn wait(&self) {
        if self.flag.load(Ordering::Acquire) {
            return;
        }
        self.notify.notified().await;
    }
    async fn run<T>(&self, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        tokio::select! { biased; _ = self.wait() => bail!("request cancelled"), value = future => value }
    }
}

#[derive(Clone)]
struct Pending {
    token: Arc<Cancellation>,
    bytes: usize,
    response_bytes: usize,
}
struct Event {
    value: Value,
    bytes: usize,
}
struct Shared {
    snapshot: Mutex<Value>,
    events: Mutex<VecDeque<Event>>,
    pending: Mutex<HashMap<String, Pending>>,
    stopping: AtomicBool,
    shutdown: Notify,
}
impl Shared {
    fn fail_pending(&self) {
        self.stopping.store(true, Ordering::Release);
        let pending: Vec<_> = lock(&self.pending)
            .iter()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        for (id, value) in pending {
            value.token.cancel();
            self.complete(
                &id,
                Err(anyhow::anyhow!("native worker terminated; owned cleanup requested")),
                None,
            );
        }
    }
    fn complete(&self, id: &str, outcome: Result<Value>, version: Option<(u64, u64)>) {
        let mut pending = lock(&self.pending);
        if pending.remove(id).is_none() {
            return;
        }
        match version {
            Some((generation, revision)) => self.publish_read_result(id, generation, revision, outcome),
            None => self.publish(id, "result", outcome),
        }
    }
    fn envelope(&self, id: &str, kind: &str, outcome: Result<Value>) -> Value {
        let snapshot = lock(&self.snapshot);
        let generation = snapshot["runtime"]["coreGeneration"].as_u64().unwrap_or(0);
        let revision = snapshot["revision"].as_u64().unwrap_or(0);
        match outcome {
            Ok(data) => {
                json!({"version":1,"id":id,"kind":kind,"coreGeneration":generation,"revision":revision,"ok":true,"data":data})
            }
            Err(error) => {
                let message = format!("{error:#}").chars().take(8192).collect::<String>();
                let code = if message.starts_with("request cancelled") {
                    "cancelled"
                } else if message.starts_with("stale core generation") {
                    "staleGeneration"
                } else if message.starts_with("stale revision") {
                    "staleRevision"
                } else if message.starts_with("unsupported") {
                    "unsupported"
                } else {
                    "operationFailed"
                };
                json!({"version":1,"id":id,"kind":kind,"coreGeneration":generation,"revision":revision,"ok":false,"error":{"code":code,"message":message}})
            }
        }
    }
    fn enqueue(&self, mut event: Value) {
        let kind = event["kind"].as_str().unwrap_or("").to_owned();
        let topic = event["data"]["stream"].as_str().map(str::to_owned);
        let mut events = lock(&self.events);
        if kind == "state" {
            events.retain(|event| event.value["kind"] != "state");
        }
        if kind == "stream"
            && topic
                .as_deref()
                .is_some_and(|topic| ["traffic", "memory", "connections"].contains(&topic))
        {
            events.retain(|old| {
                old.value["kind"] != "stream" || old.value["data"]["stream"].as_str() != topic.as_deref()
            });
        }
        let mut bytes = json_size(&event);
        let mut total = events.iter().map(|event| event.bytes).sum::<usize>();
        if total.saturating_add(bytes) > 128 * 1024 * 1024 && kind != "stream" {
            events.retain(|event| event.value["kind"] != "stream");
            total = events.iter().map(|event| event.bytes).sum();
        }
        if kind == "state" && total.saturating_add(bytes) > 128 * 1024 * 1024 {
            // The authoritative snapshot remains readable when delivery is congested.
            return;
        }
        if kind == "stream"
            && (events.len() >= 256
                || bytes > 4 * 1024 * 1024
                || events
                    .iter()
                    .filter(|event| event.value["kind"] == "stream")
                    .map(|event| event.bytes)
                    .sum::<usize>()
                    + bytes
                    > 4 * 1024 * 1024)
        {
            return;
        }
        if kind == "result" && total.saturating_add(bytes) > 128 * 1024 * 1024 {
            let succeeded = event["ok"].clone();
            if let Some(object) = event.as_object_mut() {
                object.remove("data");
            }
            event["ok"] = false.into();
            event["operationSucceeded"] = succeeded;
            event["error"] = json!({"code":"resultUnavailable","message":"result buffer capacity exceeded; inspect the authoritative snapshot before retrying a mutation"});
            bytes = json_size(&event);
        }
        events.push_back(Event { value: event, bytes });
    }
    fn publish(&self, id: &str, kind: &str, outcome: Result<Value>) {
        self.enqueue(self.envelope(id, kind, outcome));
    }
    fn publish_read_result(&self, id: &str, generation: u64, revision: u64, outcome: Result<Value>) {
        let mut event = self.envelope(id, "result", outcome);
        event["coreGeneration"] = generation.into();
        event["revision"] = revision.into();
        self.enqueue(event);
    }
    fn state(&self, store: &Store, runtime: &Value, id: &str) {
        let data = store.snapshot(runtime.clone());
        *lock(&self.snapshot) = data.clone();
        self.publish(id, "state", Ok(data));
    }
    fn stream(&self, generation: u64, stream: &str, outcome: Result<Value>) {
        self.stream_inner(generation, None, stream, outcome);
    }
    fn stream_at_revision(&self, generation: u64, revision: u64, stream: &str, outcome: Result<Value>) {
        self.stream_inner(generation, Some(revision), stream, outcome);
    }
    fn stream_inner(&self, generation: u64, expected_revision: Option<u64>, stream: &str, outcome: Result<Value>) {
        let snapshot = lock(&self.snapshot);
        if snapshot["runtime"]["coreGeneration"].as_u64() != Some(generation)
            || snapshot["runtime"]["status"] != "running"
            || expected_revision.is_some_and(|revision| snapshot["revision"].as_u64() != Some(revision))
        {
            return;
        }
        let revision = snapshot["revision"].clone();
        let event = match outcome {
            Ok(value) => {
                json!({"version":1,"id":"","kind":"stream","coreGeneration":generation,"revision":revision,"ok":true,"data":{"stream":stream,"value":value}})
            }
            Err(error) => {
                json!({"version":1,"id":"","kind":"stream","coreGeneration":generation,"revision":revision,"ok":false,"error":{"code":"streamFailed","message":format!("{stream}: {error:#}")}})
            }
        };
        let mut event = event;
        event["timestamp"] = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            .into();
        drop(snapshot);
        self.enqueue(event);
    }
}

struct Queued {
    request: Request,
    cancellation: Arc<Cancellation>,
}

/// Safe Rust interface behind the C ABI. Submission is nonblocking; all mutation
/// and I/O run on the client's own bounded worker and Tokio runtime.
pub struct Client {
    sender: mpsc::Sender<Queued>,
    shared: Arc<Shared>,
    runtime: Option<Runtime>,
    completion: Mutex<std::sync::mpsc::Receiver<()>>,
    _directory_lock: File,
}

impl Client {
    pub fn open(options: Options) -> Result<Self> {
        options.validate()?;
        std::fs::create_dir_all(&options.data_dir)?;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(&options.data_dir, std::fs::Permissions::from_mode(0o700))?;
        if options.execution_mode == ExecutionMode::IntegrationTest {
            std::fs::create_dir_all(options.data_dir.join("tmp"))?;
            std::fs::set_permissions(options.data_dir.join("tmp"), std::fs::Permissions::from_mode(0o700))?;
        }
        let directory_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(options.data_dir.join("owner.lock"))?;
        directory_lock
            .try_lock()
            .context("data directory already has an owner")?;
        for filename in [
            "state.pending",
            "runtime.pending",
            "runtime-staged.pending",
            "runtime-rollback.pending",
            "network-ownership.pending",
        ] {
            match std::fs::remove_file(options.data_dir.join(filename)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let owner = unique_id();
        let socket = options.data_dir.join("c.sock");
        if socket.as_os_str().len() > 100 {
            bail!("dataDir is too long for a macOS Unix controller socket");
        }
        let mut bytes = [0_u8; 32];
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let store_path = options.data_dir.join("state.json");
        let store: Store = if store_path.exists() {
            let bytes = std::fs::read(&store_path)?;
            if bytes.len() > MAX_DOCUMENT_BYTES * 4 {
                bail!("state exceeds size limit");
            }
            serde_json::from_slice(&bytes).context("read persisted native state")?
        } else {
            Store::new(options.mixed_port.unwrap_or(17890))
        };
        store.validate()?;
        let mut runtime_state = stopped(&owner, 0, &socket, store.settings.mixed_port);
        runtime_state["executionMode"] = if options.execution_mode == ExecutionMode::IntegrationTest {
            "integrationTest"
        } else {
            "interactive"
        }
        .into();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(store.snapshot(runtime_state.clone())),
            events: Mutex::new(VecDeque::new()),
            pending: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            shutdown: Notify::new(),
        });
        let (sender, receiver) = mpsc::channel(MAX_PENDING);
        let (complete, completion) = std::sync::mpsc::channel();
        let mut controller = Controller::new(socket, secret.clone())?;
        controller.supervisor = options.host_path.clone();
        controller.integration_test = options.execution_mode == ExecutionMode::IntegrationTest;
        let mut worker = Worker {
            network: network_client::NetworkClient::new(&options.data_dir, options.execution_mode)?,
            options,
            store,
            runtime_state,
            controller,
            secret,
            shared: Arc::clone(&shared),
            scheduler: Scheduler::default(),
            subscription: None,
        };
        runtime.spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(worker.serve(receiver))
                .catch_unwind()
                .await;
            if outcome.is_err() {
                worker.shared.fail_pending();
                let _ = std::panic::AssertUnwindSafe(worker.stop_core("")).catch_unwind().await;
            }
            let _ = complete.send(());
        });
        Ok(Self {
            sender,
            shared,
            runtime: Some(runtime),
            completion: Mutex::new(completion),
            _directory_lock: directory_lock,
        })
    }
    pub fn submit(&self, request: Request) -> i32 {
        if request.version != 1
            || request.id.is_empty()
            || request.id.len() > 256
            || request.command.len() > 128
            || !request.args.is_object()
        {
            return 1;
        }
        if self.shared.stopping.load(Ordering::Acquire) {
            return 3;
        }
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .ok_or_else(|| std::io::Error::other("request size overflow"))?;
                if self.0 > MAX_PENDING_BYTES {
                    return Err(std::io::Error::other("request exceeds limit"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        if serde_json::to_writer(&mut counter, &request).is_err() {
            return 1;
        }
        let snapshot_size = json_size(&*lock(&self.shared.snapshot));
        let response_bytes = if ["profile.import", "profile.update", "legacy.import", "backup.restore"]
            .contains(&request.command.as_str())
        {
            64 * 1024 * 1024
        } else {
            snapshot_size
                .saturating_add(counter.0)
                .saturating_add(65536)
                .max(8 * 1024 * 1024)
        };
        let mut pending = lock(&self.shared.pending);
        if self.shared.stopping.load(Ordering::Acquire) || self.sender.is_closed() {
            return 3;
        }
        if pending.len() >= MAX_PENDING
            || pending.contains_key(&request.id)
            || lock(&self.shared.events).len() + pending.len() >= MAX_EVENTS
            || pending.values().map(|value| value.bytes).sum::<usize>() + counter.0 > MAX_PENDING_BYTES
            || pending
                .values()
                .map(|value| value.response_bytes)
                .sum::<usize>()
                .saturating_add(response_bytes)
                .saturating_add(lock(&self.shared.events).iter().map(|event| event.bytes).sum::<usize>())
                > 128 * 1024 * 1024
        {
            return 2;
        }
        let cancellation = Arc::new(Cancellation::default());
        let id = request.id.clone();
        pending.insert(
            id.clone(),
            Pending {
                token: Arc::clone(&cancellation),
                bytes: counter.0,
                response_bytes,
            },
        );
        match self.sender.try_send(Queued { request, cancellation }) {
            Ok(()) => 0,
            Err(reason) => {
                pending.remove(&id);
                if matches!(reason, tokio::sync::mpsc::error::TrySendError::Closed(_)) {
                    3
                } else {
                    2
                }
            }
        }
    }
    pub fn cancel(&self, id: &str) -> bool {
        if let Some(token) = lock(&self.shared.pending).get(id) {
            token.token.cancel();
            true
        } else {
            false
        }
    }
    pub fn snapshot(&self) -> Value {
        let data = lock(&self.shared.snapshot).clone();
        json!({"version":1,"id":"","kind":"snapshot","coreGeneration":data["runtime"]["coreGeneration"],
            "revision":data["revision"],"ok":true,"data":data})
    }
    pub fn next_event(&self) -> Option<Value> {
        lock(&self.shared.events).pop_front().map(|event| event.value)
    }
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::Release);
        for token in lock(&self.shared.pending).values() {
            token.token.cancel();
        }
        self.shared.shutdown.notify_one();
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
        let _ = lock(&self.completion).recv();
        drop(self.runtime.take());
    }
}

struct Worker {
    options: Options,
    store: Store,
    runtime_state: Value,
    controller: Controller,
    secret: String,
    shared: Arc<Shared>,
    scheduler: Scheduler,
    subscription: Option<SubscriptionTask>,
    network: network_client::NetworkClient,
}
struct SubscriptionTask {
    ticket: UpdateTicket,
    cancel: Arc<Cancellation>,
    task: tokio::task::JoinHandle<Result<(String, Value)>>,
}
impl Worker {
    fn active_binary(&self) -> Result<std::path::PathBuf> {
        match &self.store.active_core {
            Some(path) => update::validate_owned_candidate(&self.options.data_dir, path),
            None => self
                .options
                .mihomo_path
                .clone()
                .context("mihomoPath is required to start the owned core"),
        }
    }
    async fn activate_core(&mut self, path: Option<&std::path::Path>, id: &str, cancel: &Cancellation) -> Result<()> {
        if self.runtime_state["helperMode"] == true {
            bail!("disable TUN before changing the active core");
        }
        let path = path
            .map(|path| update::validate_owned_candidate(&self.options.data_dir, path))
            .transpose()?;
        let binary = path
            .clone()
            .or_else(|| self.options.mihomo_path.clone())
            .context("bundled core is unavailable")?;
        let sign = tokio::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&binary)
            .kill_on_drop(true)
            .output();
        let output = cancel
            .run(async { Ok(tokio::time::timeout(Duration::from_secs(15), sign).await??) })
            .await?;
        if !output.status.success() {
            bail!("core code signature is invalid");
        }
        let report = if let Some(path) = &path {
            let report: Value = serde_json::from_slice(
                &tokio::fs::read(
                    path.parent()
                        .context("missing candidate directory")?
                        .join("candidate.json"),
                )
                .await?,
            )?;
            Some(
                json!({"channel":report["latest"]["channel"],"version":report["latest"]["version"],"candidatePath":path,"activated":true}),
            )
        } else {
            None
        };
        let yaml = self
            .store
            .owned_runtime_yaml(&self.controller.socket, &self.secret, &self.options)?;
        let config = self.options.data_dir.join("runtime-staged.yaml");
        atomic_write(&config, yaml.as_bytes()).await?;
        cancel
            .run(Controller::validate(&binary, &config, &self.options.data_dir, false))
            .await?;
        cancel.check()?;
        let prior = self.store.clone();
        let was_running = self.is_running();
        let proxy_on = self.network.proxy_on;
        let dns_servers = self.network.dns_servers.clone();
        if was_running {
            self.stop_core(id).await?;
        }
        self.store.active_core = path;
        self.store.core_update = report;
        let outcome = async {
            if was_running {
                self.start_core(id, &Cancellation::default()).await?;
            }
            self.commit(self.store.clone(), id, &Cancellation::default()).await
        }
        .await;
        if let Err(error) = outcome {
            if self.store.revision != prior.revision {
                self.restore_network_modes(proxy_on, dns_servers)?;
                return Err(error.context("new core already committed; durability warning"));
            }
            let _ = self.stop_core(id).await;
            self.store = prior;
            let restart = if was_running {
                self.start_core(id, &Cancellation::default()).await
            } else {
                Ok(())
            };
            if restart.is_ok() {
                self.restore_network_modes(proxy_on, dns_servers)?;
            }
            self.publish_state(id);
            return Err(error.context(format!("core change failed; prior core restore: {restart:?}")));
        }
        self.restore_network_modes(proxy_on, dns_servers)?;
        self.publish_state(id);
        Ok(())
    }
    fn restore_network_modes(&mut self, proxy: bool, dns: Vec<String>) -> Result<()> {
        self.network.core_pid = self.runtime_state["pid"].as_u64().unwrap_or(0) as u32;
        if proxy {
            self.network.set_proxy(true, self.store.settings.mixed_port)?;
        }
        if !dns.is_empty() {
            self.network.set_dns(dns, self.store.settings.mixed_port)?;
        }
        Ok(())
    }
    async fn serve(&mut self, mut receiver: mpsc::Receiver<Queued>) {
        self.publish_state("");
        let mut health = tokio::time::interval(Duration::from_secs(1));
        loop {
            let queued = tokio::select! { biased;
                _ = self.shared.shutdown.notified() => break,
                queued = receiver.recv() => match queued { Some(queued) => queued, None => break },
                _ = health.tick() => { self.check_core_health().await; self.update_subscriptions().await; continue; },
            };
            let id = queued.request.id.clone();
            if ["proxy.delay", "media.check", "media.checkAll"].contains(&queued.request.command.as_str())
                && self.is_running()
                && queued
                    .request
                    .core_generation
                    .is_none_or(|generation| generation == self.generation())
                && queued
                    .request
                    .revision
                    .is_none_or(|revision| revision == self.store.revision)
            {
                self.controller.read_tasks.retain(|(_, task)| !task.is_finished());
                let controller = self.controller.reader();
                let shared = Arc::clone(&self.shared);
                let generation = self.generation();
                let revision = self.store.revision;
                let port = self.store.settings.mixed_port;
                let token = Arc::clone(&queued.cancellation);
                let task = tokio::spawn(async move {
                    let mut outcome = std::panic::AssertUnwindSafe(background_read(
                        &controller,
                        &shared,
                        generation,
                        revision,
                        port,
                        &queued.request,
                        &queued.cancellation,
                    ))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("background read panicked")));
                    {
                        let current = lock(&shared.snapshot);
                        if current["runtime"]["coreGeneration"].as_u64() != Some(generation) {
                            outcome = Err(anyhow::anyhow!("stale core generation"));
                        } else if current["revision"].as_u64() != Some(revision) {
                            outcome = Err(anyhow::anyhow!("stale revision"));
                        }
                    }
                    shared.complete(&id, outcome, Some((generation, revision)));
                });
                self.controller.read_tasks.push((token, task));
                continue;
            }
            let outcome = std::panic::AssertUnwindSafe(self.execute(&queued.request, &queued.cancellation))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("native command panicked")));
            self.shared.complete(&id, outcome, None);
        }
        receiver.close();
        self.scheduler.cancel_all();
        if let Some(subscription) = self.subscription.take() {
            subscription.cancel.cancel();
            let _ = subscription.task.await;
        }
        while let Some(queued) = receiver.recv().await {
            self.shared.complete(
                &queued.request.id,
                Err(anyhow::anyhow!("request cancelled during shutdown")),
                None,
            );
        }
        let outcome = self.stop_core("").await;
        if let Err(error) = outcome {
            self.shared.publish("", "state", Err(error));
        }
    }
    fn generation(&self) -> u64 {
        self.runtime_state["coreGeneration"].as_u64().unwrap_or(0)
    }
    fn publish_state(&self, id: &str) {
        let mut runtime = self.runtime_state.clone();
        let mut network = self.network.state();
        if self.runtime_state["helperMode"] == true {
            network["tun"] = if self.is_running() { "on" } else { "failed" }.into();
        }
        runtime["network"] = network;
        self.shared.state(&self.store, &runtime, id);
    }
    fn is_running(&self) -> bool {
        self.runtime_state["status"] == "running"
    }

    fn scheduled_profiles(&self) -> Vec<ScheduleProfile> {
        self.store
            .profiles
            .iter()
            .filter_map(|profile| {
                Some(ScheduleProfile {
                    id: profile.id.clone(),
                    url: profile.url.clone()?,
                    updated_at: profile.updated_at,
                    interval_minutes: profile.update_interval_minutes,
                })
            })
            .collect()
    }
    async fn cancel_subscription(&mut self, id: &str) {
        self.scheduler.cancel(id);
        if self
            .subscription
            .as_ref()
            .is_some_and(|task| task.ticket.profile.id == id)
        {
            let task = self.subscription.take();
            if let Some(task) = task {
                task.cancel.cancel();
                task.task.abort();
                let _ = task.task.await;
            }
        }
    }
    async fn update_subscriptions(&mut self) {
        let profiles = self.scheduled_profiles();
        if self
            .subscription
            .as_ref()
            .is_some_and(|task| !self.scheduler.is_current(&task.ticket, &profiles))
            && let Some(task) = self.subscription.take()
        {
            task.cancel.cancel();
            task.task.abort();
            let _ = task.task.await;
            self.scheduler.cancel(&task.ticket.profile.id);
        }
        if self.subscription.as_ref().is_some_and(|task| task.task.is_finished())
            && let Some(task) = self.subscription.take()
        {
            let result = task
                .task
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("subscription task interrupted")));
            if self.scheduler.is_current(&task.ticket, &self.scheduled_profiles()) {
                let outcome = match result {
                    Ok((yaml, metadata)) => {
                        let mut candidate = self.store.clone();
                        match candidate.profile_mut(&task.ticket.profile.id) {
                            Ok(profile) => {
                                profile.yaml = yaml;
                                profile.updated_at = now();
                                profile.subscription_info =
                                    serde_json::from_value(metadata["subscriptionInfo"].clone()).unwrap_or(None);
                                self.commit(candidate, "subscription:auto", &Cancellation::default())
                                    .await
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                };
                self.scheduler.finish(&task.ticket, now(), outcome.is_ok());
                if let Err(error) = outcome {
                    self.shared.publish(
                        "subscription:auto",
                        "state",
                        Err(error.context("automatic subscription update failed; saved profile retained")),
                    );
                }
            }
        }
        if self.subscription.is_none()
            && let Some(ticket) = self.scheduler.claim_due(&self.scheduled_profiles(), now())
        {
            let url = ticket.profile.url.clone();
            let cancel = Arc::new(Cancellation::default());
            let task_cancel = Arc::clone(&cancel);
            let task = tokio::spawn(async move { fetch_profile(&url, &task_cancel).await });
            self.subscription = Some(SubscriptionTask { ticket, cancel, task });
        }
    }

    async fn execute(&mut self, request: &Request, cancel: &Cancellation) -> Result<Value> {
        if self.options.execution_mode == ExecutionMode::IntegrationTest
            && [
                "systemProxy.",
                "systemDns.",
                "dns.",
                "route.",
                "tun.",
                "helper.",
                "login.",
                "autostart.",
                "network.",
            ]
            .iter()
            .any(|prefix| request.command.starts_with(prefix))
        {
            bail!("integrationTest forbids system networking, helper and login operations");
        }
        cancel.check()?;
        if request
            .core_generation
            .is_some_and(|generation| generation != self.generation())
        {
            bail!("stale core generation");
        }
        if request.revision.is_some_and(|revision| revision != self.store.revision) {
            bail!("stale revision");
        }
        let args = &request.args;
        match request.command.as_str() {
            "snapshot" => Ok(self.store.snapshot(self.runtime_state.clone())),
            "profile.import" => {
                config::reject_script_source(args)?;
                if ["yaml", "url", "uri"]
                    .iter()
                    .filter(|key| args.get(**key).is_some())
                    .count()
                    != 1
                {
                    bail!("exactly one of yaml, url or uri is required");
                }
                let url = args.get("url").and_then(Value::as_str);
                let (yaml, metadata) = match args.get("yaml").and_then(Value::as_str) {
                    Some(yaml) => (yaml.to_string(), json!({})),
                    None if args.get("uri").is_some() => {
                        let proxy = uri::parse_uri(string(args, "uri")?)?;
                        let node_name = proxy["name"].as_str().context("invalid proxy name")?;
                        let profile = json!({"proxies": [proxy], "proxy-groups": [{"name": "Imported", "type": "select", "proxies": [node_name]}], "rules": ["MATCH,Imported"]});
                        (serde_yaml_ng::to_string(&profile)?, json!({}))
                    }
                    None => fetch_profile(url.context("yaml or url is required")?, cancel).await?,
                };
                let parsed = parse_profile_yaml(&yaml)?;
                if url.is_some() && parsed.get("proxies").is_none() && parsed.get("proxy-providers").is_none() {
                    bail!("remote profile must contain proxies or proxy-providers");
                }
                let profile = Profile {
                    id: unique_id(),
                    name: args
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Imported Profile")
                        .into(),
                    kind: if url.is_some() { "remote" } else { "local" }.into(),
                    yaml,
                    url: url.map(str::to_string),
                    updated_at: now(),
                    subscription_info: serde_json::from_value(metadata["subscriptionInfo"].clone()).unwrap_or(None),
                    update_interval_minutes: metadata["updateIntervalMinutes"].as_u64(),
                    home: metadata["home"].as_str().map(str::to_string),
                    selected_nodes: BTreeMap::new(),
                };
                let mut candidate = self.store.clone();
                if candidate.selected_profile_id.is_none() {
                    candidate.selected_profile_id = Some(profile.id.clone());
                }
                candidate.profiles.push(profile.clone());
                self.commit(candidate, &request.id, cancel).await?;
                Ok(serde_json::to_value(profile)?)
            }
            "profile.update" => {
                let id = string(args, "id")?;
                self.cancel_subscription(id).await;
                let old = self.store.profile(id)?.clone();
                let (yaml, metadata) = match args.get("yaml").and_then(Value::as_str) {
                    Some(yaml) => (yaml.to_string(), None),
                    None => {
                        let (yaml, metadata) =
                            fetch_profile(old.url.as_deref().context("local profile needs yaml")?, cancel).await?;
                        (yaml, Some(metadata))
                    }
                };
                parse_profile_yaml(&yaml)?;
                let mut candidate = self.store.clone();
                let profile = candidate.profile_mut(id)?;
                profile.yaml = yaml;
                profile.updated_at = now();
                if let Some(metadata) = metadata {
                    profile.subscription_info =
                        serde_json::from_value(metadata["subscriptionInfo"].clone()).unwrap_or(None);
                    if profile.update_interval_minutes.is_none() {
                        profile.update_interval_minutes = metadata["updateIntervalMinutes"].as_u64();
                    }
                    profile.home = metadata["home"].as_str().map(str::to_string);
                }
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "profile.select" | "profile.delete" | "profile.rename" => {
                let id = string(args, "id")?;
                self.store.profile(id)?;
                if request.command == "profile.delete" {
                    self.cancel_subscription(id).await;
                }
                let mut candidate = self.store.clone();
                match request.command.as_str() {
                    "profile.select" => candidate.selected_profile_id = Some(id.into()),
                    "profile.delete" => {
                        candidate.profiles.retain(|profile| profile.id != id);
                        if candidate.selected_profile_id.as_deref() == Some(id) {
                            candidate.selected_profile_id = candidate.profiles.first().map(|p| p.id.clone());
                        }
                    }
                    _ => {
                        candidate.profile_mut(id)?.name = string(args, "name")?.into();
                    }
                }
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "overrides.set" => {
                let mut candidate = self.store.clone();
                candidate.overrides = string(args, "yaml")?.into();
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "subscription.configure" => {
                let id = string(args, "id")?;
                let interval = args
                    .get("intervalMinutes")
                    .and_then(Value::as_u64)
                    .context("intervalMinutes must be a nonnegative integer")?;
                subscriptions::validate_interval(Some(interval))?;
                self.cancel_subscription(id).await;
                let mut candidate = self.store.clone();
                let profile = candidate.profile_mut(id)?;
                if profile.url.is_none() {
                    bail!("local profiles have no subscription schedule");
                }
                profile.update_interval_minutes = Some(interval);
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "rules.overrides.get" => Ok(serde_json::to_value(&self.store.rule_overrides)?),
            "rules.overrides.set" => {
                let mut candidate = self.store.clone();
                candidate.rule_overrides = serde_json::from_value(args.clone())?;
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "legacy.import" => {
                let source = std::path::PathBuf::from(string(args, "path")?);
                let fallback = self.options.data_dir.join("legacy-fallback");
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::create_dir_all(&fallback).await?;
                tokio::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o700)).await?;
                let imported = cancel
                    .run(async {
                        tokio::task::spawn_blocking(move || migration::inspect_legacy(&source, &fallback)).await?
                    })
                    .await?;
                let mut candidate = self.store.clone();
                if imported
                    .profiles
                    .iter()
                    .any(|p| candidate.profiles.iter().any(|existing| existing.id == p.id))
                {
                    bail!("legacy profiles already imported; remove duplicates before importing again");
                }
                if candidate.selected_profile_id.is_none() {
                    candidate.selected_profile_id = imported.selected_profile_id.clone();
                }
                candidate.profiles.extend(imported.profiles);
                candidate.migration_warnings = imported.warnings;
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "core.update.check" | "core.update.stage" => {
                if self.options.execution_mode == ExecutionMode::IntegrationTest {
                    bail!("integrationTest forbids remote core updates");
                }
                let alpha = match args.get("channel").and_then(Value::as_str).unwrap_or("stable") {
                    "stable" => false,
                    "alpha" => true,
                    _ => bail!("unknown core channel"),
                };
                let latest = if request.command == "core.update.check" {
                    let report = cancel.run(update::check_latest(alpha)).await?;
                    json!({"channel":report.channel,"version":report.version,"candidatePath":null,"activated":false})
                } else {
                    let report = cancel.run(update::stage_latest(&self.options.data_dir, alpha)).await?;
                    json!({"channel":report.latest.channel,"version":report.latest.version,"candidatePath":report.candidate_path,"activated":false})
                };
                let mut candidate = self.store.clone();
                candidate.core_update = Some(latest);
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "core.update.activate" => {
                if self.options.execution_mode == ExecutionMode::IntegrationTest {
                    bail!("integrationTest forbids core activation");
                }
                let path = std::path::PathBuf::from(string(args, "candidatePath")?);
                self.activate_core(Some(&path), &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "core.update.reset" => {
                if self.options.execution_mode == ExecutionMode::IntegrationTest {
                    bail!("integrationTest forbids core activation");
                }
                self.activate_core(None, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "settings.patch" => {
                let mut candidate = self.store.clone();
                if let Some(mode) = args.get("mode") {
                    candidate.settings.mode = mode.as_str().context("mode must be a string")?.into();
                }
                if let Some(port) = args.get("mixedPort") {
                    candidate.settings.mixed_port =
                        u16::try_from(port.as_u64().context("mixedPort must be an integer")?)?;
                }
                if (self.is_running() || self.controller.child.is_some() || self.runtime_state["helperMode"] == true)
                    && candidate.settings.mixed_port != self.store.settings.mixed_port
                {
                    bail!("stop the owned core before changing mixedPort");
                }
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "runtime.start" => {
                self.start_core(&request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "runtime.stop" => {
                self.stop_core(&request.id).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "runtime.restart" => {
                let proxy_on = self.network.proxy_on;
                let dns_servers = self.network.dns_servers.clone();
                self.stop_core(&request.id).await?;
                cancel.check()?;
                self.start_core(&request.id, cancel).await?;
                self.restore_network_modes(proxy_on, dns_servers)?;
                self.publish_state(&request.id);
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "core.request" => self.core_request(args, &request.id, cancel).await,
            "core.subscribe" => {
                if !self.is_running() {
                    bail!("owned core is stopped");
                }
                let names: Vec<String> =
                    serde_json::from_value(args.get("streams").context("streams required")?.clone())?;
                let shared = Arc::clone(&self.shared);
                self.controller
                    .subscribe(&names, self.generation(), move |generation, stream, data| {
                        shared.stream(generation, stream, data);
                    })?;
                Ok(json!({"streams":names}))
            }
            "core.unsubscribe" => {
                self.controller.unsubscribe();
                Ok(Value::Null)
            }
            "media.list" => Ok(serde_json::to_value(clash_verge_media_unlock::default_unlock_items())?),
            "proxy.delay" | "media.check" | "media.checkAll" => {
                if !self.is_running() {
                    bail!("owned core is stopped");
                }
                background_read(
                    &self.controller,
                    &self.shared,
                    self.generation(),
                    self.store.revision,
                    self.store.settings.mixed_port,
                    request,
                    cancel,
                )
                .await
            }
            "backup.create" => self.backup_create().await,
            "backup.list" => self.backup_list().await,
            "backup.restore" => {
                let id = safe_backup_id(string(args, "id")?)?;
                let bytes = tokio::fs::read(self.options.data_dir.join("backups").join(id)).await?;
                if bytes.len() > MAX_DOCUMENT_BYTES * 4 {
                    bail!("backup exceeds size limit");
                }
                let candidate: Store = serde_json::from_slice(&bytes)?;
                let mut candidate = candidate;
                candidate.active_core = self.store.active_core.clone();
                candidate.core_update = self.store.core_update.clone();
                self.scheduler.cancel_all();
                if let Some(task) = self.subscription.take() {
                    task.cancel.cancel();
                    task.task.abort();
                    let _ = task.task.await;
                }
                self.commit(candidate, &request.id, cancel).await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "systemProxy.set" => {
                let enabled = args
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .context("enabled is required")?;
                if enabled && !self.is_running() {
                    bail!("start the owned core before enabling system proxy");
                }
                self.network.core_pid = self.runtime_state["pid"].as_u64().unwrap_or(0) as u32;
                let outcome = self.network.set_proxy(enabled, self.store.settings.mixed_port);
                self.publish_state(&request.id);
                outcome?;
                Ok(lock(&self.shared.snapshot).clone())
            }
            "systemDns.set" => {
                if !self.is_running() {
                    bail!("start the owned core before configuring system DNS");
                }
                let servers: Vec<String> =
                    serde_json::from_value(args.get("servers").context("servers required")?.clone())?;
                self.network.core_pid = self.runtime_state["pid"].as_u64().unwrap_or(0) as u32;
                let outcome = self.network.set_dns(servers, self.store.settings.mixed_port);
                self.publish_state(&request.id);
                outcome?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            "network.restore" => {
                let outcome = self.network.restore();
                self.publish_state(&request.id);
                outcome?;
                Ok(Value::Null)
            }
            "helper.status" => {
                let status = helper::request(
                    &helper::HelperCommand::Status,
                    self.options.execution_mode == ExecutionMode::IntegrationTest,
                )?;
                Ok(serde_json::to_value(status)?)
            }
            "tun.set" => {
                self.set_tun(
                    args.get("enabled")
                        .and_then(Value::as_bool)
                        .context("enabled required")?,
                    &request.id,
                    cancel,
                )
                .await?;
                Ok(self.store.snapshot(self.runtime_state.clone()))
            }
            _ => bail!("unsupported command {}", request.command),
        }
    }

    async fn commit(&mut self, mut candidate: Store, id: &str, cancel: &Cancellation) -> Result<()> {
        candidate.revision = self.store.revision.checked_add(1).context("revision exhausted")?;
        candidate.validate()?;
        let persisted_bytes = serde_json::to_vec_pretty(&candidate)?;
        if persisted_bytes.len() > MAX_DOCUMENT_BYTES * 4 {
            bail!("native state exceeds 64 MiB limit");
        }
        let yaml = candidate.owned_runtime_yaml(&self.controller.socket, &self.secret, &self.options)?;
        let staged = self.options.data_dir.join("runtime-staged.yaml");
        atomic_write(&staged, yaml.as_bytes()).await?;
        if self.options.mihomo_path.is_some() || self.store.active_core.is_some() {
            let binary = self.active_binary()?;
            cancel
                .run(Controller::validate(
                    &binary,
                    &staged,
                    &self.options.data_dir,
                    self.controller.integration_test,
                ))
                .await?;
        }
        cancel.check()?;
        // Cancellation is deferred after this boundary: runtime reload and durable
        // publication either finish together or restore the previous runtime.
        let reload = if self.is_running() {
            if self.runtime_state["helperMode"] == true {
                self.reload_helper(&yaml)
            } else {
                self.controller
                    .request("PUT", "/configs?force=true", Some(&json!({"path":staged})))
                    .await
                    .map(|_| ())
            }
        } else {
            Ok(())
        };
        if let Err(error) = reload {
            self.restore_runtime()
                .await
                .context("failed to restore runtime after rejected reload")?;
            return Err(error);
        }
        let path = self.options.data_dir.join("state.json");
        let persistence_warning = match atomic_write(&path, &persisted_bytes).await {
            Ok(()) => None,
            Err(error) if error.downcast_ref::<config::CommittedWriteError>().is_some() => Some(error),
            Err(error) => {
                if self.is_running() {
                    self.restore_runtime()
                        .await
                        .context("persistence failed and runtime rollback failed")?;
                }
                return Err(error);
            }
        };
        self.store = candidate;
        self.runtime_state["mixedPort"] = self.store.settings.mixed_port.into();
        self.publish_state(id);
        if let Err(error) = atomic_write(&self.options.data_dir.join("runtime.yaml"), yaml.as_bytes()).await {
            // The persisted state and applied runtime are already committed.
            self.shared.publish(
                id,
                "state",
                Err(anyhow::anyhow!(
                    "state committed; runtime mirror write failed: {error:#}"
                )),
            );
        }
        if self.is_running() {
            self.restore_selections(id).await;
        }
        if let Some(error) = persistence_warning {
            return Err(error.context("configuration committed; durability confirmation failed"));
        }
        Ok(())
    }

    async fn restore_runtime(&mut self) -> Result<()> {
        let path = self.options.data_dir.join("runtime-rollback.yaml");
        let yaml = self
            .store
            .owned_runtime_yaml(&self.controller.socket, &self.secret, &self.options)?;
        atomic_write(&path, yaml.as_bytes()).await?;
        let restore = if self.runtime_state["helperMode"] == true {
            self.reload_helper(&yaml)
        } else {
            self.controller
                .request("PUT", "/configs?force=true", Some(&json!({"path":path})))
                .await
                .map(|_| ())
        };
        if let Err(error) = restore {
            self.runtime_state["status"] = "failed".into();
            self.runtime_state["error"] = format!("configuration rollback failed: {error:#}").into();
            self.publish_state("");
            return Err(error);
        }
        Ok(())
    }

    async fn start_core(&mut self, id: &str, cancel: &Cancellation) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        if self.runtime_state["helperMode"] == true {
            self.stop_helper().await?;
        }
        if self.controller.child.is_some() {
            self.controller.stop().await?;
        }
        let binary = self.active_binary()?;
        let port = self.store.settings.mixed_port;
        let probe = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .context("owned mixedPort is already occupied")?;
        let udp_probe = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .context("owned mixedPort UDP is already occupied")?;
        drop(probe);
        drop(udp_probe);
        let path = self.options.data_dir.join("runtime.yaml");
        let yaml = self
            .store
            .owned_runtime_yaml(&self.controller.socket, &self.secret, &self.options)?;
        atomic_write(&path, yaml.as_bytes()).await?;
        cancel
            .run(Controller::validate(
                &binary,
                &path,
                &self.options.data_dir,
                self.controller.integration_test,
            ))
            .await?;
        cancel.check()?;
        self.runtime_state["coreGeneration"] = self.generation().checked_add(1).context("generation exhausted")?.into();
        self.runtime_state["status"] = "starting".into();
        self.publish_state(id);
        let result = cancel
            .run(self.controller.start(
                &binary,
                &path,
                &self.options.data_dir,
                &self.options.data_dir.join("core.log"),
            ))
            .await;
        match result {
            Ok(pid) => {
                self.runtime_state["status"] = "running".into();
                self.runtime_state["pid"] = pid.into();
                self.runtime_state
                    .as_object_mut()
                    .context("invalid runtime state")?
                    .remove("error");
                self.publish_state(id);
                self.restore_selections(id).await;
                Ok(())
            }
            Err(error) => {
                let _ = self.controller.stop().await;
                let _ = self.network.restore();
                self.runtime_state["status"] = "failed".into();
                self.runtime_state["pid"] = Value::Null;
                self.runtime_state["error"] = format!("{error:#}").into();
                self.publish_state(id);
                Err(error)
            }
        }
    }

    async fn stop_core(&mut self, id: &str) -> Result<()> {
        let network_restore = self.network.restore();
        let outcome = self.controller.stop().await;
        if outcome.is_ok() {
            self.stop_helper().await?;
        }
        self.runtime_state["coreGeneration"] = self.generation().checked_add(1).context("generation exhausted")?.into();
        self.runtime_state["status"] = if outcome.is_ok() { "stopped" } else { "failed" }.into();
        if let Err(error) = &network_restore {
            self.runtime_state["error"] = format!("network recovery required: {error:#}").into();
        }
        self.runtime_state["pid"] = Value::Null;
        self.runtime_state["socketPath"] = self.controller.socket.to_string_lossy().to_string().into();
        self.publish_state(id);
        outcome?;
        network_restore
    }

    async fn check_core_health(&mut self) {
        let previous_network = self.network.state();
        let refresh = self.network.refresh();
        if refresh.is_err() || previous_network != self.network.state() {
            self.publish_state("");
        }
        if self.runtime_state["helperMode"] == true {
            match helper::request(&helper::HelperCommand::Status, false) {
                Ok(status) if status.running && status.owner.as_deref() == Some(self.network.owner()) => return,
                outcome => {
                    self.runtime_state["status"] = "failed".into();
                    self.runtime_state["pid"] = Value::Null;
                    self.runtime_state["error"] = format!("owned TUN session unavailable: {outcome:?}").into();
                    let _ = self.network.restore();
                    self.publish_state("");
                    return;
                }
            }
        }
        let exited = self
            .controller
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten();
        if let Some(status) = exited {
            let _ = self.controller.stop().await;
            let recovery = self.network.restore();
            self.runtime_state["coreGeneration"] = self.generation().saturating_add(1).into();
            self.runtime_state["status"] = "failed".into();
            self.runtime_state["pid"] = Value::Null;
            self.runtime_state["error"] =
                format!("owned Mihomo exited: {status}; network recovery: {recovery:?}").into();
            self.publish_state("");
        }
    }

    async fn restore_selections(&mut self, id: &str) {
        let selections = self
            .store
            .selected_profile_id
            .as_deref()
            .and_then(|id| self.store.profile(id).ok())
            .map(|profile| profile.selected_nodes.clone())
            .unwrap_or_default();
        for (group, node) in selections {
            if let Err(error) = self
                .controller
                .request(
                    "PUT",
                    &format!("/proxies/{}", percent_encode(&group)),
                    Some(&json!({"name":node})),
                )
                .await
            {
                self.shared.publish(
                    id,
                    "state",
                    Err(anyhow::anyhow!(
                        "configuration committed; could not restore group {group}: {error:#}"
                    )),
                );
            }
        }
    }

    async fn core_request(&mut self, args: &Value, id: &str, cancel: &Cancellation) -> Result<Value> {
        if !self.is_running() {
            bail!("owned core is stopped");
        }
        let method = string(args, "method")?;
        let path = string(args, "path")?;
        if !public_request_allowed(method, path) {
            bail!("unsupported controller operation; use owned configuration commands");
        }
        let body = args.get("body");
        if method == "GET" {
            return cancel.run(self.controller.request(method, path, body)).await;
        }
        cancel.check()?;
        // A successful controller mutation is authoritative even if local
        // persistence later fails. Report the error without pretending rollback.
        let result = self.controller.request(method, path, body).await?;
        if method == "PUT"
            && path.starts_with("/proxies/")
            && !path.contains('?')
            && let Some(node) = body.and_then(|body| body.get("name")).and_then(Value::as_str)
            && let Some(profile_id) = self.store.selected_profile_id.clone()
        {
            let group = decode_percent(&path["/proxies/".len()..])?;
            let mut candidate = self.store.clone();
            candidate
                .profile_mut(&profile_id)?
                .selected_nodes
                .insert(group, node.into());
            candidate.revision = candidate.revision.checked_add(1).context("revision exhausted")?;
            let persisted = atomic_write(
                &self.options.data_dir.join("state.json"),
                &serde_json::to_vec_pretty(&candidate)?,
            )
            .await;
            self.store = candidate;
            self.publish_state(id);
            persisted.context("proxy selection applied; saving selection failed")?;
        }
        Ok(result)
    }

    async fn backup_create(&mut self) -> Result<Value> {
        let directory = self.options.data_dir.join("backups");
        tokio::fs::create_dir_all(&directory).await?;
        let id = format!("{}.json", unique_id());
        atomic_write(&directory.join(&id), &serde_json::to_vec_pretty(&self.store)?).await?;
        Ok(json!({"id":id,"createdAt":now(),"revision":self.store.revision}))
    }
    async fn backup_list(&mut self) -> Result<Value> {
        let directory = self.options.data_dir.join("backups");
        tokio::fs::create_dir_all(&directory).await?;
        let mut entries = tokio::fs::read_dir(directory).await?;
        let mut backups = vec![];
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if safe_backup_id(&name).is_ok() && entry.file_type().await?.is_file() {
                let metadata = entry.metadata().await?;
                backups.push(json!({"id":name,"bytes":metadata.len(),"createdAt":metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs()}));
            }
        }
        backups.sort_by(|a, b| b["createdAt"].as_u64().cmp(&a["createdAt"].as_u64()));
        Ok(json!(backups))
    }
}

async fn fetch_profile(url: &str, cancel: &Cancellation) -> Result<(String, Value)> {
    let url = reqwest::Url::parse(url)?;
    if !["https", "http"].contains(&url.scheme()) || !url.username().is_empty() || url.password().is_some() {
        bail!("subscription URL must be HTTP(S) without embedded credentials");
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Clash-Verge-Native/0.1")
        .build()?;
    cancel
        .run(async {
            let mut response = client.get(url).send().await?.error_for_status()?;
            let mut metadata = json!({});
            for (key, value) in response.headers() {
                if key
                    .as_str()
                    .strip_suffix("subscription-userinfo")
                    .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('-'))
                {
                    let mut info = BTreeMap::<String, u64>::new();
                    for pair in value.to_str()?.split(';') {
                        if let Some((name, number)) = pair.trim().split_once('=')
                            && ["upload", "download", "total", "expire"].contains(&name)
                            && let Ok(number) = number.trim().parse::<u64>()
                        {
                            info.insert(name.to_string(), number);
                        }
                    }
                    metadata["subscriptionInfo"] = serde_json::to_value(info)?;
                }
            }
            if let Some(interval) = response
                .headers()
                .get("profile-update-interval")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .and_then(|v| v.checked_mul(60))
                .filter(|v| subscriptions::validate_interval(Some(*v)).is_ok())
            {
                metadata["updateIntervalMinutes"] = interval.into();
            }
            if let Some(home) = response
                .headers()
                .get("profile-web-page-url")
                .and_then(|v| v.to_str().ok())
                && let Ok(url) = reqwest::Url::parse(home)
                && ["https", "http"].contains(&url.scheme())
            {
                metadata["home"] = url.as_str().into();
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > MAX_DOCUMENT_BYTES {
                    bail!("subscription exceeds 16 MiB limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            let text = String::from_utf8(bytes)?;
            let parsed = parse_profile_yaml(&text)?;
            if parsed.get("proxies").is_none() && parsed.get("proxy-providers").is_none() {
                bail!("remote profile must contain proxies or proxy-providers");
            }
            Ok((text.trim_start_matches('\u{feff}').to_string(), metadata))
        })
        .await
}

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("{name} must be a string"))
}
fn safe_backup_id(id: &str) -> Result<&str> {
    if !id.ends_with(".json")
        || id.len() > 100
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.".contains(&byte))
        || id.contains("..")
    {
        bail!("invalid backup ID");
    }
    Ok(id)
}
fn decode_percent(text: &str) -> Result<String> {
    let mut result = Vec::new();
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let a = bytes.next().context("invalid percent encoding")?;
            let b = bytes.next().context("invalid percent encoding")?;
            let pair = [a, b];
            result.push(u8::from_str_radix(std::str::from_utf8(&pair)?, 16)?);
        } else {
            result.push(byte);
        }
    }
    String::from_utf8(result).context("invalid UTF-8 group")
}

#[cfg(test)]
mod tests;

async fn background_read(
    controller: &Controller,
    shared: &Arc<Shared>,
    generation: u64,
    revision: u64,
    port: u16,
    request: &Request,
    cancel: &Cancellation,
) -> Result<Value> {
    let args = &request.args;
    match request.command.as_str() {
        "proxy.delay" => {
            let names: Vec<String> = serde_json::from_value(args.get("names").context("names required")?.clone())?;
            if names.is_empty() || names.len() > 4096 {
                bail!("delay batch must contain 1 to 4096 nodes");
            }
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://www.gstatic.com/generate_204");
            let parsed = reqwest::Url::parse(url)?;
            if !["http", "https"].contains(&parsed.scheme()) {
                bail!("delay URL must use HTTP(S)");
            }
            let mut seen = std::collections::HashSet::new();
            let mut tasks = futures_util::stream::iter(names.into_iter().filter(|name| seen.insert(name.clone())).map(
                |name| async move {
                    let path = format!(
                        "/proxies/{}/delay?timeout=5000&url={}",
                        percent_encode(&name),
                        percent_encode(url)
                    );
                    let value = controller.request("GET", &path, None).await;
                    match value {
                        Ok(value) => json!({"name":name,"delay":value["delay"],"url":url,"testedAt":now(),"requestId":request.id}),
                        Err(error) => {
                            json!({"name":name,"delay":null,"url":url,"testedAt":now(),"requestId":request.id,"error":format!("{error:#}")})
                        }
                    }
                },
            ))
            .buffer_unordered(8);
            cancel
                .run(async {
                    let mut results = Vec::new();
                    while let Some(result) = tasks.next().await {
                        shared.stream_at_revision(generation, revision, "delay", Ok(result.clone()));
                        results.push(result);
                    }
                    Ok(json!(results))
                })
                .await
        }
        "media.check" => {
            let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{}", port))?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .proxy(proxy)
                .timeout(Duration::from_secs(15))
                .build()?;
            let service = string(args, "service")?;
            let result = cancel
                .run(async {
                    clash_verge_media_unlock::check_media_unlock_item(&client, service)
                        .await
                        .map_err(anyhow::Error::msg)
                })
                .await?;
            Ok(serde_json::to_value(result)?)
        }
        "media.checkAll" => {
            let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{}", port))?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .proxy(proxy)
                .timeout(Duration::from_secs(15))
                .build()?;
            let results = cancel
                .run(async {
                    let results = clash_verge_media_unlock::check_media_unlock(&client, |item| {
                        if let Ok(value) = serde_json::to_value(item) {
                            shared.stream_at_revision(generation, revision, "media", Ok(value));
                        }
                    })
                    .await;
                    Ok(results)
                })
                .await?;
            Ok(serde_json::to_value(results)?)
        }
        _ => bail!("unsupported background operation"),
    }
}
