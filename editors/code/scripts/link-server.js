const fs = require("node:fs/promises");
const { constants } = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..", "..");
const serverDir = path.join(repoRoot, "editors", "code", "server");
const binaryName = process.platform === "win32" ? "solidity-analyzer.exe" : "solidity-analyzer";
const sourcePath = path.join(repoRoot, "target", "debug", binaryName);
const destPath = path.join(serverDir, binaryName);

async function ensureDir(dir) {
    await fs.mkdir(dir, { recursive: true });
}

async function removeIfExists(target) {
    try {
        await fs.rm(target, { force: true });
    } catch (error) {
        if (error && error.code !== "ENOENT") {
            throw error;
        }
    }
}

async function pathExists(target) {
    try {
        await fs.access(target, constants.F_OK);
        return true;
    } catch {
        return false;
    }
}

async function linkBinary() {
    await ensureDir(serverDir);
    await removeIfExists(destPath);
    await fs.symlink(sourcePath, destPath, "file");
    console.log(`Linked ${destPath} -> ${sourcePath}`);
}

async function copyBinary() {
    await ensureDir(serverDir);
    await removeIfExists(destPath);
    await fs.copyFile(sourcePath, destPath);
    console.warn(`Copied ${sourcePath} -> ${destPath} (symlink unavailable)`);
}

async function main() {
    if (!(await pathExists(sourcePath))) {
        console.error(`Missing server binary at ${sourcePath}. Run cargo build -p solidity-analyzer first.`);
        process.exit(1);
    }

    try {
        await linkBinary();
    } catch (error) {
        if (process.platform === "win32" && error && error.code === "EPERM") {
            await copyBinary();
            return;
        }
        throw error;
    }
}

main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
});
