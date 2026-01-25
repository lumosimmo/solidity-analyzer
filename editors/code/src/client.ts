import type * as vscode from "vscode";
import type {
    DocumentSelector,
    ExecuteCommandMiddleware,
    FeatureState,
    LanguageClient,
    LanguageClientOptions,
    ServerCapabilities,
    ServerOptions,
    StaticFeature,
} from "vscode-languageclient/node";
import { SERVER_COMMANDS } from "./commandIds";
import { prepareVSCodeConfig, type ExtensionConfig } from "./config";
import { EXPERIMENTAL_CAPABILITIES, applyExperimentalCapabilities, type ExperimentalCapabilities } from "./lsp_ext";

const CLIENT_ID = "solidity-analyzer";
const CLIENT_NAME = "solidity-analyzer";

export type ClientOptions = LanguageClientOptions & {
    experimental?: ExperimentalCapabilities;
};

export type ExecuteCommandProgress = {
    withProgress<T>(title: string, task: () => Thenable<T>): Thenable<T>;
    showInformationMessage(message: string): Thenable<void>;
    showErrorMessage(message: string): Thenable<void>;
};

export type ExecuteCommandDependencies = {
    installCommandId: string;
    ensureStarted: () => Promise<void>;
    progress: ExecuteCommandProgress;
    log: (message: string, error?: unknown) => void;
};

export function createExecuteCommandMiddleware(deps: ExecuteCommandDependencies): ExecuteCommandMiddleware {
    return {
        async executeCommand(command, args, next) {
            if (command !== deps.installCommandId) {
                return next(command, args);
            }

            try {
                await deps.ensureStarted();
                const result = await deps.progress.withProgress("solidity-analyzer: Installing solc", () =>
                    next(command, args),
                );
                if (typeof result === "string" && result.length > 0) {
                    await deps.progress.showInformationMessage(result);
                }
                return result;
            } catch (error) {
                const details = error instanceof Error ? error.message : String(error);
                deps.log(`solidity-analyzer: Failed to install solc`, error);
                await deps.progress.showErrorMessage(`solidity-analyzer: Failed to install solc. ${details}`);
                return undefined;
            }
        },
    };
}

function experimentalCapabilitiesFeature(experimental: ExperimentalCapabilities): StaticFeature {
    return {
        getState(): FeatureState {
            return { kind: "static" };
        },
        fillClientCapabilities(capabilities) {
            applyExperimentalCapabilities(capabilities, experimental);
        },
        initialize(capabilities: ServerCapabilities, documentSelector: DocumentSelector | undefined): void {
            void capabilities;
            void documentSelector;
        },
        clear(): void {},
    };
}

export function clientOptions(config: ExtensionConfig, middleware?: ExecuteCommandMiddleware): ClientOptions {
    return {
        documentSelector: [
            { language: "solidity", scheme: "file" },
            { language: "solidity", scheme: "untitled" },
        ],
        initializationOptions: prepareVSCodeConfig(config),
        experimental: { ...EXPERIMENTAL_CAPABILITIES },
        middleware,
    };
}

export async function createLanguageClient(
    config: ExtensionConfig,
    serverOptions: ServerOptions,
    outputChannel: vscode.OutputChannel,
    traceOutputChannel: vscode.OutputChannel,
): Promise<LanguageClient> {
    const lc = await import("vscode-languageclient/node");
    const vscode = await import("vscode");
    let client: LanguageClient | null = null;
    const ensureStarted = async () => {
        if (!client) {
            throw new Error("language client is not initialized");
        }
        await client.start();
    };
    const progress: ExecuteCommandProgress = {
        withProgress: (title, task) =>
            vscode.window.withProgress(
                {
                    location: vscode.ProgressLocation.Notification,
                    title,
                    cancellable: false,
                },
                () => task(),
            ),
        showInformationMessage: async (message) => {
            await vscode.window.showInformationMessage(message);
        },
        showErrorMessage: async (message) => {
            await vscode.window.showErrorMessage(message);
        },
    };
    const middleware = createExecuteCommandMiddleware({
        installCommandId: SERVER_COMMANDS.installFoundrySolc,
        ensureStarted,
        progress,
        log: (message, error) => {
            outputChannel.appendLine(message);
            if (error) {
                outputChannel.appendLine(error instanceof Error ? (error.stack ?? error.message) : String(error));
            }
        },
    });
    const options = clientOptions(config, middleware);
    client = new lc.LanguageClient(CLIENT_ID, CLIENT_NAME, serverOptions, {
        ...options,
        outputChannel,
        traceOutputChannel,
    });
    const experimental = options.experimental ?? EXPERIMENTAL_CAPABILITIES;
    client.registerFeature(experimentalCapabilitiesFeature(experimental));
    return client;
}
