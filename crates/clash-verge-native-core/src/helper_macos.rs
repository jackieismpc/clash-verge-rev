use super::*;
use std::ffi::{CString, c_void};
use std::{
    fs::{self, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixListener,
        },
    },
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFEqual(a: Ref, b: Ref) -> u8;
    fn CFURLCreateFromFileSystemRepresentation(allocator: Ref, path: *const u8, length: isize, is_directory: u8)
    -> Ref;
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecStaticCodeCreateWithPath(path: Ref, flags: u32, output: *mut Ref) -> i32;
    fn SecStaticCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
    fn SecCodeCopySigningInformation(code: Ref, flags: u32, output: *mut Ref) -> i32;
    fn SecCodeCopyGuestWithAttributes(host: Ref, attributes: Ref, flags: u32, output: *mut Ref) -> i32;
    fn SecCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
    static kSecGuestAttributeAudit: Ref;
    static kSecCodeInfoUnique: Ref;
    static kSecCodeInfoIdentifier: Ref;
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}
fn checked(status: i32, value: Ref) -> Result<Owned> {
    if status != 0 || value.is_null() {
        bail!("signature verification failed ({status})");
    }
    Ok(Owned(value))
}
const STATIC_SIGNATURE_FLAGS: u32 = (1 << 0) | (1 << 3) | (1 << 4);
const HELPER_LOG_MAXIMUM: u64 = 2 * 1024 * 1024;
const HELPER_LOG_ARCHIVES: usize = 3;
fn trusted_path_metadata(path: &Path, directory: bool, metadata: PathMetadata) -> bool {
    let applications_container =
        directory && path == Path::new("/Applications") && metadata.gid == 80 && metadata.mode & 0o7777 == 0o775;
    metadata.uid == 0
        && !metadata.symlink
        && if directory { metadata.directory } else { metadata.file }
        && (metadata.mode & 0o022 == 0 || applications_container)
}
struct PathMetadata {
    uid: u32,
    gid: u32,
    mode: u32,
    directory: bool,
    file: bool,
    symlink: bool,
}
fn secure_existing(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !trusted_path_metadata(
        path,
        directory,
        PathMetadata {
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            directory: metadata.is_dir(),
            file: metadata.is_file(),
            symlink: metadata.file_type().is_symlink(),
        },
    ) {
        bail!("helper requires root-owned non-writable path: {}", path.display());
    }
    Ok(())
}
fn static_code(path: &Path) -> Result<Owned> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let url = Owned(unsafe {
        CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            path.is_dir() as u8,
        )
    });
    if url.0.is_null() {
        bail!("cannot form bundle URL");
    }
    let mut raw = std::ptr::null();
    let status = unsafe { SecStaticCodeCreateWithPath(url.0, 0, &mut raw) };
    let code = checked(status, raw)?;
    let status = unsafe { SecStaticCodeCheckValidity(code.0, STATIC_SIGNATURE_FLAGS, std::ptr::null()) };
    if status != 0 {
        bail!("installed native bundle signature is invalid ({status})");
    }
    Ok(code)
}
fn signing_info(code: Ref) -> Result<Owned> {
    let mut raw = std::ptr::null();
    let status = unsafe { SecCodeCopySigningInformation(code, 2, &mut raw) };
    checked(status, raw)
}
fn guest_for_token(token: &[u32; 8]) -> Result<Owned> {
    let data = Owned(unsafe {
        CFDataCreate(
            std::ptr::null(),
            token.as_ptr().cast(),
            std::mem::size_of_val(token) as isize,
        )
    });
    if data.0.is_null() {
        bail!("cannot allocate peer token");
    }
    let key = unsafe { kSecGuestAttributeAudit };
    let value = data.0;
    let attributes = Owned(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &value,
            1,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks).cast(),
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks).cast(),
        )
    });
    let mut raw = std::ptr::null();
    let status = unsafe { SecCodeCopyGuestWithAttributes(std::ptr::null(), attributes.0, 0, &mut raw) };
    checked(status, raw)
}
struct Identity {
    main: PathBuf,
    main_info: Owned,
    core: PathBuf,
}
impl Identity {
    fn installed() -> Result<Self> {
        let executable = std::env::current_exe()?;
        // No CLI paths. SMAppService launches this exact bundle-relative helper.
        let helpers = executable
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing helper directory"))?;
        let contents = helpers
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing Contents directory"))?;
        let app = contents
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing application bundle"))?;
        if helpers.file_name().and_then(|s| s.to_str()) != Some("Helpers")
            || contents.file_name().and_then(|s| s.to_str()) != Some("Contents")
            || app != Path::new("/Applications/Clash Native.app")
        {
            bail!(
                "install the signed root-owned application at /Applications/Clash Native.app before approving its helper"
            );
        }
        for ancestor in executable.ancestors().skip(1) {
            secure_existing(ancestor, true)?;
        }
        secure_existing(&executable, false)?;
        static_code(app)?;
        let main = contents.join("MacOS/ClashNative");
        let core = contents.join("Resources/mihomo");
        secure_existing(&main, false)?;
        secure_existing(&core, false)?;
        for path in [&main, &core] {
            for ancestor in path.ancestors().skip(1) {
                secure_existing(ancestor, true)?;
            }
        }
        let main_code = static_code(&main)?;
        let main_info = signing_info(main_code.0)?;
        static_code(&core)?;
        Ok(Self { main, main_info, core })
    }
    fn authenticate(&self, stream: &UnixStream) -> Result<(u32, u32, [u32; 8])> {
        let fd = stream.as_raw_fd();
        let mut uid = 0;
        let mut gid = 0;
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 || uid == 0 {
            bail!("helper requires an authenticated non-root app user");
        }
        let mut token = [0_u32; 8];
        let mut length = std::mem::size_of_val(&token) as libc::socklen_t;
        if unsafe { libc::getsockopt(fd, 0, 6, token.as_mut_ptr().cast(), &mut length) } != 0
            || length as usize != std::mem::size_of_val(&token)
        {
            bail!("cannot read helper peer audit token");
        }
        // Darwin audit_token_t euid index 1, pid index 5; token identifies the process incarnation.
        if token[1] != uid {
            bail!("peer credential mismatch");
        }
        let code = guest_for_token(&token)?;
        let status = unsafe { SecCodeCheckValidity(code.0, 0, std::ptr::null()) };
        if status != 0 {
            bail!("helper peer signature invalid ({status})");
        }
        let info = signing_info(code.0)?;
        for key in unsafe { [kSecCodeInfoUnique, kSecCodeInfoIdentifier] } {
            let expected = unsafe { CFDictionaryGetValue(self.main_info.0, key) };
            let actual = unsafe { CFDictionaryGetValue(info.0, key) };
            if expected.is_null() || actual.is_null() || unsafe { CFEqual(expected, actual) } == 0 {
                bail!("helper peer is not the approved installed native application");
            }
        }
        Ok((uid, gid, token))
    }
    fn revalidate(&self) -> Result<()> {
        secure_existing(&self.core, false)?;
        secure_existing(&self.main, false)?;
        let app = self
            .main
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(|| anyhow::anyhow!("invalid app path"))?;
        static_code(app)?;
        Ok(())
    }
}
#[path = "helper_bridge.rs"]
mod bridge;

struct Session {
    uid: u32,
    owner: String,
    generation: u64,
    gui_token: [u32; 8],
    child: Child,
    socket: PathBuf,
    bridge: Option<bridge::ControllerBridge>,
    directory: PathBuf,
    mixed_port: u16,
    secret: String,
    active_config: PathBuf,
    log_tasks: Vec<std::thread::JoinHandle<()>>,
}
impl Session {
    fn finish_logs(&mut self) {
        for task in self.log_tasks.drain(..) {
            let _ = task.join();
        }
    }
    fn status(&self) -> HelperStatus {
        HelperStatus {
            available: true,
            running: true,
            owned_uid: Some(self.uid),
            owner: Some(self.owner.clone()),
            generation: Some(self.generation),
            pid: Some(self.child.id()),
            private_socket: Some(self.socket.to_string_lossy().into()),
            message: "Independently approved native TUN core is running".into(),
        }
    }
    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            self.finish_logs();
            self.bridge.take();
            let _ = fs::remove_file(&self.socket);
            return Ok(());
        }
        // SIGTERM allows mihomo to remove its own routes and TUN interface.
        if unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                self.finish_logs();
                self.bridge.take();
                let _ = fs::remove_file(&self.socket);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Keep ownership and report failure; never SIGKILL an engine still responsible for route cleanup.
        bail!("owned TUN core has not exited; retry stop before unregistering the helper")
    }
}
fn empty_status() -> HelperStatus {
    HelperStatus {
        available: true,
        running: false,
        owned_uid: None,
        owner: None,
        generation: None,
        pid: None,
        private_socket: None,
        message: "Native helper is approved and ready".into(),
    }
}
fn create_directory(path: &Path, mode: u32) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(mode))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    secure_existing(path, true)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}
type HelperLog = std::sync::Arc<std::sync::Mutex<Option<crate::logs::RotatingLog>>>;
fn helper_log(directory: &Path) -> Result<HelperLog> {
    let path = directory.join("core.log");
    for name in std::iter::once(path.clone())
        .chain((1..=HELPER_LOG_ARCHIVES).map(|index| directory.join(format!("core.log.{index}"))))
    {
        match fs::symlink_metadata(&name) {
            Ok(_) => secure_existing(&name, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(std::sync::Arc::new(std::sync::Mutex::new(Some(
        crate::logs::RotatingLog::open(&path, HELPER_LOG_MAXIMUM, HELPER_LOG_ARCHIVES)?,
    ))))
}
fn capture_helper_log(mut reader: impl Read + Send + 'static, writer: HelperLog) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
                Ok(count) => {
                    if let Ok(mut log) = writer.lock()
                        && log.as_mut().is_some_and(|log| log.write(&buffer[..count]).is_err())
                    {
                        // Disable both writers after partial I/O failure; keep draining the bounded pipes.
                        *log = None;
                    }
                }
            }
        }
    })
}
fn private_write(path: &Path, content: &[u8]) -> Result<()> {
    if path.exists() {
        secure_existing(path, false)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}
fn chown(path: &Path, uid: u32, gid: u32) -> Result<()> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())?;
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn download_proxy_providers(runtime: &str, files: &mut BTreeMap<String, String>) -> Result<String> {
    let mut config: Value = serde_yaml_ng::from_str(runtime)?;
    let Some(providers) = config.get_mut("proxy-providers").and_then(Value::as_object_mut) else {
        return Ok(runtime.into());
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let download_deadline = Instant::now() + Duration::from_secs(20);
    for (name, provider) in providers {
        if provider.get("type").and_then(Value::as_str) != Some("http") {
            continue;
        }
        let url = provider
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("provider URL missing"))?
            .to_string();
        let url = reqwest::Url::parse(&url)?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("proxy provider URL must be HTTP(S)");
        }
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(values) = provider.get("header").and_then(Value::as_object) {
            for (name, value) in values {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())?;
                let values = match value {
                    Value::String(text) => vec![text.as_str()],
                    Value::Array(values) => values
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("provider header must be text"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                    _ => bail!("provider header must be text"),
                };
                for value in values {
                    headers.append(name.clone(), reqwest::header::HeaderValue::from_str(value)?);
                }
            }
        }
        let remaining = download_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("HTTP proxy provider preparation timed out");
        }
        let content = runtime.block_on(async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(remaining.min(Duration::from_secs(8)))
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()?;
            let mut response = client.get(url).headers(headers).send().await?.error_for_status()?;
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > 1024 * 1024 {
                    bail!("HTTP proxy provider exceeds limit");
                }
                body.extend_from_slice(&chunk);
            }
            String::from_utf8(body).map_err(anyhow::Error::from)
        })?;
        let parsed: Value = serde_yaml_ng::from_str(&content)?;
        for proxy in parsed
            .get("proxies")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("HTTP proxy provider requires proxies"))?
        {
            validate_proxy(proxy)?;
        }
        files.insert(format!("proxy-providers:{name}"), content);
        let provider = provider
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("invalid provider dictionary"))?;
        provider.insert("type".into(), json!("file"));
        provider.remove("url");
    }
    Ok(serde_yaml_ng::to_string(&config)?)
}

static STAGING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn validate_configuration(core: &Path, directory: &Path, config: &Path) -> Result<()> {
    let mut validation = Command::new(core)
        .arg("-t")
        .arg("-d")
        .arg(directory)
        .arg("-f")
        .arg(config)
        .env_clear()
        .env("HOME", directory)
        .env("PATH", "/usr/bin:/bin")
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if let Some(exit) = validation.try_wait()? {
            if !exit.success() {
                bail!("privileged candidate configuration failed validation ({exit})");
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            // This is the isolated -t validator, never the active routing engine.
            let _ = validation.kill();
            let _ = validation.wait();
            bail!("privileged configuration validation timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn reload_engine(socket: &Path, config: &Path, secret: &str) -> Result<()> {
    let body = serde_json::to_vec(&json!({"path":config}))?;
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "PUT /configs?force=true HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    let mut response = Vec::new();
    stream.take(64 * 1024).read_to_end(&mut response)?;
    let line = std::str::from_utf8(&response)?
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty privileged reload reply"))?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        bail!("privileged runtime reload rejected ({status})");
    }
    Ok(())
}
static STOPPING: AtomicBool = AtomicBool::new(false);
extern "C" fn terminate(_: i32) {
    STOPPING.store(true, Ordering::Relaxed);
}

fn cleanup_created_session<S>(
    session: &mut Option<S>,
    created_here: bool,
    mut stop: impl FnMut(&mut S) -> Result<()>,
) -> Result<()> {
    if !created_here {
        return Ok(());
    }
    if let Some(active) = session {
        stop(active)?;
    }
    *session = None;
    Ok(())
}
fn owned_staging_path(directory: &Path, candidate: &Path) -> bool {
    if candidate.parent() != Some(directory) {
        return false;
    }
    candidate
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("staging-"))
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(pid, sequence)| pid.parse::<u32>().is_ok() && sequence.parse::<u64>().is_ok())
}
fn staging_is_referenced(candidate: &Path, active_config: &Path, provider_paths: &[PathBuf]) -> bool {
    active_config.starts_with(candidate) || provider_paths.iter().any(|path| path.starts_with(candidate))
}
fn prune_inactive_staging(directory: &Path, active_config: &Path) -> Result<()> {
    secure_existing(directory, true)?;
    secure_existing(active_config, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(active_config)?;
    if file.metadata()?.len() > (MAX_HELPER_BYTES * 2) as u64 {
        bail!("active helper configuration exceeds cleanup inspection limit");
    }
    let mut text = String::new();
    file.take((MAX_HELPER_BYTES * 2 + 1) as u64).read_to_string(&mut text)?;
    let config: Value = serde_yaml_ng::from_str(&text)?;
    let mut providers = Vec::new();
    for key in ["proxy-providers", "rule-providers"] {
        if let Some(values) = config.get(key).and_then(Value::as_object) {
            for provider in values.values() {
                if let Some(path) = provider.get("path").and_then(Value::as_str) {
                    providers.push(PathBuf::from(path));
                }
            }
        }
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if !owned_staging_path(directory, &path) || staging_is_referenced(&path, active_config, &providers) {
            continue;
        }
        secure_existing(&path, true)?;
        fs::remove_dir_all(path)?;
    }
    Ok(())
}
/// Call only from the signed SMAppService launch daemon executable. Not used by integration tests.
pub fn serve() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("native helper must be launched by approved SMAppService as root");
    }
    if std::env::args_os().len() != 1 {
        bail!("native helper accepts no command or path arguments");
    }
    unsafe {
        libc::umask(0o077);
    }
    let identity = Identity::installed()?;
    create_directory(Path::new(HELPER_DATA), 0o711)?;
    let socket = Path::new(HELPER_SOCKET);
    // Acquire a separate root-owned lock before touching the service socket.
    let lock_path = Path::new(HELPER_DATA).join("daemon.lock");
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)?;
    secure_existing(&lock_path, false)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("independent native helper is already running");
    }
    if let Ok(metadata) = fs::symlink_metadata(socket) {
        use std::os::unix::fs::FileTypeExt;
        if metadata.uid() != 0 || !metadata.file_type().is_socket() {
            bail!("helper socket path is not owned by this service");
        }
        fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o666))?;
    listener.set_nonblocking(true)?;
    unsafe {
        libc::signal(libc::SIGTERM, terminate as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, terminate as *const () as libc::sighandler_t);
    }
    let mut session: Option<Session> = None;
    while !STOPPING.load(Ordering::Relaxed) {
        if let Some(active) = &mut session {
            if active.child.try_wait()?.is_some() {
                active.finish_logs();
                let _ = fs::remove_file(&active.socket);
                session = None;
            } else if guest_for_token(&active.gui_token).is_err() && active.stop().is_ok() {
                session = None;
            }
        }
        let (mut stream, _) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut started_this_request = false;
        let result = (|| {
            let (uid, gid, token) = identity.authenticate(&stream)?;
            let mut length = [0; 4];
            stream.read_exact(&mut length)?;
            let length = u32::from_be_bytes(length) as usize;
            if length > MAX_HELPER_BYTES {
                bail!("helper request exceeds limit");
            }
            let mut body = vec![0; length];
            stream.read_exact(&mut body)?;
            let command: HelperCommand = serde_json::from_slice(&body)?;
            match command {
                HelperCommand::Status => {
                    if let Some(active) = &session {
                        if active.uid != uid {
                            bail!("native helper is owned by another logged-in user");
                        }
                        return Ok(active.status());
                    }
                    Ok(empty_status())
                }
                HelperCommand::Reload {
                    owner,
                    generation,
                    runtime_yaml,
                    mut provider_files,
                } => {
                    let active = session
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("no owned TUN session"))?;
                    verify_owned_stop(&active.status(), uid, &owner, generation)?;
                    identity.revalidate()?;
                    let sequence = STAGING.fetch_add(1, Ordering::Relaxed);
                    let staging = active
                        .directory
                        .join(format!("staging-{}-{sequence}", std::process::id()));
                    create_directory(&staging, 0o700)?;
                    let mut engine_touched = false;
                    let previous_config = active.active_config.clone();
                    let result = (|| {
                        let runtime_yaml = download_proxy_providers(&runtime_yaml, &mut provider_files)?;
                        let mut prepared = prepare_config(
                            &runtime_yaml,
                            &provider_files,
                            &staging,
                            active.mixed_port,
                            &active.secret,
                        )?;
                        let mut configuration: Value = serde_yaml_ng::from_str(&prepared.yaml)?;
                        configuration
                            .as_object_mut()
                            .ok_or_else(|| anyhow::anyhow!("invalid prepared config"))?
                            .insert(
                                "external-controller-unix".into(),
                                json!(active.directory.join("privileged/engine.sock")),
                            );
                        prepared.yaml = serde_yaml_ng::to_string(&configuration)?;
                        for (path, content) in prepared.files {
                            private_write(&path, content.as_bytes())?;
                        }
                        let config_path = staging.join("config.yaml");
                        private_write(&config_path, prepared.yaml.as_bytes())?;
                        validate_configuration(&identity.core, &active.directory, &config_path)?;
                        engine_touched = true;
                        reload_engine(
                            &active.directory.join("privileged/engine.sock"),
                            &config_path,
                            &active.secret,
                        )?;
                        active.active_config = config_path;
                        let mut status = active.status();
                        if let Err(error) = prune_inactive_staging(&active.directory, &active.active_config) {
                            status
                                .message
                                .push_str(&format!("; inactive staging cleanup failed: {error}"));
                        }
                        Ok(status)
                    })();
                    if let Err(error) = result {
                        if engine_touched
                            && let Err(recovery) = reload_engine(
                                &active.directory.join("privileged/engine.sock"),
                                &previous_config,
                                &active.secret,
                            )
                        {
                            bail!(
                                "privileged reload failed: {error}; rollback failed: {recovery}; owned TUN session and staged data retained"
                            );
                        }
                        let _ = fs::remove_dir_all(&staging);
                        return Err(error);
                    }
                    result
                }
                HelperCommand::Stop { owner, generation } => {
                    let active = session
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("no owned TUN session"))?;
                    verify_owned_stop(&active.status(), uid, &owner, generation)?;
                    active.stop()?;
                    session = None;
                    Ok(empty_status())
                }
                HelperCommand::Start {
                    owner,
                    generation,
                    mixed_port,
                    secret,
                    runtime_yaml,
                    mut provider_files,
                } => {
                    if session.is_some() {
                        bail!("stop the existing owned TUN session before starting another");
                    }
                    if !owner.starts_with("native-mac:")
                        || owner.len() < 16
                        || owner.len() > 128
                        || !owner.bytes().all(|b| b.is_ascii_alphanumeric() || b"-:_".contains(&b))
                    {
                        bail!("invalid native owner identity");
                    }
                    identity.revalidate()?;
                    let directory = Path::new(HELPER_DATA).join(uid.to_string());
                    create_directory(&directory, 0o710)?;
                    chown(&directory, 0, gid)?;
                    create_directory(&directory.join("privileged"), 0o700)?;
                    let runtime_yaml = download_proxy_providers(&runtime_yaml, &mut provider_files)?;
                    let prepared = prepare_config(&runtime_yaml, &provider_files, &directory, mixed_port, &secret)?;
                    for (path, content) in prepared.files {
                        private_write(&path, content.as_bytes())?;
                    }
                    private_write(&directory.join("config.yaml"), prepared.yaml.as_bytes())?;
                    let private_socket = directory.join("core.sock");
                    if let Ok(metadata) = fs::symlink_metadata(&private_socket) {
                        use std::os::unix::fs::FileTypeExt;
                        if !metadata.file_type().is_socket() || metadata.uid() != uid {
                            bail!("stale TUN controller socket has invalid ownership");
                        }
                        fs::remove_file(&private_socket)?;
                    }
                    let engine_socket = directory.join("privileged/engine.sock");
                    if let Ok(metadata) = fs::symlink_metadata(&engine_socket) {
                        use std::os::unix::fs::FileTypeExt;
                        if !metadata.file_type().is_socket() || metadata.uid() != 0 {
                            bail!("privileged engine socket has invalid ownership");
                        }
                        fs::remove_file(&engine_socket)?;
                    }
                    validate_configuration(&identity.core, &directory, &directory.join("config.yaml"))?;
                    let log = helper_log(&directory)?;
                    let mut child = Command::new(&identity.core)
                        .args(["-d"])
                        .arg(&directory)
                        .arg("-f")
                        .arg(directory.join("config.yaml"))
                        .env_clear()
                        .env("HOME", &directory)
                        .env("PATH", "/usr/bin:/bin")
                        .current_dir(&directory)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let mut log_tasks = Vec::with_capacity(2);
                    if let Some(stdout) = child.stdout.take() {
                        log_tasks.push(capture_helper_log(stdout, std::sync::Arc::clone(&log)));
                    }
                    if let Some(stderr) = child.stderr.take() {
                        log_tasks.push(capture_helper_log(stderr, log));
                    }
                    session = Some(Session {
                        uid,
                        owner,
                        generation,
                        gui_token: token,
                        child,
                        socket: private_socket,
                        bridge: None,
                        directory: directory.clone(),
                        mixed_port,
                        secret: secret.clone(),
                        active_config: directory.join("config.yaml"),
                        log_tasks,
                    });
                    started_this_request = true;
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while Instant::now() < deadline {
                        let active = session.as_mut().ok_or_else(|| anyhow::anyhow!("TUN session lost"))?;
                        if let Some(exit) = active.child.try_wait()? {
                            active.finish_logs();
                            session = None;
                            bail!("owned TUN core failed to start ({exit})");
                        }
                        if engine_socket.exists() {
                            let metadata = fs::symlink_metadata(&engine_socket)?;
                            use std::os::unix::fs::FileTypeExt;
                            if metadata.uid() != 0 || !metadata.file_type().is_socket() {
                                bail!("privileged engine socket is not root-owned");
                            }
                            fs::set_permissions(&engine_socket, fs::Permissions::from_mode(0o600))?;
                            active.bridge = Some(bridge::ControllerBridge::bind(
                                &active.socket,
                                engine_socket.clone(),
                                uid,
                                gid,
                            )?);
                            return Ok(active.status());
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    bail!("owned TUN controller did not become ready")
                }
            }
        })();
        let result = match result {
            Err(error) => match cleanup_created_session(&mut session, started_this_request, Session::stop) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.context(format!("owned TUN startup cleanup failed: {cleanup}; session identity and active configuration retained for Status/Stop"))),
            },
            success => success,
        };
        let reply = match result {
            Ok(status) => HelperReply {
                status: Some(status),
                error: None,
            },
            Err(error) => HelperReply {
                status: None,
                error: Some(error.to_string()),
            },
        };
        let body = serde_json::to_vec(&reply)?;
        let delivered = stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .and_then(|_| stream.write_all(&body));
        if delivered.is_err() {
            let _ = cleanup_created_session(&mut session, started_this_request, Session::stop);
        }
    }
    if let Some(active) = &mut session {
        while active.stop().is_err() {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    fs::remove_file(socket)?;
    Ok(())
}

#[cfg(test)]
mod trust_tests {
    use super::*;
    fn directory(uid: u32, gid: u32, mode: u32) -> PathMetadata {
        PathMetadata {
            uid,
            gid,
            mode,
            directory: true,
            file: false,
            symlink: false,
        }
    }
    #[test]
    fn applications_exception_is_exact_and_never_extends_to_bundle() {
        assert!(trusted_path_metadata(
            Path::new("/Applications"),
            true,
            directory(0, 80, 0o775)
        ));
        assert!(trusted_path_metadata(
            Path::new("/Applications"),
            true,
            directory(0, 0, 0o755)
        ));
        for path in [
            "/Applications/Clash Native.app",
            "/Applications/Clash Native.app/Contents",
            "/fixture/Applications",
            "/Applications/",
        ] {
            // Path comparison canonicalizes trailing separators; that spelling names the same fixed container.
            if Path::new(path) == Path::new("/Applications") {
                continue;
            }
            assert!(!trusted_path_metadata(Path::new(path), true, directory(0, 80, 0o775)));
        }
        assert!(!trusted_path_metadata(
            Path::new("/Applications"),
            true,
            directory(501, 80, 0o775)
        ));
        assert!(!trusted_path_metadata(
            Path::new("/Applications"),
            true,
            directory(0, 20, 0o775)
        ));
        assert!(!trusted_path_metadata(
            Path::new("/Applications"),
            true,
            directory(0, 80, 0o777)
        ));
        let mut symlink = directory(0, 80, 0o775);
        symlink.symlink = true;
        assert!(!trusted_path_metadata(Path::new("/Applications"), true, symlink));
        assert!(!trusted_path_metadata(
            Path::new("/Applications"),
            false,
            directory(0, 80, 0o775)
        ));
    }
    #[test]
    fn signature_checks_include_nested_strict_and_every_architecture() {
        assert_eq!(STATIC_SIGNATURE_FLAGS, 25);
        assert_eq!((HELPER_LOG_MAXIMUM, HELPER_LOG_ARCHIVES), (2 * 1024 * 1024, 3));
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    #[test]
    fn startup_cleanup_only_stops_session_created_by_that_request() {
        let mut existing = Some(7_u64);
        let mut calls = 0;
        cleanup_created_session(&mut existing, false, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(existing, Some(7));
        assert_eq!(calls, 0);
        cleanup_created_session(&mut existing, true, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert!(existing.is_none());
        assert_eq!(calls, 1);
    }
    #[test]
    fn startup_cleanup_failure_retains_identity_for_retry() {
        let mut created = Some((501, "native-mac:fixture", 8));
        assert!(cleanup_created_session(&mut created, true, |_| bail!("fixture stop timeout")).is_err());
        assert_eq!(created, Some((501, "native-mac:fixture", 8)));
        cleanup_created_session(&mut created, true, |_| Ok(())).unwrap();
        assert!(created.is_none());
    }
    #[test]
    fn staging_cleanup_preserves_active_config_provider_refs_and_unrelated_paths() {
        let directory = Path::new("/fixture/uid");
        let active = directory.join("staging-42-3/config.yaml");
        let providers = vec![directory.join("staging-42-2/rules.yaml")];
        let stale = directory.join("staging-42-1");
        let current = directory.join("staging-42-3");
        let referenced = directory.join("staging-42-2");
        assert!(owned_staging_path(directory, &stale));
        assert!(!staging_is_referenced(&stale, &active, &providers));
        assert!(staging_is_referenced(&current, &active, &providers));
        assert!(staging_is_referenced(&referenced, &active, &providers));
        for name in ["privileged", "staging-unrelated", "staging-42-", "staging-42-1/child"] {
            assert!(!owned_staging_path(directory, &directory.join(name)));
        }
        assert!(!owned_staging_path(directory, Path::new("/foreign/staging-42-1")));
    }
}
