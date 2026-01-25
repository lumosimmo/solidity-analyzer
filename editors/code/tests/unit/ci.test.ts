import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("../../../../", import.meta.url));
const workflowPath = join(repoRoot, ".github", "workflows", "release.yml");

describe("release workflow", () => {
    test("release workflow exists", () => {
        expect(existsSync(workflowPath)).toBe(true);
    });

    test("release workflow triggers on version tags", () => {
        const contents = readFileSync(workflowPath, "utf8");
        expect(contents).toContain("tags:");
        expect(contents).toContain("v[0-9]+.[0-9]+.[0-9]+");
    });

    test("release workflow includes all VS Code targets", () => {
        const contents = readFileSync(workflowPath, "utf8");
        const targets = ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64", "win32-x64", "win32-arm64"];

        for (const target of targets) {
            expect(contents).toContain(`vscode_target: ${target}`);
        }
    });

    test("release workflow uploads standalone binaries", () => {
        const contents = readFileSync(workflowPath, "utf8");
        const binaries = [
            "solidity-analyzer-linux-x64",
            "solidity-analyzer-linux-arm64",
            "solidity-analyzer-darwin-x64",
            "solidity-analyzer-darwin-arm64",
            "solidity-analyzer-win32-x64.exe",
            "solidity-analyzer-win32-arm64.exe",
        ];

        for (const binary of binaries) {
            expect(contents).toContain(binary);
        }
    });
});
