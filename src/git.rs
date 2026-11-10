use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub dirty: bool,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicts: u32,
}

const MAX_OUTPUT: usize = 64 * 1024;

fn git_command() -> Command {
    let mut command = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Detect status using a short-lived, best-effort disk cache.
pub fn detect_cached(path: &Path, timeout: Duration, cache_ms: u64) -> Option<GitStatus> {
    if cache_ms == 0 {
        return detect(path, timeout);
    }
    let absolute = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => return detect(path, timeout),
    };
    if let Some(status) = crate::cache::get(&absolute, cache_ms) {
        return Some(status);
    }
    let status = detect(path, timeout)?;
    crate::cache::put(&absolute, &status);
    Some(status)
}

/// Detect repository status without modifying the index or waiting indefinitely.
pub fn detect(path: &Path, timeout: Duration) -> Option<GitStatus> {
    let started = Instant::now();
    let mut child = git_command()
        .args([
            "--no-optional-locks",
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
            "--ignore-submodules=all",
        ])
        .current_dir(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    // Drain concurrently: waiting before reading can deadlock on a full pipe.
    // Keep at most MAX_OUTPUT bytes, but continue draining until git exits.
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::Builder::new().spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let mut overflow = false;
        loop {
            let count = match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            let available = MAX_OUTPUT - bytes.len();
            bytes.extend_from_slice(&buffer[..count.min(available)]);
            overflow |= count > available;
        }
        (!overflow).then_some(bytes)
    });
    let reader = match reader {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    let success = loop {
        if started.elapsed() >= timeout {
            break false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => {
                std::thread::sleep(
                    Duration::from_millis(5).min(timeout.saturating_sub(started.elapsed())),
                );
            }
            Err(_) => break false,
        }
    };
    if !success {
        let _ = child.kill();
        let _ = child.wait();
    }
    // With fsmonitor disabled, git status has no helper retaining this pipe.
    // Killing and reaping git closes its write end and releases the reader.
    let bytes = reader.join().ok()??;
    if !success {
        return None;
    }
    parse_status(&bytes)
}

fn parse_status(bytes: &[u8]) -> Option<GitStatus> {
    // Filenames need not be UTF-8; only interpret the branch metadata as text.
    let mut status = GitStatus::default();
    let mut head = None;
    let mut oid = None;
    let mut saw_ab = false;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(value) = line.strip_prefix(b"# branch.head ") {
            if head.is_some() || value.is_empty() {
                return None;
            }
            head = Some(std::str::from_utf8(value).ok()?);
        } else if let Some(value) = line.strip_prefix(b"# branch.oid ") {
            if oid.is_some() {
                return None;
            }
            let value = std::str::from_utf8(value).ok()?;
            if value != "(initial)"
                && !((value.len() == 40 || value.len() == 64)
                    && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                return None;
            }
            oid = Some(value);
        } else if let Some(value) = line.strip_prefix(b"# branch.ab ") {
            if saw_ab {
                return None;
            }
            saw_ab = true;
            let value = std::str::from_utf8(value).ok()?;
            let (ahead, behind) = value.split_once(' ')?;
            let ahead = ahead.strip_prefix('+')?;
            let behind = behind.strip_prefix('-')?;
            if ahead.is_empty()
                || behind.is_empty()
                || !ahead.bytes().all(|byte| byte.is_ascii_digit())
                || !behind.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            status.ahead = ahead.parse().ok()?;
            status.behind = behind.parse().ok()?;
        } else if line.starts_with(b"# ") || line.starts_with(b"! ") {
            // Other porcelain headers (e.g. upstream) and ignored entries.
        } else if line.starts_with(b"1 ") || line.starts_with(b"2 ") {
            let xy = line.get(2..4)?;
            if line.get(4) != Some(&b' ') || !xy.iter().all(|byte| b".MTADRCU".contains(byte)) {
                return None;
            }
            status.staged += u32::from(xy[0] != b'.');
            status.modified += u32::from(xy[1] != b'.');
            status.dirty = true;
        } else if line.starts_with(b"u ") && line.len() > 2 {
            status.conflicts += 1;
            status.dirty = true;
        } else if line.starts_with(b"? ") && line.len() > 2 {
            status.untracked += 1;
            status.dirty = true;
        } else {
            return None;
        }
    }
    let head = head?;
    let oid = oid?;
    status.branch = if head == "(detached)" {
        if oid == "(initial)" {
            return None;
        }
        oid[..7].to_owned()
    } else {
        head.to_owned()
    };
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    fn output(head: &str, extra: &str) -> String {
        format!("# branch.oid {OID}\n# branch.head {head}\n{extra}")
    }

    #[test]
    fn clean_and_raw_branch() {
        let status = parse_status(output("feature/raw", "").as_bytes()).unwrap();
        assert_eq!(
            status,
            GitStatus {
                branch: "feature/raw".into(),
                ..GitStatus::default()
            }
        );
    }

    #[test]
    fn dirty_entries_and_divergence() {
        for entry in ["1 M. tracked", "2 R. renamed", "u conflict", "? untracked"] {
            let text = output(
                "main",
                &format!("# branch.upstream origin/main\n# branch.ab +12 -3\n{entry}\n"),
            );
            let status = parse_status(text.as_bytes()).unwrap();
            assert!(status.dirty);
            assert_eq!((status.ahead, status.behind), (12, 3));
        }
        let mut text = output("main", "? ").into_bytes();
        text.extend_from_slice(&[0xff, b'\n']);
        assert!(parse_status(&text).unwrap().dirty);
    }

    #[test]
    fn counts_each_status_kind() {
        let status = parse_status(output("main", "1 M. staged\n1 .M modified\n1 MM both\n2 R. renamed\nu UU conflict\n? new\n? another\n").as_bytes()).unwrap();
        assert_eq!(
            (
                status.staged,
                status.modified,
                status.untracked,
                status.conflicts
            ),
            (3, 2, 2, 1)
        );
        assert!(status.dirty);
        assert!(parse_status(output("main", "1 invalid\n").as_bytes()).is_none());
    }

    #[test]
    fn detached_and_unborn() {
        assert_eq!(
            parse_status(output("(detached)", "").as_bytes())
                .unwrap()
                .branch,
            "0123456"
        );
        let status = parse_status(b"# branch.oid (initial)\n# branch.head main\n").unwrap();
        assert_eq!(status.branch, "main");
        assert!(!status.dirty);
    }

    #[test]
    fn invalid_output() {
        for text in [
            String::new(),
            "fatal: not a git repository\n".into(),
            "# branch.head main\n".into(),
            "# branch.oid invalid\n# branch.head main\n".into(),
            output("main", "# branch.ab +4294967296 -0\n"),
            output("main", "# branch.ab +1 -no\n"),
            output("main", "nonsense\n"),
            output("main", "# branch.head duplicate\n"),
            "# branch.oid (initial)\n# branch.head (detached)\n".into(),
        ] {
            assert_eq!(parse_status(text.as_bytes()), None, "{text:?}");
        }
    }

    struct TempRepo(PathBuf);

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(path: &Path, args: &[&OsStr]) -> bool {
        git_command()
            .current_dir(path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn temporary_repository() {
        if !run(&std::env::temp_dir(), &[OsStr::new("--version")]) {
            return; // Git is optional on the host running the tests.
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let repo = loop {
            let path = std::env::temp_dir().join(format!(
                "openprompt-git-test-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => break TempRepo(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory: {error}"),
            }
        };
        let timeout = Duration::from_secs(10);
        assert_eq!(detect(&repo.0, timeout), None);
        assert_eq!(detect_cached(&repo.0, timeout, 0), None);
        assert!(run(&repo.0, &[OsStr::new("init"), OsStr::new("--quiet")]));
        let unborn = detect(&repo.0, timeout).unwrap();
        assert_eq!(detect_cached(&repo.0, timeout, 0), Some(unborn.clone()));
        assert!(!unborn.branch.is_empty());
        assert!(!unborn.dirty);
        std::fs::write(repo.0.join("tracked.txt"), "first\n").unwrap();
        let untracked = detect(&repo.0, timeout).unwrap();
        assert!(untracked.dirty);
        assert_eq!(untracked.untracked, 1);
        assert!(run(
            &repo.0,
            &[
                OsStr::new("add"),
                OsStr::new("--"),
                OsStr::new("tracked.txt")
            ]
        ));
        assert_eq!(detect(&repo.0, timeout).unwrap().staged, 1);
        let args = [
            "-c",
            "user.name=Openprompt Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
            "commit",
            "--quiet",
            "-m",
            "test",
        ];
        let args: Vec<_> = args.iter().map(OsStr::new).collect();
        assert!(run(&repo.0, &args));
        assert!(!detect(&repo.0, timeout).unwrap().dirty);
        assert!(run(
            &repo.0,
            &[
                OsStr::new("checkout"),
                OsStr::new("--quiet"),
                OsStr::new("--detach")
            ]
        ));
        assert_eq!(detect(&repo.0, timeout).unwrap().branch.len(), 7);
        std::fs::write(repo.0.join("tracked.txt"), "changed\n").unwrap();
        assert!(detect(&repo.0, timeout).unwrap().dirty);
        assert_eq!(detect(&repo.0, Duration::ZERO), None);
    }
}
