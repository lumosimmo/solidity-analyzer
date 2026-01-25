export type TraceLevel = "off" | "messages" | "verbose";
export type StatusBarShow = "always" | "never" | "whenActive";
export type StatusBarClickAction = "openLogs" | "restartServer";

type RawFeatureToggle = {
    enable?: boolean;
};

type FeatureToggle = {
    enable: boolean;
};

export type RawConfig = {
    server?: {
        path?: string | null;
        extraEnv?: Record<string, string | null> | null;
    };
    trace?: {
        server?: TraceLevel;
    };
    statusBar?: {
        show?: StatusBarShow;
        clickAction?: StatusBarClickAction;
    };
    completion?: RawFeatureToggle;
    hover?: RawFeatureToggle;
    signatureHelp?: RawFeatureToggle;
    rename?: RawFeatureToggle;
    references?: RawFeatureToggle;
    diagnostics?: {
        enable?: boolean;
        onSave?: boolean;
        onChange?: boolean;
    };
    format?: {
        enable?: boolean;
        onSave?: boolean;
    };
    lint?: {
        enable?: boolean;
        onSave?: boolean;
        fixOnSave?: boolean;
    };
    toolchain?: {
        promptInstall?: boolean;
    };
    initializeStopped?: boolean;
};

export type ExtensionConfig = {
    server: {
        path: string | null;
        extraEnv: Record<string, string>;
    };
    trace: {
        server: TraceLevel;
    };
    statusBar: {
        show: StatusBarShow;
        clickAction: StatusBarClickAction;
    };
    completion: FeatureToggle;
    hover: FeatureToggle;
    signatureHelp: FeatureToggle;
    rename: FeatureToggle;
    references: FeatureToggle;
    diagnostics: {
        enable: boolean;
        onSave: boolean;
        onChange: boolean;
    };
    format: {
        enable: boolean;
        onSave: boolean;
    };
    lint: {
        enable: boolean;
        onSave: boolean;
        fixOnSave: boolean;
    };
    toolchain: {
        promptInstall: boolean;
    };
    initializeStopped: boolean;
};

const defaultConfig: ExtensionConfig = {
    server: {
        path: null,
        extraEnv: {},
    },
    trace: {
        server: "off",
    },
    statusBar: {
        show: "whenActive",
        clickAction: "openLogs",
    },
    completion: {
        enable: true,
    },
    hover: {
        enable: true,
    },
    signatureHelp: {
        enable: true,
    },
    rename: {
        enable: true,
    },
    references: {
        enable: true,
    },
    diagnostics: {
        enable: true,
        onSave: true,
        onChange: true,
    },
    format: {
        enable: true,
        onSave: false,
    },
    lint: {
        enable: true,
        onSave: true,
        fixOnSave: false,
    },
    toolchain: {
        promptInstall: true,
    },
    initializeStopped: false,
};

const ENV_PATTERN = /\$\{env:([^}]+)\}/g;

function expandEnv(value: string, env: NodeJS.ProcessEnv): string {
    return value.replace(ENV_PATTERN, (_match, name: string) => env[name] ?? "");
}

function normalizeExtraEnv(
    extraEnv: Record<string, string | null> | null | undefined,
    env: NodeJS.ProcessEnv,
): Record<string, string> {
    if (!extraEnv) {
        return {};
    }

    const entries = Object.entries(extraEnv)
        .filter(([, value]) => value !== null && value !== undefined)
        .map(([key, value]) => [key, expandEnv(String(value), env)] as const);

    return Object.fromEntries(entries);
}

export function normalizeConfig(raw: RawConfig = {}, env: NodeJS.ProcessEnv = process.env): ExtensionConfig {
    return {
        server: {
            path: raw.server?.path ?? defaultConfig.server.path,
            extraEnv: normalizeExtraEnv(raw.server?.extraEnv, env),
        },
        trace: {
            server: raw.trace?.server ?? defaultConfig.trace.server,
        },
        statusBar: {
            show: raw.statusBar?.show ?? defaultConfig.statusBar.show,
            clickAction: raw.statusBar?.clickAction ?? defaultConfig.statusBar.clickAction,
        },
        completion: {
            enable: raw.completion?.enable ?? defaultConfig.completion.enable,
        },
        hover: {
            enable: raw.hover?.enable ?? defaultConfig.hover.enable,
        },
        signatureHelp: {
            enable: raw.signatureHelp?.enable ?? defaultConfig.signatureHelp.enable,
        },
        rename: {
            enable: raw.rename?.enable ?? defaultConfig.rename.enable,
        },
        references: {
            enable: raw.references?.enable ?? defaultConfig.references.enable,
        },
        diagnostics: {
            enable: raw.diagnostics?.enable ?? defaultConfig.diagnostics.enable,
            onSave: raw.diagnostics?.onSave ?? defaultConfig.diagnostics.onSave,
            onChange: raw.diagnostics?.onChange ?? defaultConfig.diagnostics.onChange,
        },
        format: {
            enable: raw.format?.enable ?? defaultConfig.format.enable,
            onSave: raw.format?.onSave ?? defaultConfig.format.onSave,
        },
        lint: {
            enable: raw.lint?.enable ?? defaultConfig.lint.enable,
            onSave: raw.lint?.onSave ?? defaultConfig.lint.onSave,
            fixOnSave: raw.lint?.fixOnSave ?? defaultConfig.lint.fixOnSave,
        },
        toolchain: {
            promptInstall: raw.toolchain?.promptInstall ?? defaultConfig.toolchain.promptInstall,
        },
        initializeStopped: raw.initializeStopped ?? defaultConfig.initializeStopped,
    };
}

export function prepareVSCodeConfig(config: ExtensionConfig): ExtensionConfig {
    return {
        server: {
            path: config.server.path,
            extraEnv: config.server.extraEnv,
        },
        trace: {
            server: config.trace.server,
        },
        statusBar: {
            show: config.statusBar.show,
            clickAction: config.statusBar.clickAction,
        },
        completion: {
            enable: config.completion.enable,
        },
        hover: {
            enable: config.hover.enable,
        },
        signatureHelp: {
            enable: config.signatureHelp.enable,
        },
        rename: {
            enable: config.rename.enable,
        },
        references: {
            enable: config.references.enable,
        },
        diagnostics: {
            enable: config.diagnostics.enable,
            onSave: config.diagnostics.onSave,
            onChange: config.diagnostics.onChange,
        },
        format: {
            enable: config.format.enable,
            onSave: config.format.onSave,
        },
        lint: {
            enable: config.lint.enable,
            onSave: config.lint.onSave,
            fixOnSave: config.lint.fixOnSave,
        },
        toolchain: {
            promptInstall: config.toolchain.promptInstall,
        },
        initializeStopped: config.initializeStopped,
    };
}

export async function loadConfig(): Promise<ExtensionConfig> {
    const vscode = await import("vscode");
    const config = vscode.workspace.getConfiguration("solidity-analyzer");

    const raw: RawConfig = {
        server: {
            path: config.get("server.path"),
            extraEnv: config.get("server.extraEnv"),
        },
        trace: {
            server: config.get("trace.server"),
        },
        statusBar: {
            show: config.get("statusBar.show"),
            clickAction: config.get("statusBar.clickAction"),
        },
        completion: {
            enable: config.get("completion.enable"),
        },
        hover: {
            enable: config.get("hover.enable"),
        },
        signatureHelp: {
            enable: config.get("signatureHelp.enable"),
        },
        rename: {
            enable: config.get("rename.enable"),
        },
        references: {
            enable: config.get("references.enable"),
        },
        diagnostics: {
            enable: config.get("diagnostics.enable"),
            onSave: config.get("diagnostics.onSave"),
            onChange: config.get("diagnostics.onChange"),
        },
        format: {
            enable: config.get("format.enable"),
            onSave: config.get("format.onSave"),
        },
        lint: {
            enable: config.get("lint.enable"),
            onSave: config.get("lint.onSave"),
            fixOnSave: config.get("lint.fixOnSave"),
        },
        toolchain: {
            promptInstall: config.get("toolchain.promptInstall"),
        },
        initializeStopped: config.get("initializeStopped"),
    };

    return normalizeConfig(raw);
}
