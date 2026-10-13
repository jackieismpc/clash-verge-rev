use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
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
        })
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('#') || path.contains("..") {
            bail!("invalid controller path");
        }
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

    pub async fn validate(binary: &Path, config: &Path, home: &Path) -> Result<()> {
        let output = tokio::time::timeout(
            Duration::from_secs(20),
            Command::new(binary)
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
        let output = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
        self.child = Some(
            Command::new(binary)
                .arg("-d")
                .arg(home)
                .arg("-f")
                .arg(config)
                .stdin(Stdio::null())
                .stdout(output.try_clone()?)
                .stderr(output)
                .kill_on_drop(true)
                .spawn()?,
        );
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
        if let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
            // Retain process ownership until termination is confirmed.
            self.child = None;
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
        }
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
                let outcome = async {
                    let stream = UnixStream::connect(socket).await?;
                    let mut request = format!("ws://localhost/{name}{query}").into_client_request()?;
                    request
                        .headers_mut()
                        .insert("authorization", HeaderValue::from_str(&format!("Bearer {secret}"))?);
                    let (mut ws, _) = client_async(request, stream).await?;
                    while let Some(message) = ws.next().await {
                        match message? {
                            Message::Text(text) => publish(generation, &name, Ok(serde_json::from_str(&text)?)),
                            Message::Binary(bytes) => publish(generation, &name, Ok(serde_json::from_slice(&bytes)?)),
                            Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await?,
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if let Err(error) = outcome {
                    publish(generation, &name, Err(error));
                }
            }));
        }
        Ok(())
    }
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
