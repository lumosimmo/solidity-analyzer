import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadManifest } from "./helpers";

const manifest = loadManifest();
const root = fileURLToPath(new URL("../../", import.meta.url));

describe("extension manifest", () => {
    test("main entry points at out/main", () => {
        expect(manifest.main).toBe("./out/main");
    });

    test("build and test scripts exist", () => {
        expect(typeof manifest.scripts?.build).toBe("string");
        expect(typeof manifest.scripts?.test).toBe("string");
    });

    test("activation events include Foundry and Solidity signals", () => {
        const activationEvents = manifest.activationEvents ?? [];
        expect(activationEvents).toContain("workspaceContains:foundry.toml");
        expect(activationEvents).toContain("workspaceContains:*/foundry.toml");
        expect(activationEvents).toContain("onLanguage:solidity");
    });

    test("release metadata is defined", () => {
        const repository = manifest.repository as { type?: unknown; url?: unknown } | undefined;
        expect(repository).toBeDefined();
        expect(typeof repository?.type).toBe("string");
        expect(typeof repository?.url).toBe("string");
        expect(typeof manifest.homepage).toBe("string");
        expect(typeof manifest.license).toBe("string");
        expect(typeof manifest.icon).toBe("string");
        expect(Array.isArray(manifest.categories)).toBe(true);
    });

    test("untrusted workspace capability is declared", () => {
        const capabilities = manifest.capabilities as { untrustedWorkspaces?: unknown } | undefined;
        expect(capabilities?.untrustedWorkspaces).toBeDefined();
    });

    test("format and lint settings are exposed", () => {
        const properties = manifest.contributes?.configuration?.properties ?? {};
        const keys = [
            "solidity-analyzer.format.enable",
            "solidity-analyzer.format.onSave",
            "solidity-analyzer.lint.enable",
            "solidity-analyzer.lint.onSave",
            "solidity-analyzer.lint.fixOnSave",
        ];

        for (const key of keys) {
            const entry = properties[key] as { default?: unknown } | undefined;
            expect(entry).toBeDefined();
            expect(typeof entry?.default).toBe("boolean");
        }
    });

    test("diagnostics settings are exposed", () => {
        const properties = manifest.contributes?.configuration?.properties ?? {};
        const keys = [
            "solidity-analyzer.diagnostics.enable",
            "solidity-analyzer.diagnostics.onSave",
            "solidity-analyzer.diagnostics.onChange",
        ];

        for (const key of keys) {
            const entry = properties[key] as { default?: unknown } | undefined;
            expect(entry).toBeDefined();
            expect(typeof entry?.default).toBe("boolean");
        }
    });

    test("toolchain settings are exposed", () => {
        const properties = manifest.contributes?.configuration?.properties ?? {};
        const entry = properties["solidity-analyzer.toolchain.promptInstall"] as { default?: unknown } | undefined;
        expect(entry).toBeDefined();
        expect(typeof entry?.default).toBe("boolean");
    });

    test(".vscodeignore keeps bundled server binaries", () => {
        const ignorePath = join(root, ".vscodeignore");
        expect(existsSync(ignorePath)).toBe(true);
        const contents = readFileSync(ignorePath, "utf8");
        expect(contents).toContain("!server/**");
    });

    test("manifest does not restrict packaged files", () => {
        const files = (manifest as { files?: unknown }).files;
        expect(files).toBeUndefined();
    });

    test("README explains bundled server distribution", () => {
        const readmePath = join(root, "README.md");
        expect(existsSync(readmePath)).toBe(true);
        const contents = readFileSync(readmePath, "utf8");
        const lower = contents.toLowerCase();
        expect(lower.includes("bundled") || lower.includes("included")).toBe(true);
        expect(lower).not.toContain("install the language server");
        expect(lower).not.toContain("install solidity-analyzer");
    });
});
