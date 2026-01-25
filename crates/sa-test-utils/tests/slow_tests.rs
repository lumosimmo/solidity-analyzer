use std::env;
use std::ffi::OsString;
use std::sync::MutexGuard;

#[test]
fn slow_tests_are_skipped_without_env() {
    let _guard = EnvGuard::new(sa_test_utils::env_lock());
    unsafe {
        set_slow_env(&_guard._lock, None, None);
    }

    assert!(sa_test_utils::skip_slow_tests());
}

#[test]
fn slow_tests_enabled_with_run_slow_tests() {
    let _guard = EnvGuard::new(sa_test_utils::env_lock());
    unsafe {
        set_slow_env(&_guard._lock, Some("1"), None);
    }

    assert!(!sa_test_utils::skip_slow_tests());
}

#[test]
fn slow_tests_skipped_with_skip_slow_tests() {
    let _guard = EnvGuard::new(sa_test_utils::env_lock());
    unsafe {
        set_slow_env(&_guard._lock, None, Some("1"));
    }

    assert!(sa_test_utils::skip_slow_tests());
}

#[test]
fn skip_slow_tests_overrides_run_slow_tests() {
    let _guard = EnvGuard::new(sa_test_utils::env_lock());
    unsafe {
        set_slow_env(&_guard._lock, Some("1"), Some("1"));
    }

    assert!(sa_test_utils::skip_slow_tests());
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    run_slow: Option<OsString>,
    skip_slow: Option<OsString>,
}

impl EnvGuard {
    fn new(_lock: MutexGuard<'static, ()>) -> Self {
        Self {
            _lock,
            run_slow: env::var_os("RUN_SLOW_TESTS"),
            skip_slow: env::var_os("SKIP_SLOW_TESTS"),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            restore_env(&self._lock, "RUN_SLOW_TESTS", self.run_slow.take());
            restore_env(&self._lock, "SKIP_SLOW_TESTS", self.skip_slow.take());
        }
    }
}

/// # Safety
/// Callers must hold the `MutexGuard` returned by `env_lock()` and ensure no
/// other threads or libraries access or mutate these environment variables
/// concurrently while this function runs. The unsafe contract assumes global
/// serialization; violating these invariants can cause data races and
/// unpredictable behavior for these helpers.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn restore_env(_lock: &MutexGuard<'static, ()>, key: &str, value: Option<OsString>) {
    match value {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}

/// # Safety
/// Callers must hold the `MutexGuard` returned by `env_lock()` and ensure no
/// other threads or libraries access or mutate these environment variables
/// concurrently while this function runs. The unsafe contract assumes global
/// serialization; violating these invariants can cause data races and
/// unpredictable behavior for these helpers.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn set_slow_env(
    _lock: &MutexGuard<'static, ()>,
    run_value: Option<&str>,
    skip_value: Option<&str>,
) {
    match run_value {
        Some(value) => env::set_var("RUN_SLOW_TESTS", value),
        None => env::remove_var("RUN_SLOW_TESTS"),
    }
    match skip_value {
        Some(value) => env::set_var("SKIP_SLOW_TESTS", value),
        None => env::remove_var("SKIP_SLOW_TESTS"),
    }
}
