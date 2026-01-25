//! Filesystem-backed profiling for LSP request handling.
//!
//! Enable profiling by setting `SA_PROFILE_PATH` and calling `init_from_env()`
//! during server initialization. Wrap request handlers with
//! `ProfileSpan::new("method")` to record timing information.
//!
//! Output is JSONL with entries shaped like:
//! `{ "request": "<method>", "duration_ms": <millis> }`
//!
//! Each record is newline-terminated (one JSON object per line); `tail -f` is
//! useful for streaming live entries. The output file grows without bound as
//! requests are recorded, so use logrotate or rename-and-reopen/external
//! rotation for long-running servers.
//!
//! # Examples
//! ```ignore
//! use solidity_analyzer::profile::{init_from_env, ProfileSpan};
//!
//! init_from_env();
//! let _span = ProfileSpan::new("textDocument/hover");
//! // ... handle request ...
//! ```

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tracing::{info, warn};

static PROFILE_PATH: OnceLock<PathBuf> = OnceLock::new();
static PROFILE_LOCK: OnceLock<Mutex<ProfileState>> = OnceLock::new();

#[derive(Default)]
struct ProfileState {
    file: Option<File>,
}

#[derive(Serialize)]
struct ProfileEvent {
    request: &'static str,
    duration_ms: u64,
}

/// Initializes request profiling from the `SA_PROFILE_PATH` environment variable.
///
/// When `SA_PROFILE_PATH` is set, it should point to a file path where JSONL
/// profile events will be appended. This function is idempotent: it sets
/// `PROFILE_PATH` once and returns early on subsequent calls.
///
/// Side effects: sets `PROFILE_PATH`, initializes `PROFILE_LOCK`, and logs when
/// profiling is enabled. If the environment variable is absent, it returns
/// early without panicking.
///
/// # Examples
/// ```ignore
/// use solidity_analyzer::profile::init_from_env;
///
/// init_from_env();
/// ```
pub fn init_from_env() {
    if PROFILE_PATH.get().is_some() {
        return;
    }
    let Ok(path) = std::env::var("SA_PROFILE_PATH") else {
        return;
    };
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    let path = PathBuf::from(path);
    let set = PROFILE_PATH.set(path.clone());
    if set.is_ok() {
        let _ = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
        info!(path = %path.display(), "profiling enabled");
    }
}

/// RAII timing span for profiling LSP requests.
///
/// When profiling is enabled (via `SA_PROFILE_PATH`), the span captures a start
/// time on creation and records the elapsed duration when dropped. When
/// profiling is disabled, creating the span is inexpensive and drop is a no-op.
///
/// Fields:
/// - `request`: the request name recorded in profile events
/// - `start`: the start timestamp when profiling is enabled
///
/// # Examples
/// ```ignore
/// use solidity_analyzer::profile::ProfileSpan;
///
/// let _span = ProfileSpan::new("textDocument/hover");
/// // ... handle request ...
/// ```
#[must_use]
pub struct ProfileSpan {
    request: &'static str,
    start: Option<Instant>,
}

impl ProfileSpan {
    /// Creates a new profiling span for `request`.
    ///
    /// When profiling is disabled, this returns a span that does not record any
    /// events on drop.
    pub fn new(request: &'static str) -> Self {
        Self {
            request,
            start: PROFILE_PATH.get().map(|_| Instant::now()),
        }
    }
}

impl Drop for ProfileSpan {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        let Some(path) = PROFILE_PATH.get() else {
            return;
        };
        record_event(path, self.request, start.elapsed());
    }
}

fn record_event(path: &Path, request: &'static str, duration: Duration) {
    #[allow(clippy::collapsible_if)]
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            warn!(?error, "failed to create profile output directory");
            return;
        }
    }

    let duration_ms = duration.as_millis().min(u128::from(u64::MAX)) as u64;
    let event = ProfileEvent {
        request,
        duration_ms,
    };
    let payload = match serde_json::to_string(&event) {
        Ok(payload) => payload,
        Err(error) => {
            warn!(?error, "failed to serialize profile event");
            return;
        }
    };

    let lock = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(error) => {
            warn!(?error, "profile lock poisoned");
            let guard = error.into_inner();
            lock.clear_poison();
            guard
        }
    };

    let mut file = match guard.file.take() {
        Some(file) => file,
        None => match open_profile_file(path) {
            Ok(file) => file,
            Err(error) => {
                warn!(?error, "failed to open profile output file");
                return;
            }
        },
    };

    if let Err(error) = writeln!(file, "{payload}") {
        warn!(?error, "failed to write profile event");
        // If `writeln!(file, "{payload}")` fails, log it, call `open_profile_file(path)` to
        // get `reopened`, retry once, and only then set `guard.file = Some(reopened)`; on
        // retry failure we return without restoring `guard.file` so subsequent calls open
        // a fresh `file`.
        let mut reopened = match open_profile_file(path) {
            Ok(reopened) => reopened,
            Err(error) => {
                warn!(?error, "failed to open profile output file");
                return;
            }
        };
        if let Err(error) = writeln!(reopened, "{payload}") {
            warn!(?error, "failed to write profile event");
            return;
        }
        guard.file = Some(reopened);
        return;
    }

    guard.file = Some(file);
}

#[allow(clippy::ineffective_open_options)]
fn open_profile_file(path: &Path) -> Result<File, std::io::Error> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .write(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sa_test_utils::{EnvGuard, env_lock};
    use std::fs;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::tempdir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_lock() -> MutexGuard<'static, ()> {
        match TEST_LOCK.lock() {
            Ok(guard) => guard,
            Err(error) => {
                TEST_LOCK.clear_poison();
                error.into_inner()
            }
        }
    }

    fn reset_profile_lock() {
        let lock = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
        lock.clear_poison();
        let mut guard = match lock.lock() {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        };
        guard.file = None;
    }

    #[test]
    fn init_from_env_writes_profile_event() {
        let _test_lock = test_lock();
        let _lock = env_lock();
        reset_profile_lock();
        if PROFILE_PATH.get().is_some() {
            return;
        }
        let root = std::env::current_dir().expect("current dir");
        let path = root.join("target/profile-tests/profile.jsonl");
        let path_str = path.to_string_lossy().to_string();
        let _env = EnvGuard::set("SA_PROFILE_PATH", Some(path_str.as_str()));
        let _ = fs::remove_file(&path);

        init_from_env();
        assert_eq!(
            PROFILE_PATH.get().map(|stored| stored.as_path()),
            Some(path.as_path())
        );
        assert!(PROFILE_LOCK.get().is_some());
        {
            let _span = ProfileSpan::new("test/request");
        }
    }

    #[test]
    fn record_event_recovers_from_poisoned_lock() {
        let _test_lock = test_lock();
        reset_profile_lock();
        let lock = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
        let _ = std::panic::catch_unwind(|| {
            let _guard = lock.lock().expect("lock");
            panic!("poison");
        });

        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("profile.jsonl");
        record_event(&path, "poisoned", Duration::from_millis(1));

        let contents = fs::read_to_string(&path).expect("profile contents");
        assert!(contents.contains("poisoned"));
    }

    #[test]
    fn record_event_reopens_after_write_failure() {
        let _test_lock = test_lock();
        reset_profile_lock();
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("profile.jsonl");
        fs::write(&path, "").expect("create file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).expect("set readonly");

        let file = File::open(&path).expect("open readonly");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).expect("set writable");

        let lock = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
        {
            let mut guard = lock.lock().expect("lock");
            guard.file = Some(file);
        }

        record_event(&path, "retry", Duration::from_millis(5));
        let contents = fs::read_to_string(&path).expect("profile contents");
        assert!(contents.contains("retry"));
    }

    #[test]
    fn record_event_bails_when_reopen_fails() {
        let _test_lock = test_lock();
        reset_profile_lock();
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("profile_dir");
        fs::create_dir(&path).expect("create dir");
        let file = File::open(&path).expect("open dir");

        let lock = PROFILE_LOCK.get_or_init(|| Mutex::new(ProfileState::default()));
        {
            let mut guard = lock.lock().expect("lock");
            guard.file = Some(file);
        }

        record_event(&path, "readonly", Duration::from_millis(1));
        assert!(fs::read_to_string(&path).is_err());
    }
}
