//! wz-common: durability and robustness primitives shared by every crate.
//!
//! Workbench Zero is trusted with long-term personal data, so the persistence
//! rules below are non-negotiable everywhere a canonical state file is written:
//!
//! 1. **Atomic + durable writes** ([`atomic_write`]): write to a unique sibling
//!    temp file, `fsync` the temp file, `rename` over the target, then `fsync`
//!    the parent directory. A crash at any point leaves either the old or the
//!    new file — never a torn one — and the rename survives power loss.
//! 2. **Resilient reads** ([`load_json`]): a missing file is normal; a corrupt
//!    file is never silently discarded. We first try the legacy `.tmp` sibling
//!    (a crash may have interrupted an older write), then quarantine the
//!    corrupt bytes to `*.corrupt-<timestamp>` so no user data is destroyed,
//!    and report the outcome so callers can decide on defaults.
//! 3. **Poison-tolerant locks** ([`MutexRecover`], [`RwLockRecover`]): a panic
//!    in any single subsystem must not wedge the kernel. Recovered locks log a
//!    warning and hand out the guard instead of panicking every future caller.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::de::DeserializeOwned;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique temp path next to `target` (same directory ⇒ same filesystem ⇒ the
/// final `rename` is atomic). Unique names make concurrent writers and
/// crash-restart cycles collision-free.
fn tmp_path_for(target: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".into());
    target.with_file_name(format!(".{file_name}.tmp-{}-{n}", std::process::id()))
}

/// Legacy fixed temp name used by pre-1.0 writers (`path.with_extension("tmp")`).
/// If a crash interrupted such a write, the complete copy may still be there.
fn legacy_tmp_path_for(target: &Path) -> PathBuf {
    target.with_extension("tmp")
}

/// Atomically and durably replace the contents of `path` with `contents`.
///
/// The parent directory must exist. File `fsync` errors are propagated (the
/// write is not durable); directory `fsync` failures are logged and tolerated
/// so exotic filesystems do not break the application.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = tmp_path_for(path);
    let write_result = (|| -> io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result?;
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::File::open(parent).and_then(|d| d.sync_all()) {
            tracing::warn!(path = %path.display(), error = %e, "directory fsync failed (write is renamed but not fully durable)");
        }
    }
    Ok(())
}

/// Convenience wrapper for string payloads.
pub fn atomic_write_str(path: &Path, contents: &str) -> io::Result<()> {
    atomic_write(path, contents.as_bytes())
}

/// Outcome of a resilient JSON load.
#[derive(Debug)]
pub enum JsonLoad<T> {
    /// File does not exist — first run, use defaults.
    Missing,
    /// File existed and parsed.
    Loaded(T),
    /// Main file was corrupt or missing, but the legacy `.tmp` sibling held a
    /// complete copy which was promoted into place.
    RecoveredFromTmp(T),
    /// Main file was corrupt and no usable fallback existed. The corrupt bytes
    /// were moved aside (never deleted) to the returned path; use defaults.
    Corrupt { quarantined_to: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum JsonLoadError {
    #[error("io error reading `{0}`: {1}")]
    Io(PathBuf, std::io::Error),
}

/// Load JSON from `path` following the resilience rules in the module docs.
///
/// `T` must be deserializable with `serde_json`. Blank files are treated as
/// missing. Corrupt files are quarantined and never overwritten by this
/// function — data loss by truncation is only possible if the caller proceeds
/// to persist defaults, so callers should surface the `Corrupt` outcome to
/// logs/UI before writing anything back.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<JsonLoad<T>, JsonLoadError> {
    let read = |p: &Path| -> Result<Option<String>, JsonLoadError> {
        match std::fs::read_to_string(p) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(JsonLoadError::Io(p.to_path_buf(), e)),
        }
    };
    let parse = |raw: &str| -> Option<T> {
        if raw.trim().is_empty() {
            return None;
        }
        serde_json::from_str(raw).ok()
    };

    let main = read(path)?;
    if let Some(value) = main.as_deref().and_then(&parse) {
        return Ok(JsonLoad::Loaded(value));
    }
    if main.is_none() {
        return Ok(JsonLoad::Missing);
    }

    // Main file exists but is corrupt/blank: try the legacy tmp sibling.
    let legacy_tmp = legacy_tmp_path_for(path);
    if let Some(raw) = read(&legacy_tmp)? {
        if let Some(value) = parse(&raw) {
            tracing::warn!(
                path = %path.display(),
                tmp = %legacy_tmp.display(),
                "state file corrupt; recovered from interrupted-write sibling"
            );
            if let Err(e) = atomic_write_str(path, &raw) {
                tracing::warn!(path = %path.display(), error = %e, "failed to promote recovered state");
            }
            return Ok(JsonLoad::RecoveredFromTmp(value));
        }
    }

    // Nothing usable: quarantine the corrupt bytes, never delete them.
    let quarantined_to = quarantine_path_for(path);
    if let Err(e) = std::fs::rename(path, &quarantined_to) {
        tracing::error!(
            path = %path.display(),
            error = %e,
            "failed to quarantine corrupt state file; leaving it untouched"
        );
        return Ok(JsonLoad::Corrupt {
            quarantined_to: path.to_path_buf(),
        });
    }
    tracing::error!(
        path = %path.display(),
        quarantined = %quarantined_to.display(),
        "state file corrupt; moved aside (inspect it — it may contain recoverable data)"
    );
    Ok(JsonLoad::Corrupt { quarantined_to })
}

/// Timestamped quarantine path for a corrupt file.
pub fn quarantine_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".into());
    path.with_file_name(format!(
        "{file_name}.corrupt-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    ))
}

/// `true` when the load carried usable data (`Loaded` or `RecoveredFromTmp`).
impl<T> JsonLoad<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            JsonLoad::Loaded(v) | JsonLoad::RecoveredFromTmp(v) => Some(v),
            JsonLoad::Missing | JsonLoad::Corrupt { .. } => None,
        }
    }
}

// -- poison-tolerant locks ---------------------------------------------------

/// `Mutex` extension that recovers from poisoning instead of panicking.
/// A poisoned lock means another thread panicked while holding it; the data
/// is possibly inconsistent, but panicking every future caller turns one
/// subsystem failure into a whole-process freeze. We warn and continue —
/// Workbench Zero state is rebuilt from disk on restart, so liveness wins.
pub trait MutexRecover<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexRecover<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e: PoisonError<_>| {
            tracing::warn!("mutex poisoned by a panicked thread; recovering");
            e.into_inner()
        })
    }
}

/// `RwLock` extension that recovers from poisoning instead of panicking.
pub trait RwLockRecover<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockRecover<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| {
            tracing::warn!("rwlock poisoned by a panicked thread; recovering (read)");
            e.into_inner()
        })
    }
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| {
            tracing::warn!("rwlock poisoned by a panicked thread; recovering (write)");
            e.into_inner()
        })
    }
}

/// Truncate to at most `max` **bytes** on a char boundary — stable-Rust
/// replacement for `str::floor_char_boundary` (stabilized 1.91) so the
/// workspace can hold a 1.85 MSRV.
pub fn truncate_chars(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wz-common-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn truncate_chars_never_splits_utf8() {
        let s = "héllo—世界!";
        assert_eq!(truncate_chars(s, 1000), s);
        for max in 0..=s.len() {
            let t = truncate_chars(s, max);
            assert!(t.len() <= max);
            assert!(s.is_char_boundary(t.len()));
            assert!(s.starts_with(t));
        }
    }

    #[test]
    fn atomic_write_roundtrip_and_overwrite() {
        let dir = tempdir("aw");
        let path = dir.join("state.json");
        atomic_write_str(&path, r#"{"v":1}"#).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"v":1}"#);
        atomic_write_str(&path, r#"{"v":2}"#).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"v":2}"#);
        // No temp litter remains.
        let litter: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(litter.is_empty(), "tmp litter: {litter:?}");
    }

    #[test]
    fn atomic_write_missing_parent_errors_and_cleans_tmp() {
        let dir = tempdir("aw-fail");
        let path = dir.join("nope").join("state.json");
        assert!(atomic_write_str(&path, "x").is_err());
        assert!(!dir.join("nope").exists());
    }

    #[test]
    fn load_json_missing() {
        let dir = tempdir("lj-miss");
        let r: JsonLoad<serde_json::Value> = load_json(&dir.join("none.json")).unwrap();
        assert!(matches!(r, JsonLoad::Missing));
    }

    #[test]
    fn load_json_loaded() {
        let dir = tempdir("lj-ok");
        let path = dir.join("a.json");
        atomic_write_str(&path, r#"{"a":1}"#).unwrap();
        let r: JsonLoad<serde_json::Value> = load_json(&path).unwrap();
        assert_eq!(r.into_option().unwrap(), serde_json::json!({"a": 1}));
    }

    #[test]
    fn load_json_recovers_from_legacy_tmp() {
        let dir = tempdir("lj-tmp");
        let path = dir.join("s.json");
        std::fs::write(&path, "{truncated").unwrap();
        std::fs::write(path.with_extension("tmp"), r#"{"ok":true}"#).unwrap();
        let r: JsonLoad<serde_json::Value> = load_json(&path).unwrap();
        match r {
            JsonLoad::RecoveredFromTmp(v) => assert_eq!(v, serde_json::json!({"ok": true})),
            other => panic!("expected recovery, got {other:?}"),
        }
        // Promotion healed the main file.
        let r2: JsonLoad<serde_json::Value> = load_json(&path).unwrap();
        assert!(matches!(r2, JsonLoad::Loaded(_)));
    }

    #[test]
    fn load_json_quarantines_corrupt() {
        let dir = tempdir("lj-corrupt");
        let path = dir.join("s.json");
        std::fs::write(&path, "{not json").unwrap();
        let r: JsonLoad<serde_json::Value> = load_json(&path).unwrap();
        match r {
            JsonLoad::Corrupt { quarantined_to } => {
                assert!(quarantined_to.exists());
                assert_eq!(
                    std::fs::read_to_string(&quarantined_to).unwrap(),
                    "{not json"
                );
                assert!(!path.exists());
            }
            other => panic!("expected quarantine, got {other:?}"),
        }
    }

    #[test]
    fn locks_recover_from_poisoning() {
        use std::sync::Arc;
        let m = Arc::new(Mutex::new(1u32));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("boom");
        })
        .join();
        *m.lock_or_recover() = 2;
        assert_eq!(*m.lock_or_recover(), 2);

        let rw = Arc::new(std::sync::RwLock::new(1u32));
        let rw2 = rw.clone();
        let _ = std::thread::spawn(move || {
            let _g = rw2.write().unwrap();
            panic!("boom");
        })
        .join();
        *rw.write_or_recover() = 3;
        assert_eq!(*rw.read_or_recover(), 3);
    }
}
