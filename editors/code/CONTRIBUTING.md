# Contributing (VS Code Extension)

This guide covers local development for the VS Code extension in `editors/code`.

## Prerequisites

- Bun 1.3.3+
- Rust toolchain (stable)
- VS Code 1.93+
- A Foundry workspace for manual testing

## Build the language server

From the repo root:

```sh
cargo build -p solidity-analyzer
```

## Link the server into the extension

From `editors/code/`:

```sh
bun run dev:link-server
```

This links `editors/code/server/solidity-analyzer` to the debug binary. Alternatively, set
`solidity-analyzer.server.path` in VS Code settings or export `__SA_LSP_SERVER_DEBUG` in the
Extension Host environment.

## Build and run the extension

From `editors/code/`:

```sh
bun install
bun run watch
```

Use `bun run build` for a one-off build.

Open `editors/code` in VS Code and press `F5` to launch the Extension Development Host. In the dev
host, open a Foundry workspace to exercise the server.

## Debugging

- Use `solidity-analyzer: Open Logs` to view the output and trace channels.
- The status bar reflects server state; click behavior is controlled by
  `solidity-analyzer.statusBar.clickAction`.

## Tests

```sh
bun test tests/unit
```

Integration tests:

```sh
bun run build-tests
bun tests/runTests.ts
```

## Packaging a VSIX

```sh
bun run package
```

For a VSIX with the bundled server binary:

```sh
cargo run -p xtask -- ext
```
