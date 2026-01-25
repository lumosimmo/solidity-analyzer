import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));

describe("packaging scripts", () => {
    test("platform packaging helpers exist", () => {
        expect(existsSync(join(root, "scripts/package-linux.sh"))).toBe(true);
        expect(existsSync(join(root, "scripts/package-macos.sh"))).toBe(true);
        expect(existsSync(join(root, "scripts/package-windows.ps1"))).toBe(true);
    });
});
