# solidity-analyzer

![Open VSX Version](https://img.shields.io/open-vsx/v/lumosimmo/solidity-analyzer)

solidity-analyzer is a Solidity language server implementation with first-class [Foundry](https://github.com/foundry-rs/foundry) support. A VS Code extension is provided for easy setup, while you can use it with any editor that supports the Language Server Protocol (LSP).

The architecture of solidity-analyzer is structured as a set of libraries for analyzing Solidity code. It's heavily inspired by [rust-analyzer](https://rust-analyzer.github.io/), the popular Rust language server.

## ‼️ This extension is currently only published on Open VSX, NOT the VS Code Marketplace. If you find a published version on the VS Code Marketplace, it is a scam.

## Features

solidity-analyzer provides IDE features for Solidity development, including:

- diagnostics for real-time error and warning reporting
- go to definition, references, and renaming
- code completion
- hover and signature help
- document and workspace symbols
- formatting and linting
- code actions for quick fixes
- workspace awareness for Foundry projects

## Manual VS Code Extension Install

To build and install the extension manually from source, follow these steps:

```bash
git clone https://github.com/lumosimmo/solidity-analyzer.git
cd solidity-analyzer
cargo run -p xtask -- ext
code --install-extension editors/code/solidity-analyzer.vsix
```

If `code` is not available in your shell, use VS Code: `Extensions` -> `...` -> `Install from VSIX...` and select `editors/code/solidity-analyzer.vsix`.

## Windows Support

Windows support is currently not tested and can be buggy and incomplete. I decided to focus on Unix-like OSes (Linux and macOS) first because I don't have a Windows machine for testing. Pull requests to improve Windows support are welcome.

## License

GPL-3.0 License. See [LICENSE](LICENSE) for details.
