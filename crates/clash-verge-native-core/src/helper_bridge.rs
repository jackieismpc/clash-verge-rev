use super::*;
use std::{
    os::{fd::AsRawFd, unix::net::UnixListener},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub struct ControllerBridge {
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ControllerBridge {
    pub fn bind(frontend: &Path, engine: PathBuf, uid: u32, gid: u32) -> Result<Self> {
        let listener = UnixListener::bind(frontend)?;
        chown(frontend, uid, gid)?;
        fs::set_permissions(frontend, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let connections = Arc::new(AtomicUsize::new(0));
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if connections.load(Ordering::Relaxed) >= 32 {
                            continue;
                        }
                        connections.fetch_add(1, Ordering::Relaxed);
                        let connections = connections.clone();
                        let engine = engine.clone();
                        std::thread::spawn(move || {
                            let _ = serve_request(stream, &engine, uid);
                            connections.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stopping,
            thread: Some(thread),
        })
    }
}
impl Drop for ControllerBridge {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn serve_request(mut stream: UnixStream, engine: &Path, uid: u32) -> Result<()> {
    let mut peer = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer, &mut gid) } != 0 || peer != uid {
        bail!("controller peer UID mismatch");
    }
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut header = Vec::new();
    // Read exactly the headers: never forward a pipelined second HTTP request.
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 32 * 1024 {
            bail!("controller header too large");
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let text = std::str::from_utf8(&header)?;
    let mut lines = text.split("\r\n");
    let request = lines.next().ok_or_else(|| anyhow::anyhow!("empty HTTP request"))?;
    let words: Vec<_> = request.split(' ').collect();
    if words.len() != 3 || words[2] != "HTTP/1.1" {
        bail!("invalid HTTP request line");
    }
    let mut length = None;
    let mut websocket = false;
    let mut output = format!("{request}\r\n");
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid HTTP header"))?;
        if name.is_empty() || name.bytes().any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-') {
            bail!("invalid HTTP header name");
        }
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            bail!("privileged controller rejects chunked request bodies");
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                bail!("duplicate content length");
            }
            length = Some(value.parse::<usize>()?);
        }
        if name.eq_ignore_ascii_case("upgrade") {
            if !value.eq_ignore_ascii_case("websocket") {
                bail!("unsupported upgrade");
            }
            websocket = true;
        }
        if !name.eq_ignore_ascii_case("connection") {
            output.push_str(line);
            output.push_str("\r\n");
        }
    }
    let length = length.unwrap_or(0);
    if length > 64 * 1024 {
        bail!("privileged controller body too large");
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    super::super::helper_controller::allowed_request(words[0], words[1], &body, websocket)?;
    output.push_str(if websocket {
        "Connection: Upgrade\r\n\r\n"
    } else {
        "Connection: close\r\n\r\n"
    });
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut upstream = UnixStream::connect(engine)?;
    upstream.set_read_timeout(Some(Duration::from_secs(60)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(5)))?;
    upstream.write_all(output.as_bytes())?;
    upstream.write_all(&body)?;
    if websocket {
        upstream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            if response.len() >= 32 * 1024 {
                bail!("controller upgrade response too large");
            }
            let mut byte = [0];
            upstream.read_exact(&mut byte)?;
            response.push(byte[0]);
        }
        stream.write_all(&response)?;
        let status = std::str::from_utf8(&response)?
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default();
        if status != "101" {
            let _ = upstream.shutdown(std::net::Shutdown::Both);
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return Ok(());
        }
        upstream.set_read_timeout(None)?;
        stream.set_read_timeout(None)?;
        let mut input = stream.try_clone()?;
        let mut engine_writer = upstream.try_clone()?;
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut input, &mut engine_writer);
            let _ = engine_writer.shutdown(std::net::Shutdown::Write);
        });
    }
    let _ = std::io::copy(&mut upstream, &mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}
