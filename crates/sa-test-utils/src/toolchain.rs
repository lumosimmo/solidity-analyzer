use std::env;
use std::path::{Path, PathBuf};

pub use sa_test_support::StubSolcOptions;
use sa_test_support::write_stub_solc_with_options as write_stub_solc_with_options_support;

#[derive(Clone, Copy, Debug)]
pub enum SolcTestMode {
    Auto,
    Stub,
    Real,
}

pub fn stub_solc_output_empty() -> &'static str {
    include_str!("../test_data/solc/solc-output-empty.json")
}

pub fn stub_solc_output_error() -> &'static str {
    include_str!("../test_data/solc/solc-output-error.json")
}

pub fn solc_path_for_tests(dir: &Path, version: &str, options: StubSolcOptions<'_>) -> PathBuf {
    solc_path_for_tests_with_mode(dir, version, options, SolcTestMode::Auto)
}

pub fn solc_path_for_tests_with_mode(
    dir: &Path,
    version: &str,
    options: StubSolcOptions<'_>,
    mode: SolcTestMode,
) -> PathBuf {
    match mode {
        SolcTestMode::Stub => write_stub_solc_with_options(dir, version, options),
        SolcTestMode::Real => resolve_real_solc().unwrap_or_else(|| {
            panic!("RUN_SLOW_TESTS=1 requires solc on PATH or SOLC_PATH to be set")
        }),
        SolcTestMode::Auto => {
            if crate::slow_tests_enabled() {
                resolve_real_solc().unwrap_or_else(|| {
                    panic!("RUN_SLOW_TESTS=1 requires solc on PATH or SOLC_PATH to be set")
                })
            } else {
                write_stub_solc_with_options(dir, version, options)
            }
        }
    }
}

pub fn write_stub_solc_with_options(
    dir: &Path,
    version: &str,
    mut options: StubSolcOptions<'_>,
) -> PathBuf {
    if options.json.is_none() {
        options.json = Some(stub_solc_output_empty());
    }
    write_stub_solc_with_options_support(dir, version, options)
}

pub fn write_stub_solc(dir: &Path, version: &str, json: Option<&str>) -> PathBuf {
    write_stub_solc_with_options(
        dir,
        version,
        StubSolcOptions {
            json,
            sleep_seconds: None,
            capture_stdin: false,
        },
    )
}

pub fn stub_solc_input_path(solc_path: &Path) -> PathBuf {
    solc_path.with_file_name("solc-input.json")
}

fn resolve_real_solc() -> Option<PathBuf> {
    if let Ok(path) = env::var("SOLC_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let exe_name = if cfg!(windows) { "solc.exe" } else { "solc" };
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
