import { describe, expect, test } from "bun:test";
import { CLIENT_COMMAND_IDS, SERVER_COMMAND_IDS } from "../../src/commandIds";
import { loadManifest } from "./helpers";

const manifest = loadManifest();

describe("command ownership", () => {
    test("client command registrations exclude server-owned ids", () => {
        expect(CLIENT_COMMAND_IDS.length).toBeGreaterThan(0);
        expect(SERVER_COMMAND_IDS.length).toBeGreaterThan(0);

        for (const serverCommand of SERVER_COMMAND_IDS) {
            expect(CLIENT_COMMAND_IDS).not.toContain(serverCommand);
        }
    });

    test("server-owned commands stay in the manifest", () => {
        const commands = (manifest.contributes?.commands ?? []).map((command: { command: string }) => command.command);

        for (const serverCommand of SERVER_COMMAND_IDS) {
            expect(commands).toContain(serverCommand);
        }
    });
});
