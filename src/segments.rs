use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub struct Options {
    pub runtimes: bool,
    pub environment: bool,
    pub session: bool,
    pub timeout: Duration,
}

const MAX_OUTPUT: usize = 8 * 1024;
const MAX_ANCESTORS: usize = 12;
const MAX_CACHE_FILE: u64 = 16 * 1024;
const CACHE_TTL_MS: u64 = 60_000;
const CACHE_SLOTS: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Runtime {
    Node,
    Python,
    Rust,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Context {
    node: bool,
    python: bool,
    rust: bool,
}

/// Detect optional prompt labels. All runtime probes share one timeout budget.
/// Labels are untrusted text and must be sanitized by the renderer.
pub fn detect(path: &Path, options: &Options) -> Vec<String> {
    let started = Instant::now();
    let deadline = started.checked_add(options.timeout).unwrap_or(started);
    let mut labels = Vec::new();
    if options.runtimes {
        let context = project_context(path);
        for (enabled, runtime) in [
            (context.node, Runtime::Node),
            (context.python, Runtime::Python),
            (context.rust, Runtime::Rust),
        ] {
            if enabled && Instant::now() < deadline {
                if let Some(label) = probe(runtime, deadline) {
                    labels.push(label);
                }
            }
        }
    }
    if options.environment {
        let virtual_env = env::var_os("VIRTUAL_ENV");
        let conda_env = env::var_os("CONDA_DEFAULT_ENV");
        if let Some(label) = environment_label(virtual_env.as_deref(), conda_env.as_deref()) {
            labels.push(label);
        }
    }
    if options.session {
        labels.extend(session_labels(|key| env::var_os(key)));
    }
    labels
}

fn project_context(path: &Path) -> Context {
    let mut context = Context::default();
    // Resolve relative input once, without canonicalizing (which may access a
    // network filesystem or resolve symlinks outside the supplied directory).
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else if let Ok(cwd) = env::current_dir() {
        cwd.join(path)
    } else {
        return context;
    };
    // Include the current directory and at most twelve parent directories.
    for directory in absolute.ancestors().take(MAX_ANCESTORS + 1) {
        context.node |= directory.join("package.json").is_file();
        context.python |= [
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            ".python-version",
        ]
        .iter()
        .any(|name| directory.join(name).is_file());
        context.rust |= ["Cargo.toml", "rust-toolchain.toml"]
            .iter()
            .any(|name| directory.join(name).is_file());
        if context.node && context.python && context.rust {
            break;
        }
    }
    context
}

fn environment_label(virtual_env: Option<&OsStr>, conda_env: Option<&OsStr>) -> Option<String> {
    let name = virtual_env
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            let value = value.to_string_lossy();
            let name = value
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()?;
            (!name.is_empty()).then(|| name.to_owned())
        })
        .or_else(|| {
            conda_env
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string_lossy().into_owned())
        })?;
    Some(format!("env:{name}"))
}

fn session_labels(mut lookup: impl FnMut(&str) -> Option<OsString>) -> Vec<String> {
    let mut labels = Vec::new();
    let mut present = |key| lookup(key).is_some_and(|value| !value.is_empty());
    if present("SSH_CONNECTION") || present("SSH_TTY") {
        labels.push("ssh".to_owned());
    }
    // Trusted shell integration may set this once at initialization after checking
    // its token. This is only a display hint, never an authorization decision.
    // In particular, USER=root and USERNAME=Administrator are not evidence.
    if lookup("OPENPROMPT_ADMIN").as_deref() == Some(OsStr::new("1")) {
        labels.push("admin".to_owned());
    }
    labels
}

fn executable(runtime: Runtime) -> Option<PathBuf> {
    #[cfg(windows)]
    let names: &[&str] = match runtime {
        Runtime::Node => &["node.exe"],
        Runtime::Python => &["python.exe", "python3.exe"],
        Runtime::Rust => &["rustc.exe"],
    };
    #[cfg(not(windows))]
    let names: &[&str] = match runtime {
        Runtime::Node => &["node"],
        Runtime::Python => &["python3", "python"],
        Runtime::Rust => &["rustc"],
    };
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        // Never implicitly search the project directory or interpret scripts via
        // a shell. Relative PATH entries are intentionally excluded as well.
        if !directory.is_absolute() {
            continue;
        }
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if candidate.metadata().ok()?.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                return Some(candidate);
            }
        }
    }
    None
}

fn probe(runtime: Runtime, deadline: Instant) -> Option<String> {
    let executable = executable(runtime)?;
    let cache_key = cache_key(runtime, &executable);
    if let Some(label) = cache_key.as_ref().and_then(read_cache) {
        return Some(label);
    }
    let label = probe_executable(runtime, &executable, deadline)?;
    if let Some(key) = cache_key {
        // Only successful, parsed probes are cached. Failure is best effort.
        let _ = write_cache(key, &label);
    }
    Some(label)
}

fn probe_executable(runtime: Runtime, executable: &Path, deadline: Instant) -> Option<String> {
    let mut command = Command::new(executable);
    match runtime {
        Runtime::Node => {
            command.args(["--version"]);
            command.env_remove("NODE_OPTIONS").env_remove("NODE_PATH");
        }
        Runtime::Python => {
            command.args(["-I", "-S", "--version"]);
            for (key, _) in env::vars_os() {
                // Windows environment variable names are case-insensitive.
                let name = key.to_string_lossy();
                let is_python = if cfg!(windows) {
                    name.to_ascii_uppercase().starts_with("PYTHON")
                } else {
                    name.starts_with("PYTHON")
                };
                if is_python {
                    command.env_remove(key);
                }
            }
        }
        Runtime::Rust => {
            command.args(["--version"]);
            command.env("RUSTUP_AUTO_INSTALL", "0");
        }
    }
    // Avoid local Python modules and rustup project overrides. An explicitly
    // selected RUSTUP_TOOLCHAIN remains honored, but cannot trigger a download.
    command.current_dir(env::temp_dir());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    if Instant::now() >= deadline {
        return None;
    }
    let mut child = command.spawn().ok()?;
    let (sender, receiver) = mpsc::channel();
    // Each pipe retains at most half the total output limit. Readers keep
    // draining after overflow so that the child cannot deadlock on a full pipe.
    // Never join these threads: a descendant might retain an inherited pipe.
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let readers_started = start_reader(stdout, sender.clone()).is_ok()
        && start_reader(stderr, sender.clone()).is_ok();
    drop(sender);
    if !readers_started {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    loop {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) => std::thread::sleep(
                Duration::from_millis(1).min(deadline.saturating_duration_since(Instant::now())),
            ),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut label = None;
    for _ in 0..2 {
        let bytes = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()??;
        if let Some(parsed) = parse_version(runtime, &bytes) {
            label = Some(parsed);
        }
    }
    label
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    length: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheKey {
    runtime: Runtime,
    executable: String,
    executable_stamp: FileStamp,
    rustup_toolchain: Option<String>,
    rustup_settings: Option<String>,
    rustup_settings_stamp: Option<FileStamp>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheRecord {
    key: CacheKey,
    created_ms: u64,
    label: String,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(FileStamp {
        length: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn encoded_path(value: &OsStr) -> String {
    // Preserve non-Unicode paths, rather than allowing lossy key collisions.
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in value.as_encoded_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn cache_key(runtime: Runtime, executable: &Path) -> Option<CacheKey> {
    let mut key = CacheKey {
        runtime,
        executable: encoded_path(executable.as_os_str()),
        executable_stamp: file_stamp(executable)?,
        rustup_toolchain: None,
        rustup_settings: None,
        rustup_settings_stamp: None,
    };
    if runtime == Runtime::Rust {
        key.rustup_toolchain = env::var_os("RUSTUP_TOOLCHAIN").as_deref().map(encoded_path);
        let home = env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".rustup")))?;
        let home = if home.is_absolute() {
            home
        } else {
            // Probes run in the temporary directory, not the project directory.
            env::temp_dir().join(home)
        };
        let settings = home.join("settings.toml");
        key.rustup_settings = Some(encoded_path(settings.as_os_str()));
        match fs::metadata(&settings) {
            Ok(_) => key.rustup_settings_stamp = Some(file_stamp(&settings)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(key)
}

fn cache_directory() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("openprompt").join("runtime-v1"))
}

fn checked_directory(path: &Path, create: bool) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            std::io::Error::other("runtime cache directory is not a real directory"),
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if create {
                // Create only the two owned directories, never an arbitrary tree.
                match fs::create_dir(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                checked_directory(path, false)
            } else {
                Ok(false)
            }
        }
        Err(error) => Err(error),
    }
}

fn ready_cache_directory(path: &Path, create: bool) -> std::io::Result<bool> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("invalid cache path"))?;
    if !checked_directory(parent, create)? {
        return Ok(false);
    }
    checked_directory(path, create)
}

fn cache_filename(key: &CacheKey) -> Option<String> {
    // Fixed slots bound persistent cache data to 64 * 16 KiB. Hash collisions
    // only evict entries: the entire key is always verified before a cache hit.
    let serialized = toml::to_string(key).ok()?;
    let hash = serialized
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    Some(format!("runtime-{:02x}.toml", hash % CACHE_SLOTS))
}

fn recognized_cache_filename(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    (0..CACHE_SLOTS).any(|slot| name == format!("runtime-{slot:02x}.toml"))
}

fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn parse_cache_record(bytes: &[u8], key: &CacheKey, now_ms: u64) -> Option<String> {
    if bytes.len() as u64 > MAX_CACHE_FILE {
        return None;
    }
    let record: CacheRecord = toml::from_str(std::str::from_utf8(bytes).ok()?).ok()?;
    if record.key != *key || now_ms.checked_sub(record.created_ms)? >= CACHE_TTL_MS {
        return None;
    }
    // A writable cache must not bypass the version parser's label validation.
    let raw = match key.runtime {
        Runtime::Node => record.label.strip_prefix("node:")?.to_owned(),
        Runtime::Python => format!("Python {}", record.label.strip_prefix("python:")?),
        Runtime::Rust => format!("rustc {}", record.label.strip_prefix("rust:")?),
    };
    let parsed = parse_version(key.runtime, raw.as_bytes())?;
    (parsed == record.label).then_some(parsed)
}

fn read_cache(key: &CacheKey) -> Option<String> {
    let directory = cache_directory()?;
    if !ready_cache_directory(&directory, false).ok()? {
        return None;
    }
    let path = directory.join(cache_filename(key)?);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CACHE_FILE {
        return None;
    }
    let file = fs::File::open(&path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_FILE + 1).read_to_end(&mut bytes).ok()?;
    parse_cache_record(&bytes, key, now_ms()?)
}

fn write_cache(key: CacheKey, label: &str) -> Option<()> {
    let directory = cache_directory()?;
    if !ready_cache_directory(&directory, true).ok()? {
        return None;
    }
    let path = directory.join(cache_filename(&key)?);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => return None,
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let record = CacheRecord {
        key,
        created_ms: now_ms()?,
        label: label.to_owned(),
    };
    let bytes = toml::to_string(&record).ok()?;
    if bytes.len() as u64 > MAX_CACHE_FILE {
        return None;
    }
    // NamedTempFile is private by default, resides on the same filesystem, and
    // persist atomically replaces one slot without exposing partial TOML.
    let mut temporary = tempfile::Builder::new()
        .prefix(".runtime-")
        .tempfile_in(&directory)
        .ok()?;
    temporary.write_all(bytes.as_bytes()).ok()?;
    temporary.persist(path).ok()?;
    Some(())
}

/// Delete recognized runtime cache files only; never follow a symlink directory
/// or recurse into other cache data. Missing cache directories are harmless.
pub fn clear_cache() -> Result<usize, String> {
    let Some(directory) = cache_directory() else {
        return Ok(0);
    };
    if !ready_cache_directory(&directory, false).map_err(|error| error.to_string())? {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !recognized_cache_filename(&entry.file_name()) {
            continue;
        }
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_file() && !kind.is_symlink() {
            match fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    Ok(removed)
}

fn start_reader(
    mut pipe: impl Read + Send + 'static,
    sender: mpsc::Sender<Option<Vec<u8>>>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("openprompt-version".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            let mut valid = true;
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let retained = count.min(MAX_OUTPUT / 2 - bytes.len());
                        bytes.extend_from_slice(&buffer[..retained]);
                        valid &= retained == count;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            let _ = sender.send(valid.then_some(bytes));
        })
        .map(|_| ())
}

fn parse_version(runtime: Runtime, bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let (label, version) = match runtime {
        Runtime::Node => ("node:v", text.strip_prefix('v')?),
        Runtime::Python => ("python:", text.strip_prefix("Python ")?),
        Runtime::Rust => ("rust:", text.strip_prefix("rustc ")?),
    };
    let version = version.split_whitespace().next()?;
    if version.len() > 128
        || !version.starts_with(|c: char| c.is_ascii_digit())
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        || !version.contains('.')
    {
        return None;
    }
    Some(format!("{label}{version}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn sample_cache_key(path: &str) -> CacheKey {
        CacheKey {
            runtime: Runtime::Node,
            executable: encoded_path(OsStr::new(path)),
            executable_stamp: FileStamp {
                length: 123,
                modified_seconds: 42,
                modified_nanos: 123,
            },
            rustup_toolchain: None,
            rustup_settings: None,
            rustup_settings_stamp: None,
        }
    }

    fn sample_cache_record(key: CacheKey, created_ms: u64, label: &str) -> Vec<u8> {
        toml::to_string(&CacheRecord {
            key,
            created_ms,
            label: label.to_owned(),
        })
        .unwrap()
        .into_bytes()
    }

    #[test]
    fn cached_records_expire_and_reject_future_timestamps() {
        let key = sample_cache_key("/bin/node");
        let bytes = sample_cache_record(sample_cache_key("/bin/node"), 100_000, "node:v22.0.0");
        assert_eq!(
            parse_cache_record(&bytes, &key, 100_000),
            Some("node:v22.0.0".into())
        );
        assert_eq!(
            parse_cache_record(&bytes, &key, 159_999),
            Some("node:v22.0.0".into())
        );
        assert_eq!(parse_cache_record(&bytes, &key, 160_000), None);
        assert_eq!(parse_cache_record(&bytes, &key, 99_999), None);
    }

    #[test]
    fn cache_slot_collisions_require_the_full_key() {
        let mut slots = std::collections::HashMap::new();
        for index in 0..=CACHE_SLOTS {
            let path = format!("/tools/{index}/node");
            let key = sample_cache_key(&path);
            let filename = cache_filename(&key).unwrap();
            assert!(recognized_cache_filename(OsStr::new(&filename)));
            if let Some(previous) = slots.insert(filename, path.clone()) {
                let bytes =
                    sample_cache_record(sample_cache_key(&previous), 100_000, "node:v22.0.0");
                assert_eq!(parse_cache_record(&bytes, &key, 100_000), None);
                return;
            }
        }
        panic!("65 distinct keys must collide in 64 slots");
    }

    #[test]
    fn cached_records_reject_changed_metadata_and_invalid_labels() {
        let mut key = sample_cache_key("/bin/node");
        let bytes = sample_cache_record(sample_cache_key("/bin/node"), 100_000, "node:v22.0.0");
        key.executable_stamp.length += 1;
        assert_eq!(parse_cache_record(&bytes, &key, 100_000), None);
        key = sample_cache_key("/bin/node");
        key.executable_stamp.modified_nanos += 1;
        assert_eq!(parse_cache_record(&bytes, &key, 100_000), None);
        key = sample_cache_key("/bin/node");
        key.rustup_toolchain = Some("nightly".into());
        assert_eq!(parse_cache_record(&bytes, &key, 100_000), None);
        for label in [
            "node:v22.0.0\u{1b}[0m",
            "node:v22.0.0 extra",
            "python:3.13.0",
            "",
        ] {
            let bytes = sample_cache_record(sample_cache_key("/bin/node"), 100_000, label);
            assert_eq!(
                parse_cache_record(&bytes, &sample_cache_key("/bin/node"), 100_000),
                None
            );
        }
        assert_eq!(
            parse_cache_record(b"not valid toml", &sample_cache_key("/bin/node"), 100_000),
            None
        );
        assert_eq!(
            parse_cache_record(
                &vec![b' '; MAX_CACHE_FILE as usize + 1],
                &sample_cache_key("/bin/node"),
                100_000
            ),
            None
        );
    }

    #[test]
    fn cache_recognizes_only_its_fixed_slot_names() {
        for name in [
            "runtime-40.toml",
            "runtime-ff.toml",
            "runtime-0A.toml",
            "runtime-0.toml",
            "runtime-00.toml.bak",
            ".runtime-temp",
            "other.toml",
        ] {
            assert!(!recognized_cache_filename(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn relative_context_search_matches_absolute_current_directory() {
        let cwd = env::current_dir().unwrap();
        assert_eq!(project_context(Path::new(".")), project_context(&cwd));
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = env::temp_dir().join(format!(
                    "openprompt-segments-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_runtime_versions() {
        assert_eq!(
            parse_version(Runtime::Node, b"v22.14.0\r\n"),
            Some("node:v22.14.0".into())
        );
        assert_eq!(
            parse_version(Runtime::Python, b"Python 3.13.2\n"),
            Some("python:3.13.2".into())
        );
        assert_eq!(
            parse_version(Runtime::Rust, b"rustc 1.87.0-nightly (abc 2025-01-01)\n"),
            Some("rust:1.87.0-nightly".into())
        );
        for bytes in [b"".as_slice(), b"vgarbage", b"v22.0\x1b[0m", b"v22.\xff"] {
            assert_eq!(parse_version(Runtime::Node, bytes), None);
        }
    }

    #[test]
    fn finds_each_marker_in_current_directory_and_ancestors() {
        let temp = TempDirectory::new();
        let child = temp.0.join("child");
        std::fs::create_dir(&child).unwrap();
        // A directory with a marker's name must not count as a project file.
        std::fs::create_dir(temp.0.join("package.json")).unwrap();
        assert!(!project_context(&child).node);
        std::fs::remove_dir(temp.0.join("package.json")).unwrap();
        for marker in [
            "package.json",
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            ".python-version",
            "Cargo.toml",
            "rust-toolchain.toml",
        ] {
            let file = temp.0.join(marker);
            std::fs::write(&file, "").unwrap();
            let expected = Context {
                node: marker == "package.json",
                python: matches!(
                    marker,
                    "pyproject.toml" | "requirements.txt" | "Pipfile" | ".python-version"
                ),
                rust: matches!(marker, "Cargo.toml" | "rust-toolchain.toml"),
            };
            assert_eq!(project_context(&temp.0), expected);
            assert_eq!(project_context(&child), expected);
            std::fs::remove_file(file).unwrap();
        }
    }

    #[test]
    fn limits_ancestor_search() {
        let temp = TempDirectory::new();
        std::fs::write(temp.0.join("package.json"), "").unwrap();
        let mut path = temp.0.clone();
        for _ in 0..MAX_ANCESTORS {
            path.push("nested");
        }
        std::fs::create_dir_all(&path).unwrap();
        assert!(project_context(&path).node);
        path.push("too-deep");
        std::fs::create_dir(&path).unwrap();
        assert!(!project_context(&path).node);
    }

    #[test]
    fn environment_names_prefer_virtual_env() {
        assert_eq!(
            environment_label(
                Some(OsStr::new("C:\\work\\.venv\\")),
                Some(OsStr::new("base"))
            ),
            Some("env:.venv".into())
        );
        assert_eq!(
            environment_label(Some(OsStr::new("/work/venv/")), None),
            Some("env:venv".into())
        );
        assert_eq!(
            environment_label(Some(OsStr::new("")), Some(OsStr::new("science"))),
            Some("env:science".into())
        );
        assert_eq!(environment_label(None, None), None);
    }

    #[test]
    fn session_requires_ssh_or_explicit_integration_marker() {
        let labels = session_labels(|key| match key {
            "SSH_TTY" => Some("/dev/pts/0".into()),
            "OPENPROMPT_ADMIN" => Some("1".into()),
            _ => None,
        });
        assert_eq!(labels, ["ssh", "admin"]);
        assert!(session_labels(|key| match key {
            "USER" => Some("root".into()),
            "USERNAME" => Some("Administrator".into()),
            "OPENPROMPT_ADMIN" => Some("true".into()),
            "SSH_CONNECTION" => Some("".into()),
            _ => None,
        })
        .is_empty());
    }
}
