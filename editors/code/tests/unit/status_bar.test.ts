import { describe, expect, test } from "bun:test";
import { COMMANDS } from "../../src/commandIds";
import { normalizeConfig } from "../../src/config";
import { shouldShowStatusBar, statusBarCommand } from "../../src/statusBar";

describe("status bar", () => {
    test("defaults are configured", () => {
        const config = normalizeConfig();
        expect(config.statusBar.show).toBe("whenActive");
        expect(config.statusBar.clickAction).toBe("openLogs");
    });

    test("visibility settings respect the active language", () => {
        expect(shouldShowStatusBar("always", undefined)).toBe(true);
        expect(shouldShowStatusBar("never", "solidity")).toBe(false);
        expect(shouldShowStatusBar("whenActive", "solidity")).toBe(true);
        expect(shouldShowStatusBar("whenActive", "rust")).toBe(false);
    });

    test("click action maps to commands", () => {
        expect(statusBarCommand("openLogs")).toBe(COMMANDS.openLogs);
        expect(statusBarCommand("restartServer")).toBe(COMMANDS.restartServer);
    });
});
