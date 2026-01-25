import { describe, expect, test } from "bun:test";
import { normalizeConfig, prepareVSCodeConfig } from "../../src/config";

describe("diagnostics settings", () => {
    test("defaults match extension behavior", () => {
        const config = normalizeConfig();
        expect(config.diagnostics.enable).toBe(true);
        expect(config.diagnostics.onSave).toBe(true);
        expect(config.diagnostics.onChange).toBe(true);
    });

    test("prepareVSCodeConfig forwards diagnostics settings", () => {
        const config = normalizeConfig({
            diagnostics: { enable: false, onSave: false, onChange: true },
        });

        expect(prepareVSCodeConfig(config)).toMatchObject({
            diagnostics: { enable: false, onSave: false, onChange: true },
        });
    });
});
