import * as vscode from "vscode";
import { CLIENT_COMMANDS, COMMANDS, SERVER_COMMANDS } from "./commandIds";
import { Ctx } from "./ctx";

export { COMMANDS };

function getErrorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

async function handleCommandError(actionLabel: string, error: unknown, ctx: Ctx): Promise<void> {
    const details = getErrorMessage(error);
    console.error(`solidity-analyzer: Failed to ${actionLabel}`, error);
    const choice = await vscode.window.showErrorMessage(
        `solidity-analyzer: Failed to ${actionLabel}. ${details}`,
        "Open Logs",
    );

    if (choice === "Open Logs") {
        ctx.showLogs();
    }
}

export function registerCommands(ctx: Ctx, context: vscode.ExtensionContext): vscode.Disposable {
    let installCommandDisposable: vscode.Disposable | null = null;

    const registerInstallCommand = () => {
        if (installCommandDisposable) {
            return;
        }
        installCommandDisposable = vscode.commands.registerCommand(
            SERVER_COMMANDS.installFoundrySolc,
            async (...args: unknown[]) => {
                try {
                    await ctx.start();
                    await vscode.commands.executeCommand(SERVER_COMMANDS.installFoundrySolc, ...args);
                } catch (error) {
                    await handleCommandError("install solc", error, ctx);
                }
            },
        );
    };

    const unregisterInstallCommand = () => {
        if (!installCommandDisposable) {
            return;
        }
        installCommandDisposable.dispose();
        installCommandDisposable = null;
    };

    const syncInstallCommand = (state: Ctx["serverState"]) => {
        if (state === "stopped" || state === "error") {
            registerInstallCommand();
        } else {
            unregisterInstallCommand();
        }
    };

    syncInstallCommand(ctx.serverState);

    const disposables = [
        ctx.onDidChangeServerState((state) => {
            syncInstallCommand(state);
        }),
        new vscode.Disposable(() => {
            unregisterInstallCommand();
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.startServer, async () => {
            try {
                await ctx.start();
            } catch (error) {
                await handleCommandError("start the server", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.stopServer, async () => {
            try {
                await ctx.stop();
            } catch (error) {
                await handleCommandError("stop the server", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.restartServer, async () => {
            try {
                await ctx.restart();
            } catch (error) {
                await handleCommandError("restart the server", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.openLogs, () => {
            ctx.showLogs();
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.rename, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.rename");
            } catch (error) {
                await handleCommandError("trigger rename", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.findReferences, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.referenceSearch.trigger");
            } catch (error) {
                await handleCommandError("find references", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.triggerSignatureHelp, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.triggerParameterHints");
            } catch (error) {
                await handleCommandError("trigger signature help", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.formatDocument, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.formatDocument");
            } catch (error) {
                await handleCommandError("format the document", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.formatSelection, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.formatSelection");
            } catch (error) {
                await handleCommandError("format the selection", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.runLint, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.codeAction", { kind: "quickfix" });
            } catch (error) {
                await handleCommandError("run lint actions", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.fixAllLints, async () => {
            try {
                await vscode.commands.executeCommand("editor.action.codeAction", {
                    kind: "source.fixAll",
                    apply: "first",
                });
            } catch (error) {
                await handleCommandError("apply lint fixes", error, ctx);
            }
        }),
        vscode.commands.registerCommand(CLIENT_COMMANDS.showIndexedFiles, async () => {
            try {
                await ctx.start();
                const result = await vscode.commands.executeCommand(SERVER_COMMANDS.indexedFiles);
                if (!Array.isArray(result)) {
                    await vscode.window.showInformationMessage("solidity-analyzer: No indexed files returned.");
                    return;
                }
                const files = result.filter((item): item is string => typeof item === "string");
                if (files.length === 0) {
                    await vscode.window.showInformationMessage("solidity-analyzer: No indexed files returned.");
                    return;
                }
                files.sort();
                const content = `Indexed files (${files.length}):\n${files.join("\n")}`;
                const doc = await vscode.workspace.openTextDocument({ content, language: "text" });
                await vscode.window.showTextDocument(doc, { preview: true });
            } catch (error) {
                await handleCommandError("show indexed files", error, ctx);
            }
        }),
    ];
    const disposable = vscode.Disposable.from(...disposables);
    context.subscriptions.push(disposable);
    return disposable;
}
