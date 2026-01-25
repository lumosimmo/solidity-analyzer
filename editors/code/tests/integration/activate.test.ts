import * as assert from "node:assert/strict";
import * as path from "node:path";
import type * as vscodeType from "vscode";
import { EXTENSION_ID, OUTPUT_CHANNEL_NAMES } from "../../src/constants";
import type { Context } from "./index";

let vscodeApi: typeof import("vscode") | null = null;

type ExtensionExports = {
    getOutputChannelNames?: () => readonly [string, string] | undefined;
};

export async function getTests(context: Context): Promise<void> {
    const vscode = await resolveVscode();
    if (!vscode) {
        return;
    }

    await context.suite("activation", (suite) => {
        suite.addTest("activates on a Foundry workspace", async () => {
            const extension = vscode.extensions.getExtension(EXTENSION_ID);
            assert.ok(extension, "extension is available");

            const fixtureUri = vscode.Uri.file(path.join(extension.extensionPath, "tests/fixtures/foundry"));
            const workspaceFolder = await ensureWorkspaceFolder(vscode, fixtureUri);

            await extension.activate();
            assert.equal(extension.isActive, true);

            const foundryToml = path.join(workspaceFolder.uri.fsPath, "foundry.toml");
            try {
                await vscode.workspace.fs.stat(vscode.Uri.file(foundryToml));
            } catch (error) {
                const detail = error instanceof Error ? error.message : String(error);
                assert.fail(`Missing foundry.toml at ${foundryToml}: ${detail}`);
            }

            const exports = extension.exports as ExtensionExports | undefined;
            const channelNames = exports?.getOutputChannelNames?.() ?? [];
            assert.ok(channelNames.includes(OUTPUT_CHANNEL_NAMES.MAIN));
            assert.ok(channelNames.includes(OUTPUT_CHANNEL_NAMES.TRACE));
        });
    });
}

async function resolveVscode(): Promise<typeof import("vscode") | null> {
    if (vscodeApi) {
        return vscodeApi;
    }

    try {
        vscodeApi = await import("vscode");
        return vscodeApi;
    } catch {
        return null;
    }
}

async function ensureWorkspaceFolder(
    vscode: typeof import("vscode"),
    uri: vscodeType.Uri,
): Promise<vscodeType.WorkspaceFolder> {
    const existing = vscode.workspace.workspaceFolders ?? [];
    const match = existing.find((folder) => folder.uri.fsPath === uri.fsPath);
    if (match) {
        return match;
    }

    const didUpdate = vscode.workspace.updateWorkspaceFolders(existing.length, 0, { uri, name: "foundry-fixture" });
    assert.ok(
        didUpdate,
        `Failed to open workspace folder ${uri.toString()}; updateWorkspaceFolders returned ${didUpdate}`,
    );

    return await waitForWorkspaceFolder(vscode, uri);
}

async function waitForWorkspaceFolder(
    vscode: typeof import("vscode"),
    uri: vscodeType.Uri,
    timeoutMs = 10_000,
): Promise<vscodeType.WorkspaceFolder> {
    let disposable: vscodeType.Disposable | undefined;
    const existing = vscode.workspace.workspaceFolders?.find((folder) => folder.uri.fsPath === uri.fsPath);
    if (existing) {
        return existing;
    }

    return await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            disposable?.dispose();
            reject(new Error(`Workspace folder not opened: ${uri.fsPath}`));
        }, timeoutMs);
        let resolved = false;
        const resolveOnce = (folder: vscodeType.WorkspaceFolder) => {
            if (resolved) {
                return;
            }
            resolved = true;
            clearTimeout(timer);
            disposable?.dispose();
            resolve(folder);
        };

        disposable = vscode.workspace.onDidChangeWorkspaceFolders(() => {
            const folder = vscode.workspace.workspaceFolders?.find((entry) => entry.uri.fsPath === uri.fsPath);
            if (folder) {
                resolveOnce(folder);
            }
        });

        const folder = vscode.workspace.workspaceFolders?.find((entry) => entry.uri.fsPath === uri.fsPath);
        if (folder) {
            resolveOnce(folder);
        }
    });
}
