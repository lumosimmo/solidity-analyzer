# solidity-analyzer

This extension provides Solidity language support for Foundry projects by running the solidity-analyzer language server.

solidity-analyzer is a Solidity language server implementation with first-class [Foundry](https://github.com/foundry-rs/foundry) support. A VS Code extension is provided for easy setup, while you can use it with any editor that supports the Language Server Protocol (LSP).

The architecture of solidity-analyzer is structured as a set of libraries for analyzing Solidity code. It's heavily inspired by [rust-analyzer](https://rust-analyzer.github.io/), the popular Rust language server.

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

## Quick start

1. Install Foundry: https://getfoundry.sh/introduction/installation/
2. Install the extension (VS Code 1.93+ required).
3. Open a Foundry workspace (contains `foundry.toml` or `remappings.txt`) in VS Code. The bundled server starts automatically.
4. If the configured `solc` is missing, accept the prompt to install it via Foundry.

## Windows Support

Windows support is currently not tested and can be buggy and incomplete. I decided to focus on Unix-like OSes (Linux and macOS) first because I don't have a Windows machine for testing. Pull requests to improve Windows support are welcome.

## Configuration

All settings live under `solidity-analyzer.*`.

Use a custom language server binary:

```json
{
    "solidity-analyzer.server.path": "/path/to/solidity-analyzer"
}
```

## License

GPL-3.0 License. See [LICENSE](LICENSE) for details.
