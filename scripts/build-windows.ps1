$ErrorActionPreference = "Stop"

cargo build --workspace
cargo test --workspace
