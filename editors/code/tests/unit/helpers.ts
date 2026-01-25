import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export function loadManifest(): Record<string, unknown> {
    const root = fileURLToPath(new URL("../../", import.meta.url));
    const manifestPath = join(root, "package.json");
    return JSON.parse(readFileSync(manifestPath, "utf8"));
}
