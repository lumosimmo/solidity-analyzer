const { spawnSync } = require("node:child_process");
const path = require("node:path");
const { pack, signPackage } = require("@vscode/vsce/out/package");

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

function readFlagValue(args, index, flag) {
    const arg = args[index];
    if (arg === flag) {
        const nextArg = args[index + 1];
        if (!nextArg) {
            throw new Error(`Missing value for ${flag}: expected a path after ${flag}`);
        }
        if (nextArg.startsWith("-")) {
            throw new Error(`Invalid value for ${flag}: value appears to be another flag ('${nextArg}')`);
        }
        return { value: nextArg, nextIndex: index + 1 };
    }

    if (arg.startsWith(`${flag}=`)) {
        const value = arg.slice(flag.length + 1);
        if (!value) {
            throw new Error(`Missing value for ${flag}: expected a path after ${flag}=`);
        }
        if (value.startsWith("-")) {
            throw new Error(`Invalid value for ${flag}: value appears to be another flag ('${value}')`);
        }
        return { value, nextIndex: index };
    }

    return null;
}

function resolvePackagePath(args) {
    let outArg;
    const remainingArgs = [];

    for (let i = 0; i < args.length; i += 1) {
        const arg = args[i];
        if (arg === "--out" || arg === "-o") {
            const nextArg = args[i + 1];
            if (!nextArg) {
                throw new Error(`Missing value for ${arg}: expected a path after ${arg}`);
            }
            if (nextArg.startsWith("-")) {
                throw new Error(`Invalid value for ${arg}: value appears to be another flag ('${nextArg}')`);
            }
            outArg = nextArg;
            i += 1;
            continue;
        }

        if (arg.startsWith("--out=") || arg.startsWith("-o=")) {
            const value = arg.slice(arg.indexOf("=") + 1);
            if (!value) {
                throw new Error("Missing value for --out: expected a path after --out");
            }
            if (value.startsWith("-")) {
                throw new Error(`Invalid value for --out: value appears to be another flag ('${value}')`);
            }
            outArg = value;
            continue;
        }

        remainingArgs.push(arg);
    }

    const packagePath = path.resolve(
        process.cwd(),
        outArg || process.env.SOLIDITY_ANALYZER_OUT || "solidity-analyzer.vsix",
    );

    return { packagePath, remainingArgs };
}

function parseOptions(args) {
    const { packagePath, remainingArgs } = resolvePackagePath(args);
    const options = {
        packagePath,
        dependencies: false,
    };
    const ignoredArgs = [];

    for (let i = 0; i < remainingArgs.length; i += 1) {
        const arg = remainingArgs[i];

        if (arg === "--dependencies") {
            ignoredArgs.push(arg);
            continue;
        }

        if (arg === "--no-dependencies") {
            options.dependencies = false;
            continue;
        }

        if (arg === "--pre-release") {
            options.preRelease = true;
            continue;
        }

        if (arg === "--allow-star-activation") {
            options.allowStarActivation = true;
            continue;
        }

        if (arg === "--allow-missing-repository") {
            options.allowMissingRepository = true;
            continue;
        }

        if (arg === "--allow-unused-files-pattern") {
            options.allowUnusedFilesPattern = true;
            continue;
        }

        if (arg === "--allow-package-all-secrets") {
            options.allowPackageAllSecrets = true;
            continue;
        }

        if (arg === "--allow-package-env-file") {
            options.allowPackageEnvFile = true;
            continue;
        }

        if (arg === "--skip-license") {
            options.skipLicense = true;
            continue;
        }

        if (arg === "--ignore-other-target-folders") {
            options.ignoreOtherTargetFolders = true;
            continue;
        }

        if (arg === "--follow-symlinks") {
            options.followSymlinks = true;
            continue;
        }

        if (arg === "--no-rewrite-relative-links") {
            options.rewriteRelativeLinks = false;
            continue;
        }

        if (arg === "--no-gitHubIssueLinking") {
            options.gitHubIssueLinking = false;
            continue;
        }

        if (arg === "--no-gitLabIssueLinking") {
            options.gitLabIssueLinking = false;
            continue;
        }

        if (arg === "--allow-package-secrets") {
            const secrets = [];
            while (i + 1 < remainingArgs.length && !remainingArgs[i + 1].startsWith("-")) {
                secrets.push(remainingArgs[i + 1]);
                i += 1;
            }
            if (secrets.length === 0) {
                throw new Error("Missing value for --allow-package-secrets: expected one or more secrets");
            }
            options.allowPackageSecrets = (options.allowPackageSecrets || []).concat(secrets);
            continue;
        }

        if (arg.startsWith("--allow-package-secrets=")) {
            const value = arg.slice("--allow-package-secrets=".length);
            if (!value) {
                throw new Error("Missing value for --allow-package-secrets: expected one or more secrets");
            }
            const secrets = value
                .split(",")
                .map((secret) => secret.trim())
                .filter(Boolean);
            if (secrets.length === 0) {
                throw new Error("Missing value for --allow-package-secrets: expected one or more secrets");
            }
            options.allowPackageSecrets = (options.allowPackageSecrets || []).concat(secrets);
            continue;
        }

        const targetValue = readFlagValue(remainingArgs, i, "--target");
        if (targetValue) {
            options.target = targetValue.value;
            i = targetValue.nextIndex;
            continue;
        }

        const targetShortValue = readFlagValue(remainingArgs, i, "-t");
        if (targetShortValue) {
            options.target = targetShortValue.value;
            i = targetShortValue.nextIndex;
            continue;
        }

        const readmeValue = readFlagValue(remainingArgs, i, "--readme-path");
        if (readmeValue) {
            options.readmePath = readmeValue.value;
            i = readmeValue.nextIndex;
            continue;
        }

        const changelogValue = readFlagValue(remainingArgs, i, "--changelog-path");
        if (changelogValue) {
            options.changelogPath = changelogValue.value;
            i = changelogValue.nextIndex;
            continue;
        }

        const ignoreFileValue = readFlagValue(remainingArgs, i, "--ignoreFile");
        if (ignoreFileValue) {
            options.ignoreFile = ignoreFileValue.value;
            i = ignoreFileValue.nextIndex;
            continue;
        }

        const baseContentUrlValue = readFlagValue(remainingArgs, i, "--baseContentUrl");
        if (baseContentUrlValue) {
            options.baseContentUrl = baseContentUrlValue.value;
            i = baseContentUrlValue.nextIndex;
            continue;
        }

        const baseImagesUrlValue = readFlagValue(remainingArgs, i, "--baseImagesUrl");
        if (baseImagesUrlValue) {
            options.baseImagesUrl = baseImagesUrlValue.value;
            i = baseImagesUrlValue.nextIndex;
            continue;
        }

        const githubBranchValue = readFlagValue(remainingArgs, i, "--githubBranch");
        if (githubBranchValue) {
            options.githubBranch = githubBranchValue.value;
            i = githubBranchValue.nextIndex;
            continue;
        }

        const gitlabBranchValue = readFlagValue(remainingArgs, i, "--gitlabBranch");
        if (gitlabBranchValue) {
            options.gitlabBranch = gitlabBranchValue.value;
            i = gitlabBranchValue.nextIndex;
            continue;
        }

        const signToolValue = readFlagValue(remainingArgs, i, "--sign-tool");
        if (signToolValue) {
            options.signTool = signToolValue.value;
            i = signToolValue.nextIndex;
            continue;
        }

        ignoredArgs.push(arg);
    }

    return { options, ignoredArgs };
}

async function main() {
    run("bun", ["run", "vscode:prepublish"]);

    const { options, ignoredArgs } = parseOptions(process.argv.slice(2));
    if (ignoredArgs.length > 0) {
        console.warn(`Ignoring unsupported vsce args: ${ignoredArgs.map((arg) => `'${arg}'`).join(", ")}`);
    }

    const { packagePath } = await pack(options);
    if (options.signTool) {
        await signPackage(packagePath, options.signTool);
    }

    console.log(`Packaged: ${packagePath}`);
}

main().catch((error) => {
    console.error("Packaging failed:", error);
    process.exit(1);
});
