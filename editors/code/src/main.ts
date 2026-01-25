import * as vscode from "vscode";
import { registerCommands } from "./commands";
import { loadConfig } from "./config";
import { Ctx } from "./ctx";

let ctx: Ctx | null = null;
let commandDisposable: vscode.Disposable | null = null;

export async function activate(context: vscode.ExtensionContext) {
    try {
        const config = await loadConfig();
        ctx = new Ctx(context);

        if (!config.initializeStopped) {
            await ctx.start(config);
        }

        commandDisposable = registerCommands(ctx, context);
        context.subscriptions.push(
            vscode.workspace.onDidChangeConfiguration(async (event) => {
                if (!ctx || !event.affectsConfiguration("solidity-analyzer")) {
                    return;
                }
                try {
                    const updatedConfig = await loadConfig();
                    await ctx.updateConfig(updatedConfig);
                } catch (error) {
                    const message = error instanceof Error ? error.message : String(error);
                    console.error("solidity-analyzer: failed to update configuration", error);
                    vscode.window.showErrorMessage(`solidity-analyzer: Failed to update configuration. ${message}`);
                }
            }),
        );
    } catch (error) {
        commandDisposable?.dispose();
        commandDisposable = null;
        if (ctx) {
            try {
                await ctx.dispose();
            } catch (disposeError) {
                console.error("solidity-analyzer: cleanup failed", disposeError);
            }
            ctx = null;
        }
        const message = error instanceof Error ? error.message : String(error);
        console.error("solidity-analyzer: activation failed", error);
        vscode.window.showErrorMessage(`solidity-analyzer failed to activate: ${message}`);
        throw error;
    }
}

export async function deactivate() {
    if (!ctx) {
        return;
    }

    try {
        await ctx.dispose();
    } catch (error) {
        console.error("solidity-analyzer: shutdown failed", error);
    } finally {
        ctx = null;
    }
}

export function getOutputChannelNames(): readonly [string, string] | undefined {
    if (!ctx) {
        return undefined;
    }
    return ctx.getOutputChannelNames();
}
