import { describe, expect, test } from "bun:test";
import { clientOptions } from "../../src/client";
import { normalizeConfig } from "../../src/config";

describe("client capabilities", () => {
    test("experimental capabilities are advertised", () => {
        const options = clientOptions(normalizeConfig());
        expect(options.experimental?.snippetTextEdit).toBe(true);
        expect(options.experimental?.codeActionGroup).toBe(true);
    });
});
