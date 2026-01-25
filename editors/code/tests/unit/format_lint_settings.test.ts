import { describe, expect, test } from "bun:test";
import { normalizeConfig, prepareVSCodeConfig } from "../../src/config";

describe("format/lint settings", () => {
    test("defaults enable lint on save", () => {
        const config = normalizeConfig();
        expect(config.format.enable).toBe(true);
        expect(config.format.onSave).toBe(false);
        expect(config.lint.enable).toBe(true);
        expect(config.lint.onSave).toBe(true);
        expect(config.lint.fixOnSave).toBe(false);
    });

    test("prepareVSCodeConfig forwards format/lint settings", () => {
        const config = normalizeConfig({
            format: { enable: false, onSave: true },
            lint: { enable: false, onSave: true, fixOnSave: true },
        });

        expect(prepareVSCodeConfig(config)).toMatchObject({
            format: { enable: false, onSave: true },
            lint: { enable: false, onSave: true, fixOnSave: true },
        });
    });
});
