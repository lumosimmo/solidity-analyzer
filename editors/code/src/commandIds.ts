export const CLIENT_COMMANDS = {
    startServer: "solidity-analyzer.startServer",
    stopServer: "solidity-analyzer.stopServer",
    restartServer: "solidity-analyzer.restartServer",
    openLogs: "solidity-analyzer.openLogs",
    rename: "solidity-analyzer.rename",
    findReferences: "solidity-analyzer.findReferences",
    triggerSignatureHelp: "solidity-analyzer.triggerSignatureHelp",
    formatDocument: "solidity-analyzer.formatDocument",
    formatSelection: "solidity-analyzer.formatSelection",
    runLint: "solidity-analyzer.runLint",
    fixAllLints: "solidity-analyzer.fixAllLints",
    showIndexedFiles: "solidity-analyzer.showIndexedFiles",
} as const;

export const SERVER_COMMANDS = {
    installFoundrySolc: "solidity-analyzer.installFoundrySolc",
    indexedFiles: "solidity-analyzer.indexedFiles",
} as const;

export const COMMANDS = { ...CLIENT_COMMANDS, ...SERVER_COMMANDS } as const;

export type ClientCommandId = (typeof CLIENT_COMMANDS)[keyof typeof CLIENT_COMMANDS];
export type ServerCommandId = (typeof SERVER_COMMANDS)[keyof typeof SERVER_COMMANDS];
export type CommandId = ClientCommandId | ServerCommandId;

export const CLIENT_COMMAND_IDS = Object.values(CLIENT_COMMANDS) as ClientCommandId[];
export const SERVER_COMMAND_IDS = Object.values(SERVER_COMMANDS) as ServerCommandId[];
