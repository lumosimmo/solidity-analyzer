import { describe, expect, test } from "bun:test";
import { loadManifest } from "./helpers";

interface ManifestCommand {
    command: string;
}

interface ManifestMenuEntry {
    command: string;
    when?: string;
}

const manifest = loadManifest();
const commandIds = [
    "solidity-analyzer.formatDocument",
    "solidity-analyzer.formatSelection",
    "solidity-analyzer.runLint",
    "solidity-analyzer.fixAllLints",
];

describe("format/lint commands", () => {
    test("command ids are registered in the manifest", () => {
        const commands = Array.isArray(manifest.contributes?.commands)
            ? (manifest.contributes.commands as ManifestCommand[])
            : [];

        for (const commandId of commandIds) {
            expect(commands.some((entry) => entry.command === commandId)).toBe(true);
        }
    });

    test("menus gate commands to Solidity files", () => {
        const menus = manifest.contributes?.menus ?? {};
        const commandPalette = Array.isArray(menus.commandPalette) ? (menus.commandPalette as ManifestMenuEntry[]) : [];
        const contextMenu = Array.isArray(menus["editor/context"])
            ? (menus["editor/context"] as ManifestMenuEntry[])
            : [];

        for (const commandId of commandIds) {
            const paletteEntry = commandPalette.find((entry) => entry.command === commandId);
            const contextEntry = contextMenu.find((entry) => entry.command === commandId);
            expect(paletteEntry?.when).toBe("editorLangId == solidity");
            expect(contextEntry?.when).toBe("editorLangId == solidity");
        }
    });
});
