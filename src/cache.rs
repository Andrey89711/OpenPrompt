//! Best-effort Git status cache. Records contain absolute local paths and branch
//! names (not file contents); clearing removes this persisted private metadata.
use crate::git::GitStatus;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 64 * 1024;
const MAX_TTL_MS: u64 = 5000;

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u32,
    timestamp_ms: u64,
    path: String,
    result: GitStatus,
    fingerprint: Option<Fingerprint>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Fingerprint {
    head: Option<Stamp>,
    index: Option<Stamp>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    length: u64,
    seconds: u64,
    nanos: u32,
}

fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn directory() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("openprompt").join("git-v1"))
}

fn filename(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}.toml", hasher.finish())
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn prepare_directory(path: &Path) -> std::io::Result<()> {
    if !path.parent().is_some_and(real_directory) {
        return Err(std::io::Error::other(
            "cache parent is not a real directory",
        ));
    }
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && real_directory(path) => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn read_entry(path: &Path) -> Option<Entry> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let entry: Entry = toml::from_str(std::str::from_utf8(&bytes).ok()?).ok()?;
    (entry.version == 1 && Path::new(&entry.path).is_absolute()).then_some(entry)
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let git = path
        .ancestors()
        .map(|parent| parent.join(".git"))
        .find(|git| git.exists())?;
    // Worktrees/submodules use gitfiles: TTL alone remains the fallback there.
    if !git.is_dir() {
        return None;
    }
    let stamp = |name| {
        let metadata = fs::metadata(git.join(name)).ok()?;
        let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Stamp {
            length: metadata.len(),
            seconds: modified.as_secs(),
            nanos: modified.subsec_nanos(),
        })
    };
    Some(Fingerprint {
        head: stamp("HEAD"),
        index: stamp("index"),
    })
}

pub fn get(path: &Path, ttl_ms: u64) -> Option<GitStatus> {
    get_at(&directory()?, path, ttl_ms, now_ms()?)
}

fn get_at(directory: &Path, path: &Path, ttl_ms: u64, now: u64) -> Option<GitStatus> {
    if ttl_ms == 0 || !real_directory(directory) || !real_directory(directory.parent()?) {
        return None;
    }
    let exact = path.to_str()?;
    let entry = read_entry(&directory.join(filename(exact)))?;
    if entry.path != exact
        || now.checked_sub(entry.timestamp_ms)? >= ttl_ms.min(MAX_TTL_MS)
        || entry.fingerprint != fingerprint(path)
    {
        return None;
    }
    Some(entry.result)
}

pub fn put(path: &Path, status: &GitStatus) {
    let _ = (|| {
        let directory = directory()?;
        prepare_directory(directory.parent()?).ok()?;
        prepare_directory(&directory).ok()?;
        put_at(&directory, path, status, now_ms()?).ok()
    })();
}

fn put_at(
    directory: &Path,
    path: &Path,
    status: &GitStatus,
    timestamp_ms: u64,
) -> std::io::Result<()> {
    if !real_directory(directory) || !path.is_absolute() {
        return Err(std::io::Error::other("invalid cache directory or path"));
    }
    let exact = path
        .to_str()
        .ok_or_else(|| std::io::Error::other("non-UTF-8 path"))?;
    let destination = directory.join(filename(exact));
    if fs::symlink_metadata(&destination).is_ok()
        && !read_entry(&destination).is_some_and(|entry| entry.path == exact)
    {
        return Err(std::io::Error::other(
            "cache destination is not an owned entry",
        ));
    }
    let entry = Entry {
        version: 1,
        timestamp_ms,
        path: exact.into(),
        result: status.clone(),
        fingerprint: fingerprint(path),
    };
    let text = toml::to_string(&entry).map_err(std::io::Error::other)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("cache entry too large"));
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        timestamp_ms,
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = file.write_all(text.as_bytes());
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, &destination));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Remove only recognized cache records, never directories or unrelated files.
pub fn clear() -> Result<usize, String> {
    let directory = directory().ok_or("cache directory unavailable")?;
    clear_at(&directory)
}

fn clear_at(directory: &Path) -> Result<usize, String> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
        Ok(_) if !real_directory(directory) => {
            return Err("refusing non-directory or symlink cache".into())
        }
        Ok(_) => {}
    }
    if !directory.parent().is_some_and(real_directory) {
        return Err("refusing symlink cache parent".into());
    }
    let mut count = 0;
    for item in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        if let Some(entry) = read_entry(&item.path()) {
            if item.file_name() == std::ffi::OsStr::new(&filename(&entry.path)) {
                fs::remove_file(item.path()).map_err(|error| error.to_string())?;
                count += 1;
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temporary() -> Temp {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "openprompt-cache-test-{}-{}-{}",
            std::process::id(),
            now_ms().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Temp(path)
    }

    #[test]
    fn roundtrip_expiry_future_and_max_ttl() {
        let temp = temporary();
        let path = temp.0.join("repo");
        let status = GitStatus {
            branch: "main".into(),
            staged: 3,
            ..GitStatus::default()
        };
        put_at(&temp.0, &path, &status, 1000).unwrap();
        assert_eq!(get_at(&temp.0, &path, 1000, 1500), Some(status));
        assert_eq!(get_at(&temp.0, &path, 1000, 2000), None);
        assert_eq!(get_at(&temp.0, &path, 1000, 999), None);
        assert_eq!(get_at(&temp.0, &path, 0, 1000), None);
        assert_eq!(get_at(&temp.0, &path, u64::MAX, 6000), None);
    }

    #[test]
    fn collision_and_unrelated_files_are_preserved() {
        let temp = temporary();
        let first = temp.0.join("one");
        let second = temp.0.join("two");
        put_at(&temp.0, &first, &GitStatus::default(), 1000).unwrap();
        fs::rename(
            temp.0.join(filename(first.to_str().unwrap())),
            temp.0.join(filename(second.to_str().unwrap())),
        )
        .unwrap();
        assert_eq!(get_at(&temp.0, &second, 1000, 1000), None);
        assert!(put_at(&temp.0, &second, &GitStatus::default(), 1000).is_err());
        fs::write(temp.0.join("notes.txt"), "personal notes").unwrap();
        fs::write(temp.0.join(filename(first.to_str().unwrap())), "unrelated").unwrap();
        assert!(put_at(&temp.0, &first, &GitStatus::default(), 1000).is_err());
        assert_eq!(clear_at(&temp.0).unwrap(), 0);
        assert!(temp.0.join("notes.txt").exists());
    }

    #[test]
    fn clear_only_recognized_entries_and_bound_reads() {
        let temp = temporary();
        let path = temp.0.join("repo");
        put_at(&temp.0, &path, &GitStatus::default(), 1000).unwrap();
        fs::write(
            temp.0.join("large.toml"),
            vec![b'a'; MAX_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_entry(&temp.0.join("large.toml")).is_none());
        assert_eq!(clear_at(&temp.0).unwrap(), 1);
        assert!(temp.0.join("large.toml").exists());
    }

    #[test]
    fn head_and_index_changes_invalidate() {
        let temp = temporary();
        let repo = temp.0.join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        put_at(&temp.0, &repo, &GitStatus::default(), 1000).unwrap();
        assert!(get_at(&temp.0, &repo, 1000, 1000).is_some());
        fs::write(git.join("index"), "new index").unwrap();
        assert!(get_at(&temp.0, &repo, 1000, 1000).is_none());
        put_at(&temp.0, &repo, &GitStatus::default(), 1000).unwrap();
        assert!(get_at(&temp.0, &repo, 1000, 1000).is_some());
        fs::write(git.join("HEAD"), "ref: refs/heads/another-branch\n").unwrap();
        assert!(get_at(&temp.0, &repo, 1000, 1000).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directories_and_destinations_are_refused() {
        use std::os::unix::fs::symlink;
        let temp = temporary();
        let target = temp.0.join("target");
        let link = temp.0.join("link");
        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();
        let repo = temp.0.join("repo");
        assert!(clear_at(&link).is_err());
        assert!(put_at(&link, &repo, &GitStatus::default(), 1000).is_err());
        let notes = temp.0.join("notes");
        fs::write(&notes, "keep").unwrap();
        symlink(&notes, target.join(filename(repo.to_str().unwrap()))).unwrap();
        assert!(put_at(&target, &repo, &GitStatus::default(), 1000).is_err());
        assert_eq!(fs::read_to_string(notes).unwrap(), "keep");
        assert_eq!(clear_at(&target).unwrap(), 0);
    }
}
