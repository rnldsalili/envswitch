#!/usr/bin/env bun
import { join, resolve, dirname, delimiter, basename, isAbsolute } from "node:path";
import { mkdir, writeFile, readFile, realpath, lstat } from "node:fs/promises";
import { context, transcript, countRead, finish } from "../context.ts";
import { responsesFor } from "../fixture.ts";
export const HANDSHAKE = "envswitch-fake-op-v1\n";
export async function runBlackbox(input: any, command: string[], fakeOp: string, expected?: any) {
  if (input.kind !== "blackbox") throw new Error("blackbox runner requires a blackbox scenario");
  if (!command.length) throw new Error("CLI command is required");
  // Resolve the native fake itself. A missing file must never fall through to PATH.
  const binary = resolve(fakeOp), stat = await Bun.file(binary).exists();
  if (!stat || basename(binary) !== (process.platform === "win32" ? "op.exe" : "op") || (process.platform === "win32" && !binary.toLowerCase().endsWith("op.exe"))) throw new Error(`native fake-op missing: ${binary}`);
  const executable = resolve(command[0]);
  if (!isAbsolute(command[0])) throw new Error("black-box CLI executable must be absolute");
  await rejectAlternateOp(binary, [dirname(executable), dirname(await realpath(executable).catch(() => executable)), ...(process.platform === "win32" ? [join(process.env.SystemRoot ?? "C:\\Windows", "System32")] : [])]);
  const c = await context(input), s = c.expand(input), t = transcript(input);
  const transport = join(c.tmp, ".transport"); await mkdir(transport);
  const fixture = join(transport, "fixture.json"), trace = join(transport, "trace.jsonl");
  const responses = responsesFor(s);
  await writeFile(fixture, JSON.stringify({ responses })); await writeFile(trace, "");
  const env = { ...c.env, PATH: dirname(binary) + delimiter + (process.env.PATH ?? ""), ENVSWITCH_FAKE_OP_FIXTURE: fixture, ENVSWITCH_FAKE_OP_TRACE: trace };
  for (const key of Object.keys(env)) if (key.toUpperCase() === "PATH" && key !== "PATH") delete (env as Record<string, string | undefined>)[key];
  try {
    // Also exercise PATH lookup: this is the exact executable name the CLI uses.
    if (resolve(Bun.which("op", { PATH: env.PATH, cwd: c.paths.ROOT }) ?? "") !== binary) throw new Error("PATH does not resolve to the native fake-op");
    const handshake = Bun.spawn([binary, "__envswitch_fake_handshake"], { cwd: c.paths.ROOT, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [token, handshakeError, code] = await Promise.all([new Response(handshake.stdout).text(), new Response(handshake.stderr).text(), handshake.exited]);
    if (code || token !== HANDSHAKE || handshakeError) throw new Error(`fake-op handshake failed (${code}): ${handshakeError || JSON.stringify(token)}`);
    if (s.missingOp) { const emptyPath = join(transport, "empty-path"); await mkdir(emptyPath); env.PATH = emptyPath; }
    const child = Bun.spawn([...command, ...(s.argv ?? [])], { cwd: c.paths.ROOT, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr, exitCode] = await Promise.all([new Response(child.stdout).arrayBuffer(), new Response(child.stderr).arrayBuffer(), child.exited]);
    t.exitCode = exitCode; t.stdoutBase64 = c.bytes(Buffer.from(stdout)); t.stderrBase64 = c.bytes(Buffer.from(stderr));
    const invocations = (await readFile(trace, "utf8")).trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
    if (JSON.stringify(invocations.shift()) !== JSON.stringify(["__envswitch_fake_handshake"])) throw new Error("missing fake-op handshake trace");
    if ((expected?.opCalls?.length || s.expectOpCalls) && !invocations.length) throw new Error("empty fake-op trace: expected opCalls");
    t.opCalls = invocations;
    for (const args of t.opCalls) countRead(t.readCounts, args);
    for (const line of Buffer.from(stderr).toString("utf8").split("\n")) {
      const m = line.replace(/\x1b\[[0-9;]*m/g, "").match(/^envswitch \[(info|done|dry-run)\] ([\s\S]*)$/);
      if (m) t.progress.push({ status: m[1], message: m[2] });
    }
    if (invocations.some(args => !responses.some(r => JSON.stringify(r.argv) === JSON.stringify(args)))) throw new Error("CLI called unconfigured fake-op argv");
    return await finish(t, c);
  } finally { await c.cleanup(); }
}
export async function rejectAlternateOp(fake: string, directories: string[]) {
  const verified = await realpath(fake);
  for (const dir of directories) for (const name of ["op", "op.exe"]) {
    const candidate = join(dir, name);
    const entry = await lstat(candidate).catch((error: any) => { if (error.code === "ENOENT" || error.code === "ENOTDIR") return null; throw error; });
    if (!entry) continue;
    const actual = await realpath(candidate).catch(() => { throw new Error(`unsafe alternate op in executable search directory: ${candidate}`); });
    if ((process.platform === "win32" ? actual.toLowerCase() : actual) !== (process.platform === "win32" ? verified.toLowerCase() : verified)) throw new Error(`unsafe alternate op in executable search directory: ${candidate}`);
  }
}

if (import.meta.main) {
  const [scenario, fakeOp, ...command] = process.argv.slice(2);
  if (!scenario || !fakeOp || !command.length) throw new Error("usage: bun parity/blackbox/run.ts <scenario.json> <fake-op> <CLI command...>");
  console.log(JSON.stringify(await runBlackbox(await Bun.file(scenario).json(), command, fakeOp), null, 2));
}
