import type * as lc from "vscode-languageclient/node";
import { existsSync } from "node:fs";
import { join } from "node:path";
import type { ExtensionConfig } from "./config";

const DEBUG_ENV_KEY = "__SA_LSP_SERVER_DEBUG";
const DEFAULT_SERVER_COMMAND = "solidity-analyzer";
const BUNDLED_SERVER_DIR = "server";

function resolveExecutableName(platform: NodeJS.Platform): string {
    return platform === "win32" ? `${DEFAULT_SERVER_COMMAND}.exe` : DEFAULT_SERVER_COMMAND;
}

function resolveBundledServerPath(extensionPath: string, platform: NodeJS.Platform): string {
    return join(extensionPath, BUNDLED_SERVER_DIR, resolveExecutableName(platform));
}

export function resolveServerPath(
    config: ExtensionConfig,
    extensionPath?: string,
    env: NodeJS.ProcessEnv = process.env,
    platform: NodeJS.Platform = process.platform,
): string {
    if (config.server.path) {
        return config.server.path;
    }

    const debugPath = env[DEBUG_ENV_KEY];
    if (debugPath) {
        return debugPath;
    }

    if (extensionPath) {
        const bundledPath = resolveBundledServerPath(extensionPath, platform);
        if (existsSync(bundledPath)) {
            return bundledPath;
        }
    }

    return DEFAULT_SERVER_COMMAND;
}

export function createServerOptions(
    config: ExtensionConfig,
    extensionPath: string,
    env: NodeJS.ProcessEnv = process.env,
): lc.ServerOptions {
    const command = resolveServerPath(config, extensionPath, env);

    return {
        command,
        args: [],
        options: {
            env: {
                ...env,
                ...config.server.extraEnv,
            },
        },
    };
}
