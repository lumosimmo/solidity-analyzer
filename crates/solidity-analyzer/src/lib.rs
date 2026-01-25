use tracing_subscriber::EnvFilter;

mod config;
mod diagnostics;
mod document;
mod handlers;
mod indexer;
pub mod lsp_ext;
mod lsp_utils;
mod profile;
mod server;
mod state;
mod status;
mod task_pool;
mod workspace;

pub use server::Server;

pub fn init_tracing() {
    // Called in normal startup; Server::new also calls for test/harness coverage.
    // init_from_env is idempotent, so this is safe to repeat.
    profile::init_from_env();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::init_tracing;

    #[test]
    fn init_tracing_is_idempotent() {
        init_tracing();
        init_tracing();
    }
}
