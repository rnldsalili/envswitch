import { describe, test, expect } from "bun:test";
import { readFile, readdir, mkdtemp, writeFile, chmod, rm, cp } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { differences } from "./diff/compare.ts";
import { verifyOracle } from "./oracle/materialize.ts";
import { runScenario } from "./adapters/ts/run.ts";
import { nativeFixture } from "./adapters/rust/update.ts";
import { normalizeOrdered, plain } from "./adapters/rust/ordered.ts";
import { validate } from "./schema.ts";
import { context } from "./context.ts";
import { rejectAlternateOp, runBlackbox } from "./blackbox/run.ts";
import { responsesFor } from "./fixture.ts";
const os = ({ darwin: "macos", linux: "linux", win32: "windows" } as Record<string, string>)[process.platform];
const dir = join(import.meta.dir, "goldens", os);
const golden = async (id: string) => JSON.parse(await readFile(join(dir, `${id}.json`), "utf8"));
const bytes = (file: any) => Buffer.from(file.bytesBase64, "base64").toString();

describe("parity harness invariants", () => {
  test("diff preserves array order, empty versus missing, ANSI, CR and trailing LF", () => {
    expect(differences({ b: 1, a: 2 }, { a: 2, b: 1 })).toHaveLength(1);
    expect(differences({ x: ["a", "b"] }, { x: ["b", "a"] })).toHaveLength(2);
    expect(differences({ x: "" }, {})).toHaveLength(2);
    for (const value of ["a\r\n", "a\n\n", "\x1b[32ma\x1b[0m\n"]) expect(differences({ stdoutBase64: Buffer.from(value).toString("base64") }, { stdoutBase64: Buffer.from("a\n").toString("base64") })).toHaveLength(1);
  });
  test("Rust integer-like object keys survive the transport without JS reordering", () => {
    const encoded = ["object", [["z", ["scalar", true]], ["1", ["string", [97]]]]];
    expect(Object.keys(plain(encoded))).toEqual(["1", "z"]); // JS's unavoidable reordering.
    expect(normalizeOrdered(JSON.parse(JSON.stringify(encoded)), s => s)).toEqual(encoded);
  });
  test("explicit fake responses override generated defaults", () => {
    const argv = ["whoami", "--format", "json"];
    const responses = responsesFor({ op: { responses: [{ argv, stdout: "alpha", exitCode: 7 }] } });
    expect(responses.filter(r => JSON.stringify(r.argv) === JSON.stringify(argv))).toEqual([{ argv, stdout: "alpha", stderr: "", exitCode: 7 }]);
  });
  test("missing native fake aborts before CLI execution", async () => {
    await expect(runBlackbox({ id: "guard", kind: "blackbox" }, [join(tmpdir(), "must-not-execute")], "/missing-envswitch-native-fake/op")).rejects.toThrow("native fake-op missing");
  });
  test.skipIf(process.platform === "win32")("failed handshake aborts before CLI execution", async () => {
    const temp = await mkdtemp(join(tmpdir(), "envswitch-handshake-check-"));
    try {
      const op = join(temp, "op"); await writeFile(op, '#!/bin/sh\nprintf "wrong-token\\n"\n'); await chmod(op, 0o755);
      await expect(runBlackbox({ id: "guard", kind: "blackbox" }, [join(tmpdir(), "must-not-execute")], op)).rejects.toThrow("fake-op handshake failed");
    } finally { await rm(temp, { recursive: true, force: true }); }
  });
  test("every committed OS golden validates against the full transcript schema", async () => {
    const schema = await Bun.file(join(import.meta.dir, "transcript.schema.json")).json();
    const parent = join(import.meta.dir, "goldens");
    for (const platform of await readdir(parent)) for (const file of await readdir(join(parent, platform))) {
      if (!file.endsWith(".json") || file.startsWith("_")) continue;
      expect(validate(await Bun.file(join(parent, platform, file)).json(), schema)).toEqual([]);
    }
    const broken = await golden("parseArgs-defaults"); broken.result.flags.vault = 3;
    expect(validate(broken, schema).length).toBeGreaterThan(0);
  });
  test("byte normalization scrubs random writer names without corrupting non-UTF-8", async () => {
    const c = await context({});
    try {
      const input = Buffer.concat([Buffer.from([255]), Buffer.from(`${c.paths.ROOT}/..env.123.12345678-1234-1234-1234-123456789abc.tmp`)]);
      expect(Buffer.from(c.bytes(input), "base64")).toEqual(Buffer.concat([Buffer.from([255]), Buffer.from("{ROOT}/..env.{PID}.{UUID}.tmp")]));
    } finally { await c.cleanup(); }
  });
  test("alternate op files in the executable directory are rejected", async () => {
    const c = await context({});
    try {
      const fake = join(c.tmp, "fake-op"); await writeFile(fake, "fixture");
      await writeFile(join(c.paths.ROOT, "op.exe"), "must not execute");
      await expect(rejectAlternateOp(fake, [c.paths.ROOT])).rejects.toThrow("unsafe alternate op");
    } finally { await c.cleanup(); }
  });
  test("oracle source hash rejects a mutated exported file", async () => {
    const tmp = await mkdtemp(join(tmpdir(), "envswitch-oracle-hash-"));
    try {
      await cp(join(process.env.ENVSWITCH_ORACLE_DIR ?? join(import.meta.dir, ".."), "src"), join(tmp, "src"), { recursive: true });
      await verifyOracle(tmp);
      await writeFile(join(tmp, "src/cli.ts"), "mutated source");
      await expect(verifyOracle(tmp)).rejects.toThrow("oracle src tree hash differs");
    } finally { await rm(tmp, { recursive: true, force: true }); }
  });
  test("empty scripted required input is a harness fault", async () => {
    const oracle = process.env.ENVSWITCH_ORACLE_DIR;
    if (!oracle) return; // CI supplies the materialized oracle; hash test is independent.
    await expect(runScenario({ id: "input-fault", kind: "cli", argv: [], prompts: [""], config: { targets: [{ name: "api", output: ".env" }] } }, oracle)).rejects.toThrow("empty required input");
  });
  test("all OS goldens contain required fields, stable paths and no temp leftovers", async () => {
    const schema = await Bun.file(join(import.meta.dir, "transcript.schema.json")).json();
    const files = (await readdir(dir)).filter(f => f.endsWith(".json") && !f.startsWith("_"));
    expect(files.length).toBeGreaterThan(400);
    for (const file of files) {
      const t = await Bun.file(join(dir, file)).json();
      expect(Object.keys(t).sort()).toEqual(schema.required.toSorted());
      expect(t.schemaVersion).toBe(1); expect(t.leftoverTempFiles).toEqual([]);
      const decoded = [JSON.stringify(t), Buffer.from(t.stdoutBase64, "base64").toString(), Buffer.from(t.stderrBase64, "base64").toString()].join("\n");
      expect(decoded).not.toMatch(/envswitch-parity-[A-Za-z0-9]{6}/);
      expect(t.files.some((f: any) => f.path.includes("/bun/@t@/"))).toBe(false);
      expect(decoded).not.toMatch(/unconfigured fake-op|missing scripted answer|unused scripted answer|barrier timed out/);
    }
  });
  test("collation goldens assert winning bytes, including stable ties", async () => {
    for (const [name, winner] of Object.entries({ "accent-winner": "0:z", "canonical-equal": "1:e\u0301", "punctuation-winner": "0:-x", "case-winner": "0:A", "numeric-winner": "0:item2", "equal-title-api-order": "2:same", expanded: "24:中", "non-latin": "0:中" })) {
      const t = await golden(`switch-collation-${name}`);
      expect(t.exitCode).toBe(0); expect(bytes(t.writtenFiles[0])).toBe(`WINNER=${winner}\n`);
    }
  });
  test("cache reads once across targets and library stays silent", async () => {
    const t = await golden("switch-all-cache");
    expect(Object.values(t.readCounts)).toHaveLength(8);
    expect(Object.values(t.readCounts).every(v => v === 1)).toBe(true);
    expect(t.stdoutBase64).toBe(""); expect(t.stderrBase64).toBe("");
    const silent = await golden("switch-silent-library"); expect(silent.progress).toEqual([]);
  });
  test("serialization failure preserves old bytes and write failure removes the temp file", async () => {
    const invalid = await golden("dotenv-invalid-preserves-existing");
    expect(invalid.exitCode).toBe(2); expect(bytes(invalid.files[0])).toBe("EXISTING=1\n"); expect(invalid.writtenFiles).toEqual([]);
    const failed = await golden("dotenv-rename-cleanup"); expect(failed.exitCode).toBe(2); expect(failed.leftoverTempFiles).toEqual([]);
  });
  test("black-box failures never disclose command stdout", async () => {
    const t = await golden("blackbox-failure-stdout-secret");
    expect(t.exitCode).toBe(2); expect(t.stdoutBase64).toBe("");
    const error = Buffer.from(t.stderrBase64, "base64").toString();
    expect(error).toContain("safe failure"); expect(error).not.toContain("DEMO_SUITE_PRIVATE");
  });
  test("review edge scenarios exercise the intended paths", async () => {
    expect((await golden("blackbox-maxbuffer-exact-limit")).exitCode).toBe(0);
    expect((await golden("blackbox-maxbuffer-failure")).exitCode).toBe(2);
    expect((await golden("blackbox-non-utf8-stdout")).exitCode).toBe(0);
    expect((await golden("cli-guided-vault-list-failure")).exitCode).toBe(2);
    const cancel = await golden("cli-guided-cancel-mode");
    expect(cancel.exitCode).toBe(130); expect(cancel.prompts.map((p: any) => p.message)).toEqual(["Mode"]);
    expect(cancel.readCounts["alpha/cfg"]).toBe(1);
    if (process.platform !== "win32") expect((await golden("path-eacces")).exitCode).toBe(2);
  });
  test("a8 TS acceptance, e6 source status and h1 skip order stay explicit", async () => {
    for (const name of ["overflow", "depth-129", "lone-surrogate"]) for (const kind of ["config", "cli"]) {
      const t = await golden(`${kind}-a8-${name}`);
      expect(t.exitCode).toBe(0); expect(t.divergence).toBe("a8");
    }
    for (const name of ["status-4", ...(process.platform !== "win32" ? ["sigterm"] : [])]) {
      const t = await golden(`cli-update-installer-${name}`);
      expect(t.exitCode).toBe(1); expect(t.divergence).toContain("e6");
    }
    const skipped = await golden("cli-guided-two-unreadable-vaults");
    expect(skipped.exitCode).toBe(0); expect(skipped.divergence).toBe("h1");
    expect(skipped.progress.filter((p: any) => p.message.startsWith("skipping vault")).map((p: any) => p.message)).toEqual([
      "skipping vault alpha (could not list items)", "skipping vault beta (could not list items)",
    ]);
  });
  test("native update fakes commit protocol state and emit exact terminal outcomes", async () => {
    const target = process.platform === "darwin" ? "aarch64-apple-darwin" : process.platform === "win32" ? "x86_64-pc-windows-msvc" : "aarch64-unknown-linux-musl";
    for (const id of ["cli-update-install", "cli-update-same", "cli-update-missing-installer", "cli-update-installer-status-4", "cli-update-installer-throws"]) {
      const input = await Bun.file(join(import.meta.dir, "scenarios", id + ".json")).json(), c = await context(input);
      try {
        const scenario = c.expand(input), expected: any = {};
        const native = (await nativeFixture(c, scenario, expected, { version: "0.0.0", target }))!;
        const before = JSON.parse(await readFile(join(c.paths.APP, "install.json"), "utf8"));
        expect(before.tag).toBe("v0.0.0"); expect(before.payloadPath).toBe(native.currentExe);
        expect(await Bun.file(native.currentExe).exists()).toBe(true);
        await c.actions(native.onInstall);
        const after = await c.snapshot();
        expect(after.files).toEqual(expected.files); expect(after.directories).toEqual(expected.directories);
        if (input.update?.exitCode || input.update?.installerError) {
          expect(native.stdout).toBe(""); expect(native.onInstall).toEqual([]);
        } else {
          const receipt = await Bun.file(join(c.paths.APP, "install.json")).json();
          expect(await readFile(join(c.paths.APP, ".version"), "utf8")).toBe(receipt.tag + "\n");
          expect(await Bun.file(receipt.payloadPath).exists()).toBe(true);
          expect(await Bun.file(receipt.launcherPath).exists()).toBe(true);
          expect(native.stdout).toContain(scenario.rustUpdate.disposition === "unchanged" ? `success: envswitch is already up to date (${receipt.tag})\n` : `success: ${scenario.rustUpdate.disposition} envswitch ${receipt.tag}\n`);
          expect(scenario.rustUpdate.installerStdout).toBe(c.normalizeString(native.stdout));
          if (scenario.rustUpdate.disposition === "unchanged") expect(native.onInstall).toEqual([]);
          else expect(native.stdout).toContain(`info: binary: ${receipt.payloadPath}\ninfo: launcher: ${receipt.launcherPath}\n`);
        }
      } finally { await c.cleanup(); }
    }
  });
  test("parser empty-next errata and scope terminators stay covered", async () => {
    const t = await golden("parseArgs-empty-help-next");
    expect(t.result.flags.help).toBe(true); expect(t.result.positional).toEqual([""]);
    for (const term of ["cr", "lf", "ls", "ps"]) for (const scope of ["client", "target", "group"]) expect((await golden(`classify-scope-${scope}-${term}-end`)).result).toBe("shared");
  });
});
