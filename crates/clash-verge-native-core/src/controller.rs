use crate::Cancellation;
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::UnixStream,
    process::{Child, Command},
    task::JoinHandle,
};
use tokio_tungstenite::{
    client_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

pub struct Controller {
    pub socket: PathBuf,
    secret: String,
    client: reqwest::Client,
    pub child: Option<Child>,
    pub streams: Vec<JoinHandle<()>>,
    retired_streams: Vec<JoinHandle<()>>,
    log_tasks: Vec<JoinHandle<()>>,
    pub(crate) read_tasks: Vec<(Arc<Cancellation>, JoinHandle<()>)>,
    delay_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) integration_test: bool,
}

impl Controller {
    pub fn new(socket: PathBuf, secret: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .unix_socket(socket.clone())
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            socket,
            secret,
            client,
            child: None,
            streams: vec![],
            retired_streams: vec![],
            log_tasks: vec![],
            read_tasks: vec![],
            delay_slots: Arc::new(tokio::sync::Semaphore::new(8)),
            integration_test: false,
        })
    }

    pub fn reader(&self) -> Self {
        Self {
            socket: self.socket.clone(),
            secret: self.secret.clone(),
            client: self.client.clone(),
            child: None,
            streams: vec![],
            retired_streams: vec![],
            log_tasks: vec![],
            read_tasks: vec![],
            delay_slots: Arc::clone(&self.delay_slots),
            integration_test: self.integration_test,
        }
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('#') || path.contains("..") {
            bail!("invalid controller path");
        }
        let _permit = if path.contains("/delay?") {
            Some(self.delay_slots.acquire().await.context("delay scheduler stopped")?)
        } else {
            None
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut request = self
            .client
            .request(method, format!("http://localhost{path}"))
            .bearer_auth(&self.secret);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("Mihomo HTTP {}: {}", status.as_u16(), text);
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).context("Mihomo returned invalid JSON")
    }

    pub async fn validate(binary: &Path, config: &Path, home: &Path, integration_test: bool) -> Result<()> {
        let output = tokio::time::timeout(
            Duration::from_secs(20),
            owned_command(binary, home, integration_test)
                .arg("-t")
                .arg("-d")
                .arg(home)
                .arg("-f")
                .arg(config)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("Mihomo validation timed out")??;
        if !output.status.success() {
            bail!(
                "Mihomo rejected configuration: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    pub async fn start(&mut self, binary: &Path, config: &Path, home: &Path, log: &Path) -> Result<u32> {
        if self.child.is_some() {
            bail!("owned core is already running");
        }
        if tokio::fs::try_exists(&self.socket).await? {
            bail!("owned controller path is already occupied");
        }
        let writer = Arc::new(std::sync::Mutex::new(Some(crate::logs::RotatingLog::open(
            log,
            2 * 1024 * 1024,
            3,
        )?)));
        let mut child = owned_command(binary, home, self.integration_test)
            .arg("-d")
            .arg(home)
            .arg("-f")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(stdout) = child.stdout.take() {
            self.log_tasks.push(capture_log(stdout, Arc::clone(&writer)));
        }
        if let Some(stderr) = child.stderr.take() {
            self.log_tasks.push(capture_log(stderr, writer));
        }
        self.child = Some(child);
        let pid = self
            .child
            .as_ref()
            .and_then(Child::id)
            .context("missing core process ID")?;
        let ready = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(status) = self.child.as_mut().context("core disappeared")?.try_wait()? {
                    bail!("Mihomo exited before readiness ({status}); inspect core.log");
                }
                if self.request("GET", "/version", None).await.is_ok() {
                    return Ok(pid);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        match ready {
            Ok(Ok(pid)) => Ok(pid),
            outcome => {
                self.stop().await?;
                match outcome {
                    Ok(Err(error)) => Err(error),
                    _ => bail!("Mihomo controller did not become ready within 15 seconds"),
                }
            }
        }
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.unsubscribe();
        for task in self.retired_streams.drain(..) {
            let _ = task.await;
        }
        for (cancel, _) in &self.read_tasks {
            cancel.cancel();
        }
        for (_, task) in self.read_tasks.drain(..) {
            let _ = task.await;
        }
        if let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
            // Retain process ownership until termination is confirmed.
            self.child = None;
            for mut task in self.log_tasks.drain(..) {
                if tokio::time::timeout(Duration::from_secs(2), &mut task).await.is_err() {
                    task.abort();
                    let _ = task.await;
                }
            }
            if let Err(error) = tokio::fs::remove_file(&self.socket).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn unsubscribe(&mut self) {
        for task in self.streams.drain(..) {
            task.abort();
            self.retired_streams.push(task);
        }
        self.retired_streams.retain(|task| !task.is_finished());
    }

    pub fn subscribe<F>(&mut self, names: &[String], generation: u64, publish: F) -> Result<()>
    where
        F: Fn(u64, &str, Result<Value>) + Send + Sync + Clone + 'static,
    {
        for name in names {
            if !["traffic", "memory", "logs", "connections"].contains(&name.as_str()) {
                bail!("unknown stream {name}");
            }
        }
        self.unsubscribe();
        for name in names {
            let socket = self.socket.clone();
            let secret = self.secret.clone();
            let publish = publish.clone();
            let name = name.clone();
            self.streams.push(tokio::spawn(async move {
                let query = if name == "logs" {
                    "?level=debug"
                } else {
                    "?interval=1000"
                };
                let mut backoff = Duration::from_millis(250);
                loop {
                    let outcome: Result<()> = async {
                        let stream = tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(&socket))
                            .await
                            .context("Stream connect timed out")??;
                        let mut request = format!("ws://localhost/{name}{query}").into_client_request()?;
                        request
                            .headers_mut()
                            .insert("authorization", HeaderValue::from_str(&format!("Bearer {secret}"))?);
                        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), client_async(request, stream))
                            .await
                            .context("Stream handshake timed out")??;
                        publish(generation, "streamStatus", Ok(json!({"stream":name,"connected":true})));
                        loop {
                            let message = tokio::time::timeout(Duration::from_secs(45), ws.next())
                                .await
                                .context("Stream idle timeout")?
                                .context("Stream disconnected")??;
                            match message {
                                Message::Text(text) => publish(generation, &name, Ok(serde_json::from_str(&text)?)),
                                Message::Binary(bytes) => {
                                    publish(generation, &name, Ok(serde_json::from_slice(&bytes)?))
                                }
                                Message::Ping(bytes) => {
                                    tokio::time::timeout(Duration::from_secs(5), ws.send(Message::Pong(bytes)))
                                        .await
                                        .context("Stream pong timed out")??;
                                }
                                Message::Close(_) => bail!("Stream closed"),
                                _ => {}
                            }
                        }
                    }
                    .await;
                    if let Err(error) = outcome {
                        publish(generation, &name, Err(error));
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(8));
                }
            }));
        }
        Ok(())
    }
}

fn capture_log<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    mut pipe: R,
    writer: Arc<std::sync::Mutex<Option<crate::logs::RotatingLog>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buffer = [0u8; 8192];
        while let Ok(count) = pipe.read(&mut buffer).await {
            if count == 0 {
                break;
            }
            // Drain even after a filesystem failure so the child cannot block on its pipe.
            if let Ok(mut log) = writer.lock()
                && log.as_mut().is_some_and(|log| log.write(&buffer[..count]).is_err())
            {
                *log = None;
            }
        }
    })
}

fn owned_command(binary: &Path, home: &Path, integration_test: bool) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(home);
    if integration_test {
        command.env_clear().env("HOME", home).env("TMPDIR", home.join("tmp"));
        // Only the forked child changes its mask; this closure performs one syscall.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o077);
                Ok(())
            });
        }
    }
    command
}

pub fn public_request_allowed(method: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    match method {
        "GET" => [
            "/version",
            "/configs",
            "/proxies",
            "/providers/proxies",
            "/rules",
            "/connections",
            "/providers/rules",
            "/dns/query",
        ]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/"))),
        "PUT" => {
            path.starts_with("/proxies/")
                || path.starts_with("/providers/proxies/")
                || path.starts_with("/providers/rules/")
        }
        "DELETE" => path == "/connections" || path.starts_with("/connections/"),
        _ => false,
    }
}

pub fn percent_encode(text: &str) -> String {
    text.as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(byte) {
                char::from(*byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub fn stopped(owner: &str, generation: u64, socket: &Path, port: u16) -> Value {
    json!({"status":"stopped","ownerId":owner,"coreGeneration":generation,
        "socketPath":socket,"mixedPort":port,"pid":null})
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    #[tokio::test]
    async fn unix_stream_reconnects_after_close_and_stop_reaps_tasks() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.work/stream-tests");
        tokio::fs::create_dir_all(&root).await?;
        let root = root.canonicalize()?;
        let socket = root.join(format!(
            "{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let listener = tokio::net::UnixListener::bind(&socket)?;
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await?;
                let mut ws = tokio_tungstenite::accept_async(stream).await?;
                ws.send(Message::Text("{\"up\":1,\"down\":2}".into())).await?;
                ws.close(None).await?;
            }
            Ok::<(), anyhow::Error>(())
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut controller = Controller::new(socket.clone(), "fixture-secret".into())?;
        controller.subscribe(&["traffic".into()], 7, move |generation, name, result| {
            if name == "streamStatus" && result.is_ok() {
                let _ = tx.send(generation);
            }
        })?;
        for _ in 0..2 {
            assert_eq!(tokio::time::timeout(Duration::from_secs(3), rx.recv()).await?, Some(7));
        }
        controller.stop().await?;
        assert!(controller.streams.is_empty());
        assert!(controller.retired_streams.is_empty());
        assert!(controller.log_tasks.is_empty());
        tokio::time::timeout(Duration::from_secs(3), server).await???;
        tokio::fs::remove_file(socket).await?;
        Ok(())
    }
}
