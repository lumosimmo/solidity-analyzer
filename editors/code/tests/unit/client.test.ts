import { describe, expect, test } from "bun:test";
import { clientOptions, createExecuteCommandMiddleware } from "../../src/client";
import { SERVER_COMMANDS } from "../../src/commandIds";
import { normalizeConfig, prepareVSCodeConfig } from "../../src/config";

describe("client options", () => {
    test("document selector includes solidity files", () => {
        const config = normalizeConfig();
        const options = clientOptions(config);
        const selector = options.documentSelector ?? [];

        const hasFileSolidity = selector.some((item) => item.language === "solidity" && item.scheme === "file");
        const hasUntitledSolidity = selector.some((item) => item.language === "solidity" && item.scheme === "untitled");

        expect(hasFileSolidity).toBe(true);
        expect(hasUntitledSolidity).toBe(true);
    });

    test("initialization options mirror the config", () => {
        const config = normalizeConfig({
            server: { path: "/bin/sa", extraEnv: { SA_LOG: "trace" } },
            trace: { server: "messages" },
            initializeStopped: false,
        });

        const options = clientOptions(config);
        expect(options.initializationOptions).toEqual(prepareVSCodeConfig(config));
    });

    test("executeCommand middleware wraps install in progress", async () => {
        let started = false;
        let progressUsed = false;
        let infoMessage = "";

        const middleware = createExecuteCommandMiddleware({
            installCommandId: SERVER_COMMANDS.installFoundrySolc,
            ensureStarted: async () => {
                started = true;
            },
            progress: {
                withProgress: async (_title, task) => {
                    progressUsed = true;
                    return task();
                },
                showInformationMessage: async (message) => {
                    infoMessage = message;
                },
                showErrorMessage: async () => {},
            },
            log: () => {},
        });

        const result = await middleware.executeCommand?.(
            SERVER_COMMANDS.installFoundrySolc,
            [],
            async () => "installed solc",
        );

        expect(started).toBe(true);
        expect(progressUsed).toBe(true);
        expect(infoMessage).toBe("installed solc");
        expect(result).toBe("installed solc");
    });

    test("client options accept executeCommand middleware", () => {
        const middleware = createExecuteCommandMiddleware({
            installCommandId: SERVER_COMMANDS.installFoundrySolc,
            ensureStarted: async () => {},
            progress: {
                withProgress: async (_title, task) => task(),
                showInformationMessage: async () => {},
                showErrorMessage: async () => {},
            },
            log: () => {},
        });
        const options = clientOptions(normalizeConfig(), middleware);

        expect(options.middleware).toBe(middleware);
    });
});
