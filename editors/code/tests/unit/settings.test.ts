import { describe, expect, test } from "bun:test";
import { normalizeConfig, prepareVSCodeConfig } from "../../src/config";

describe("extension settings", () => {
    test("defaults are applied", () => {
        const config = normalizeConfig();
        expect(config.server.path).toBeNull();
        expect(config.trace.server).toBe("off");
        expect(config.initializeStopped).toBe(false);
        expect(config.statusBar.show).toBe("whenActive");
        expect(config.statusBar.clickAction).toBe("openLogs");
        expect(config.toolchain.promptInstall).toBe(true);
    });

    test("environment variables are expanded in server.extraEnv", () => {
        const config = normalizeConfig(
            {
                server: {
                    extraEnv: {
                        SA_CONFIG_PATH: "${env:SA_CONFIG_PATH}",
                        SA_MODE: "debug-${env:SA_MODE}-1",
                    },
                },
            },
            { SA_CONFIG_PATH: "/tmp/sa.toml", SA_MODE: "fast" },
        );

        expect(config.server.extraEnv.SA_CONFIG_PATH).toBe("/tmp/sa.toml");
        expect(config.server.extraEnv.SA_MODE).toBe("debug-fast-1");
    });

    test("prepareVSCodeConfig returns initialization options", () => {
        const config = normalizeConfig({
            server: { path: "/bin/sa", extraEnv: { SA_LOG: "trace" } },
            trace: { server: "verbose" },
            initializeStopped: true,
            statusBar: { show: "always", clickAction: "restartServer" },
        });

        expect(prepareVSCodeConfig(config)).toEqual({
            server: { path: "/bin/sa", extraEnv: { SA_LOG: "trace" } },
            trace: { server: "verbose" },
            initializeStopped: true,
            statusBar: { show: "always", clickAction: "restartServer" },
            completion: { enable: true },
            hover: { enable: true },
            signatureHelp: { enable: true },
            rename: { enable: true },
            references: { enable: true },
            diagnostics: { enable: true, onSave: true, onChange: true },
            format: { enable: true, onSave: false },
            lint: { enable: true, onSave: true, fixOnSave: false },
            toolchain: { promptInstall: true },
        });
    });
});
