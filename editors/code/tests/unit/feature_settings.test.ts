import { describe, expect, test } from "bun:test";
import { normalizeConfig, prepareVSCodeConfig } from "../../src/config";
import { loadManifest } from "./helpers";

const manifest = loadManifest();

const featureSettingKeys = [
    "solidity-analyzer.completion.enable",
    "solidity-analyzer.hover.enable",
    "solidity-analyzer.signatureHelp.enable",
    "solidity-analyzer.rename.enable",
    "solidity-analyzer.references.enable",
    "solidity-analyzer.diagnostics.enable",
    "solidity-analyzer.diagnostics.onSave",
    "solidity-analyzer.diagnostics.onChange",
];

describe("feature settings", () => {
    test("defaults are enabled", () => {
        const config = normalizeConfig();
        expect(config.completion.enable).toBe(true);
        expect(config.hover.enable).toBe(true);
        expect(config.signatureHelp.enable).toBe(true);
        expect(config.rename.enable).toBe(true);
        expect(config.references.enable).toBe(true);
        expect(config.diagnostics.enable).toBe(true);
        expect(config.diagnostics.onSave).toBe(true);
        expect(config.diagnostics.onChange).toBe(true);
    });

    test("manifest exposes feature flags", () => {
        const properties = manifest.contributes?.configuration?.properties ?? {};

        for (const key of featureSettingKeys) {
            const entry = properties[key] as { default?: unknown } | undefined;
            expect(entry).toBeDefined();
            expect(typeof entry?.default).toBe("boolean");
        }
    });

    test("prepareVSCodeConfig forwards feature flags", () => {
        const config = normalizeConfig({
            completion: { enable: false },
            hover: { enable: true },
            signatureHelp: { enable: false },
            rename: { enable: false },
            references: { enable: true },
            diagnostics: { enable: true, onSave: false, onChange: true },
        });

        expect(prepareVSCodeConfig(config)).toMatchObject({
            completion: { enable: false },
            hover: { enable: true },
            signatureHelp: { enable: false },
            rename: { enable: false },
            references: { enable: true },
            diagnostics: { enable: true, onSave: false, onChange: true },
        });
    });
});
