import { describe, expect, test } from "bun:test";
import { loadManifest } from "./helpers";

const manifest = loadManifest();

describe("publish metadata", () => {
    test("publisher is defined", () => {
        expect(typeof manifest.publisher).toBe("string");
        expect((manifest.publisher as string).length).toBeGreaterThan(0);
    });

    test("version is release-ready", () => {
        expect(typeof manifest.version).toBe("string");
        expect((manifest.version as string).endsWith("-dev")).toBe(false);
    });
});
