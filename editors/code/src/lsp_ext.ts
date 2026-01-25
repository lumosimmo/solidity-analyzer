import type { ClientCapabilities } from "vscode-languageclient/node";

export type ExperimentalCapabilities = {
    snippetTextEdit: boolean;
    codeActionGroup: boolean;
    serverStatusNotification: boolean;
};

export const EXPERIMENTAL_CAPABILITIES: ExperimentalCapabilities = {
    snippetTextEdit: true,
    codeActionGroup: true,
    serverStatusNotification: true,
};

export function applyExperimentalCapabilities(
    capabilities: ClientCapabilities,
    overrides: Partial<ExperimentalCapabilities> = {},
): void {
    const existing =
        typeof capabilities.experimental === "object" && capabilities.experimental !== null
            ? capabilities.experimental
            : {};

    capabilities.experimental = {
        ...EXPERIMENTAL_CAPABILITIES,
        ...existing,
        ...overrides,
    };
}

export type ServerStatusParams = {
    health: "ok" | "warning" | "error";
    quiescent: boolean;
    message?: string;
};

export const SERVER_STATUS_METHOD = "experimental/serverStatus";
