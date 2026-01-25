import { describe, expect, test } from "bun:test";
import { COMMANDS } from "../../src/commandIds";
import { loadManifest } from "./helpers";

interface ManifestCommand {
    command: string;
    category?: string;
}

const manifest = loadManifest();

describe("command surface", () => {
    test("all command ids exist in manifest with category metadata", () => {
        const manifestCommands = Array.isArray(manifest.contributes?.commands)
            ? (manifest.contributes.commands as ManifestCommand[])
            : [];

        for (const commandId of Object.values(COMMANDS)) {
            const command = manifestCommands.find((entry) => entry.command === commandId);
            expect(command).toBeDefined();
            expect(command?.category).toBe("solidity-analyzer");
        }
    });
});
