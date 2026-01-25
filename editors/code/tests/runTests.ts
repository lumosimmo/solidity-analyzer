import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { runTests } from "@vscode/test-electron";

async function main() {
    const runFlag = process.env.RUN_VSCODE_TESTS ?? "1";
    if (runFlag !== "1") {
        console.log("RUN_VSCODE_TESTS != 1. Skipping VS Code integration tests.");
        return;
    }

    const extensionDevelopmentPath = path.resolve(__dirname, "../");
    const jsonData = fs.readFileSync(path.join(extensionDevelopmentPath, "package.json"), "utf8");
    const manifest = JSON.parse(jsonData) as { engines?: { vscode?: string } };
    const minimalVersion = resolveVscodeEngine(manifest.engines?.vscode);

    const extensionTestsPath = path.resolve(extensionDevelopmentPath, "out/tests/integration/index.js");
    if (!fs.existsSync(extensionTestsPath)) {
        throw new Error("Integration tests are missing. Run `bun run build-tests` first.");
    }

    const baseTempDir = process.platform === "darwin" ? "/tmp" : os.tmpdir();
    const testProfileRoot = path.join(baseTempDir, "solidity-analyzer-vscode-test");
    fs.mkdirSync(testProfileRoot, { recursive: true });

    const launchArgs = [
        `--user-data-dir=${path.join(testProfileRoot, "user-data")}`,
        `--extensions-dir=${path.join(testProfileRoot, "extensions")}`,
    ];

    const previousElectronRunAsNode = process.env.ELECTRON_RUN_AS_NODE;
    delete process.env.ELECTRON_RUN_AS_NODE;

    try {
        await runTests({
            version: minimalVersion,
            launchArgs,
            extensionDevelopmentPath,
            extensionTestsPath,
        });

        if (minimalVersion !== "stable") {
            await runTests({
                version: "stable",
                launchArgs,
                extensionDevelopmentPath,
                extensionTestsPath,
            });
        }
    } finally {
        if (previousElectronRunAsNode !== undefined) {
            process.env.ELECTRON_RUN_AS_NODE = previousElectronRunAsNode;
        }
    }
}

function resolveVscodeEngine(rawVersion: string | undefined): string {
    if (!rawVersion) {
        return "stable";
    }

    const trimmed = rawVersion.trim();
    if (trimmed === "stable" || trimmed === "insiders") {
        return trimmed;
    }

    const normalized = trimmed.replace(/^[~^]/, "");
    if (normalized === "stable" || normalized === "insiders") {
        return normalized;
    }

    if (/[><=*| ]/.test(normalized) || normalized.includes("-") || normalized.toLowerCase().includes("x")) {
        return "stable";
    }

    return /^\d+\.\d+\.\d+$/.test(normalized) ? normalized : "stable";
}

main().catch((error) => {
    console.error("Failed to run VS Code integration tests", error);
    process.exit(1);
});
