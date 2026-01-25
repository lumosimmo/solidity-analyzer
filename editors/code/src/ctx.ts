import * as vscode from "vscode";
import * as lc from "vscode-languageclient/node";
import { DidChangeConfigurationNotification } from "vscode-languageclient/node";
import { createServerOptions } from "./bootstrap";
import { loadConfig, normalizeConfig, prepareVSCodeConfig, type ExtensionConfig, type TraceLevel } from "./config";
import { createLanguageClient } from "./client";
import { shouldShowStatusBar, statusBarCommand } from "./statusBar";
import { OUTPUT_CHANNEL_NAMES } from "./constants";
import { SERVER_STATUS_METHOD, type ServerStatusParams } from "./lsp_ext";

type ServerState = "starting" | "running" | "stopped" | "error";

const TRACE_LEVELS: Record<TraceLevel, lc.Trace> = {
    off: lc.Trace.Off,
    messages: lc.Trace.Messages,
    verbose: lc.Trace.Verbose,
};

export class Ctx {
    private client: lc.LanguageClient | null = null;
    private config: ExtensionConfig | null = null;
    private readonly outputChannel: vscode.OutputChannel;
    private readonly traceChannel: vscode.OutputChannel;
    private readonly statusBar: vscode.StatusBarItem;
    private readonly statusBarActiveEditorListener: vscode.Disposable;
    private statusBarState: ServerState = "stopped";
    private lspStatus: ServerStatusParams | null = null;
    private readonly stateEmitter = new vscode.EventEmitter<ServerState>();
    readonly onDidChangeServerState = this.stateEmitter.event;

    constructor(private readonly context: vscode.ExtensionContext) {
        this.outputChannel = vscode.window.createOutputChannel(OUTPUT_CHANNEL_NAMES.MAIN);
        this.traceChannel = vscode.window.createOutputChannel(OUTPUT_CHANNEL_NAMES.TRACE);
        this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
        this.statusBarActiveEditorListener = vscode.window.onDidChangeActiveTextEditor((editor) => {
            this.updateStatusBarVisibility(editor);
        });
        context.subscriptions.push(
            this.outputChannel,
            this.traceChannel,
            this.statusBar,
            this.statusBarActiveEditorListener,
            this.stateEmitter,
        );
        this.updateStatusBarState("stopped");
    }

    get languageClient(): lc.LanguageClient | null {
        return this.client;
    }

    get currentConfig(): ExtensionConfig | null {
        return this.config;
    }

    get serverState(): ServerState {
        return this.statusBarState;
    }

    getOutputChannelNames(): readonly [string, string] {
        return [this.outputChannel.name, this.traceChannel.name];
    }

    private resolvedConfig(): ExtensionConfig {
        return this.config ?? normalizeConfig();
    }

    private updateStatusBarVisibility(editor?: vscode.TextEditor | null): void {
        const config = this.resolvedConfig();
        const languageId = editor?.document.languageId;
        const shouldShow = shouldShowStatusBar(config.statusBar.show, languageId);

        if (shouldShow) {
            this.statusBar.show();
        } else {
            this.statusBar.hide();
        }
    }

    private applyStatusBarConfig(): void {
        const config = this.resolvedConfig();
        this.statusBar.command = statusBarCommand(config.statusBar.clickAction);
        this.updateStatusBarVisibility(vscode.window.activeTextEditor);
    }

    private updateServerStatus(status: ServerStatusParams): void {
        this.lspStatus = status;
        this.renderStatusBar();
    }

    private renderStatusBar(): void {
        switch (this.statusBarState) {
            case "starting":
                this.statusBar.text = "$(sync~spin) solidity-analyzer";
                this.statusBar.tooltip = "solidity-analyzer is starting.";
                break;
            case "error":
                this.statusBar.text = "$(error) solidity-analyzer";
                this.statusBar.tooltip = "solidity-analyzer encountered an error.";
                break;
            case "stopped":
                this.statusBar.text = "$(circle-slash) solidity-analyzer";
                this.statusBar.tooltip = "solidity-analyzer is stopped.";
                break;
            case "running":
            default: {
                const status = this.lspStatus;
                if (!status) {
                    this.statusBar.text = "$(check) solidity-analyzer";
                    this.statusBar.tooltip = "solidity-analyzer is running.";
                    break;
                }

                const message = status.message && status.message.length > 0 ? status.message : "OK";
                const icon =
                    status.health === "error"
                        ? "$(error)"
                        : status.health === "warning"
                          ? "$(warning)"
                          : status.quiescent
                            ? "$(check)"
                            : "$(sync~spin)";
                this.statusBar.text = `${icon} solidity-analyzer: ${message}`;
                this.statusBar.tooltip = `solidity-analyzer: ${message}`;
                break;
            }
        }

        this.applyStatusBarConfig();
    }

    private updateStatusBarState(state: ServerState): void {
        this.statusBarState = state;
        if (state !== "running") {
            this.lspStatus = null;
        }
        this.renderStatusBar();
        this.stateEmitter.fire(state);
    }

    async updateConfig(config: ExtensionConfig): Promise<void> {
        this.config = config;
        this.applyStatusBarConfig();

        if (this.client) {
            await this.client.setTrace(TRACE_LEVELS[config.trace.server]);
            this.client.sendNotification(DidChangeConfigurationNotification.type, {
                settings: prepareVSCodeConfig(config),
            });
        }
    }

    async start(configOverride?: ExtensionConfig): Promise<void> {
        if (this.client) {
            return;
        }

        let client: lc.LanguageClient | null = null;
        let stateListener: vscode.Disposable | null = null;
        let statusListener: vscode.Disposable | null = null;

        try {
            const config = configOverride ?? this.config ?? (await loadConfig());
            await this.updateConfig(config);
            this.updateStatusBarState("starting");
            const serverOptions = createServerOptions(config, this.context.extensionPath);
            client = await createLanguageClient(config, serverOptions, this.outputChannel, this.traceChannel);

            stateListener = client.onDidChangeState((event) => {
                switch (event.newState) {
                    case lc.State.Starting:
                        this.updateStatusBarState("starting");
                        break;
                    case lc.State.Running:
                        this.updateStatusBarState("running");
                        break;
                    case lc.State.Stopped:
                        this.updateStatusBarState("stopped");
                        break;
                }
            });
            statusListener = client.onNotification(SERVER_STATUS_METHOD, (params) => {
                this.updateServerStatus(params);
            });

            await client.start();
            // Register client and listener before setTrace to ensure cleanup on failure
            this.client = client;
            if (!stateListener || !statusListener) {
                throw new Error("failed to attach server status listeners");
            }
            this.context.subscriptions.push(client, stateListener, statusListener);
            await client.setTrace(TRACE_LEVELS[config.trace.server]);
            this.outputChannel.appendLine("solidity-analyzer language server started.");
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            this.outputChannel.appendLine(`Failed to start solidity-analyzer language server: ${message}`);
            if (error instanceof Error && error.stack) {
                this.outputChannel.appendLine(error.stack);
            }

            // Clean up stateListener if created but not yet registered
            if (stateListener && !this.client) {
                stateListener.dispose();
            }
            if (statusListener && !this.client) {
                statusListener.dispose();
            }

            // Try to stop client if started but not yet registered
            if (client && !this.client) {
                try {
                    await client.stop();
                } catch {
                    // Ignore stop errors during cleanup
                }
            }

            this.client = null;
            this.updateStatusBarState("error");
            throw error;
        }
    }

    async stop(): Promise<void> {
        if (!this.client) {
            this.updateStatusBarState("stopped");
            return;
        }

        const client = this.client;
        this.client = null;

        try {
            await client.stop();
            this.outputChannel.appendLine("solidity-analyzer language server stopped.");
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            this.outputChannel.appendLine(`Error while stopping solidity-analyzer language server: ${message}`);
            if (error instanceof Error && error.stack) {
                this.outputChannel.appendLine(error.stack);
            }
        } finally {
            this.updateStatusBarState("stopped");
        }
    }

    async restart(): Promise<void> {
        await this.stop();
        await this.start();
    }

    showLogs(): void {
        this.outputChannel.show(true);
        this.traceChannel.show(true);
    }

    async dispose(): Promise<void> {
        try {
            await this.stop();
        } finally {
            this.outputChannel.dispose();
            this.traceChannel.dispose();
        }
    }
}
