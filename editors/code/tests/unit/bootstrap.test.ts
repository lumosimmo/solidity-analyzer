import { describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { resolveServerPath } from "../../src/bootstrap";
import { normalizeConfig } from "../../src/config";

describe("bootstrap", () => {
    test("uses configured server.path when set", () => {
        const config = normalizeConfig({ server: { path: "/tmp/sa" } });
        const resolved = resolveServerPath(config, undefined, { __SA_LSP_SERVER_DEBUG: "/tmp/debug" });
        expect(resolved).toBe("/tmp/sa");
    });

    test("uses __SA_LSP_SERVER_DEBUG when path is unset", () => {
        const config = normalizeConfig();
        const resolved = resolveServerPath(config, undefined, { __SA_LSP_SERVER_DEBUG: "/tmp/debug" });
        expect(resolved).toBe("/tmp/debug");
    });

    test("uses bundled binary when present", () => {
        const config = normalizeConfig();
        const extensionPath = mkdtempSync(join(tmpdir(), "sa-ext-"));
        const serverDir = join(extensionPath, "server");
        mkdirSync(serverDir, { recursive: true });
        const bundled = join(serverDir, "solidity-analyzer");
        writeFileSync(bundled, "");

        const resolved = resolveServerPath(config, extensionPath, {});
        expect(resolved).toBe(bundled);
    });

    test("falls back to PATH when bundled binary is missing", () => {
        const config = normalizeConfig();
        const extensionPath = mkdtempSync(join(tmpdir(), "sa-ext-"));
        const resolved = resolveServerPath(config, extensionPath, {});
        expect(resolved).toBe("solidity-analyzer");
    });

    test("uses .exe suffix for Windows bundled binaries", () => {
        const config = normalizeConfig();
        const extensionPath = mkdtempSync(join(tmpdir(), "sa-ext-"));
        const serverDir = join(extensionPath, "server");
        mkdirSync(serverDir, { recursive: true });
        const bundled = join(serverDir, "solidity-analyzer.exe");
        writeFileSync(bundled, "");

        const resolved = resolveServerPath(config, extensionPath, {}, "win32");
        expect(resolved).toBe(bundled);
    });
});
