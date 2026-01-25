const { spawnSync } = require("node:child_process");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { publish } = require("@vscode/vsce/out/publish");

const RUN_TIMEOUT_MS = 10 * 60 * 1000;

function run(command, args) {
    const result = spawnSync(command, args, {
        stdio: "inherit",
        timeout: RUN_TIMEOUT_MS,
    });
    if (result.error) {
        const errno = result.error.errno;
        const errnoDetail = typeof errno === "number" ? ` (errno: ${errno})` : "";
        console.error(`Failed to start ${command}: ${result.error.message}${errnoDetail}`);
        const exitCode = typeof errno === "number" && errno > 0 ? errno : 1;
        process.exit(exitCode);
    }
    if (result.status !== 0) {
        const exitStatus = typeof result.status === "number" && result.status > 0 ? result.status : 1;
        process.exit(exitStatus);
    }
}

function readFlagValue(args, flag) {
    for (let i = 0; i < args.length; i += 1) {
        const arg = args[i];
        if (arg === flag) {
            const nextArg = args[i + 1];
            if (!nextArg) {
                throw new Error(`Missing value for ${flag}: expected a token after ${flag}`);
            }
            if (nextArg.startsWith("-")) {
                throw new Error(`Invalid value for ${flag}: value appears to be another flag ('${nextArg}')`);
            }
            return nextArg;
        }

        if (arg.startsWith(`${flag}=`)) {
            const value = arg.slice(flag.length + 1);
            if (!value) {
                throw new Error(`Missing value for ${flag}: expected a token after ${flag}=`);
            }
            if (value.startsWith("-")) {
                throw new Error(`Invalid value for ${flag}: value appears to be another flag ('${value}')`);
            }
            return value;
        }
    }

    return null;
}

function loadManifest() {
    const manifestPath = path.join(process.cwd(), "package.json");
    return JSON.parse(readFileSync(manifestPath, "utf8"));
}

function ensureReleaseVersion(version) {
    if (version.endsWith("-dev")) {
        throw new Error(`Refusing to publish a dev version (${version}). Remove the -dev suffix first.`);
    }
}

async function main() {
    const manifest = loadManifest();
    const version = String(manifest.version || "");
    ensureReleaseVersion(version);

    const args = process.argv.slice(2);
    const patArg = readFlagValue(args, "--pat");
    const pat = patArg || process.env.VSCE_PAT;

    if (!pat) {
        throw new Error("Missing VS Code Marketplace token. Provide --pat or set VSCE_PAT.");
    }

    run("bun", ["run", "vscode:prepublish"]);
    await publish({ pat });
    console.log("Publish completed.");
}

main().catch((error) => {
    console.error("Publish failed:", error);
    process.exit(1);
});
