import { describe, expect, test } from "bun:test";
import { COMMANDS } from "../../src/commandIds";
import { loadManifest } from "./helpers";

const manifest = loadManifest();
const expectedCommands = Object.values(COMMANDS);

describe("commands manifest", () => {
    test("command ids are registered", () => {
        const commands = (manifest.contributes?.commands ?? []).map((command: { command: string }) => command.command);

        for (const commandId of expectedCommands) {
            expect(commands).toContain(commandId);
        }
    });
});
