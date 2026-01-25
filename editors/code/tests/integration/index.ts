import { readdir } from "node:fs/promises";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

interface TestModule {
    getTests: (ctx: Context) => Promise<void>;
}

function isTestModule(module: unknown): module is TestModule {
    if (!module || typeof module !== "object") {
        return false;
    }
    const candidate = module as { getTests?: unknown };
    return typeof candidate.getTests === "function";
}

class Test {
    readonly name: string;
    readonly runFn: () => Promise<void>;

    constructor(name: string, runFn: () => Promise<void>) {
        this.name = name;
        this.runFn = runFn;
    }
}

class Suite {
    tests: Test[];
    readonly timeoutMs: number;

    constructor(timeoutMs = resolveTestTimeoutMs()) {
        this.tests = [];
        this.timeoutMs = timeoutMs;
    }

    addTest(name: string, run: () => Promise<void>): void {
        const test = new Test(name, run);
        this.tests.push(test);
    }

    async run(): Promise<void> {
        let failed = 0;
        for (const test of this.tests) {
            try {
                await runWithTimeout(test.runFn, this.timeoutMs, test.name);
                ok(`  ✔ ${test.name}`);
            } catch (error) {
                const detail = error instanceof Error ? (error.stack ?? error.message) : String(error);
                errorLog(`  ✖ ${test.name}\n  ${detail}`);
                failed += 1;
            }
        }
        if (failed) {
            const plural = failed > 1 ? "s" : "";
            throw new Error(`${failed} failed test${plural}`);
        }
    }
}

export class Context {
    async suite(name: string, run: (ctx: Suite) => Promise<void> | void): Promise<void> {
        const ctx = new Suite();
        try {
            ok(`⌛ ${name}`);
            await run(ctx);
            await ctx.run();
            ok(`✔ ${name}`);
        } catch (error) {
            const err = error instanceof Error ? error : new Error(String(error));
            errorLog(`✖ ${name}\n  ${err.stack ?? String(error)}`);
            throw error;
        }
    }
}

export async function run(): Promise<void> {
    const context = new Context();
    const testFiles = (await readdir(path.resolve(__dirname))).filter((name) => name.endsWith(".test.js"));
    testFiles.sort((a, b) => a.localeCompare(b));

    for (const testFile of testFiles) {
        const resolvedPath = path.resolve(__dirname, testFile);
        try {
            const testModule: unknown = await import(pathToFileURL(resolvedPath).href);
            if (!isTestModule(testModule)) {
                throw new Error(`Missing getTests export in ${resolvedPath}`);
            }
            await testModule.getTests(context);
        } catch (error) {
            if (error instanceof Error) {
                error.message = `Failed to load integration test ${resolvedPath}: ${error.message}`;
                throw error;
            }
            throw new Error(`Failed to load integration test ${resolvedPath}: ${String(error)}`);
        }
    }
}

function ok(message: string): void {
    console.log(message);
}

function errorLog(message: string): void {
    console.error(message);
}

const DEFAULT_TEST_TIMEOUT_MS = 30_000;

function resolveTestTimeoutMs(): number {
    const rawTimeout = process.env.SA_INTEGRATION_TEST_TIMEOUT_MS;
    if (!rawTimeout) {
        return DEFAULT_TEST_TIMEOUT_MS;
    }

    const parsed = Number.parseInt(rawTimeout, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_TEST_TIMEOUT_MS;
}

async function runWithTimeout(runFn: () => Promise<void>, timeoutMs: number, testName: string): Promise<void> {
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    const timeoutPromise = new Promise<never>((_, reject) => {
        timeoutId = setTimeout(() => {
            reject(new Error(`Test timed out after ${timeoutMs}ms: ${testName}`));
        }, timeoutMs);
    });

    try {
        await Promise.race([runFn(), timeoutPromise]);
    } finally {
        if (timeoutId) {
            clearTimeout(timeoutId);
        }
    }
}
