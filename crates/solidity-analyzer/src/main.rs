use tracing::{error, info};

#[tokio::main]
async fn main() {
    solidity_analyzer::init_tracing();
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let server = tokio::spawn(tower_lsp::Server::new(stdin, stdout, socket).serve(service));

    tokio::select! {
        result = server => {
            match result {
                Ok(()) => info!("lsp server exited"),
                Err(error) => error!(?error, "lsp server exited unexpectedly"),
            }
        }
        _ = shutdown_signal() => {
            info!("received shutdown signal");
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            error!(?error, "failed to listen for ctrl-c");
        }
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut term = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                error!(?error, "failed to install SIGTERM handler");
                ctrl_c.await;
                return;
            }
        };

        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await;
    }
}
