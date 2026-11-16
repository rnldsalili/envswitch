#!/usr/bin/env bun
// Transport only: shared sandbox/fixture/snapshot rules, never substitute TS for Rust behavior.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { context, transcript, finish } from "../../context.ts";
import { responsesFor } from "../../fixture.ts";
import { runBlackbox } from "../../blackbox/run.ts";
import { ordered, plain, normalizeOrdered } from "./ordered.ts";

async function nativeFixture(c: Awaited<ReturnType<typeof context>>, s: any, expected: any, build: { version: string; target: string }) {
  if (!s.rustUpdate) return null;
  const suffix = process.platform === "win32" ? ".exe" : "";
  const appRoot = join(c.tmp, "app"), installDir = s.rustUpdate.installDir;
  const canonical = (v: string) => "v" + v.trim().replace(/^v/, "");
  const oldFile = c.before.files.find((f: any) => f.path === "{APP}" + (process.platform === "win32" ? "\\" : "/") + ".version");
  const oldTag = canonical(oldFile ? Buffer.from(oldFile.bytesBase64, "base64").toString().trim() || "0.0.4" : "0.0.4");
  const payload = (tag: string) => join(c.paths.APP, "bin", `envswitch_${tag.slice(1)}_${build.target}${suffix}`);
  const launcher = join(installDir, process.platform === "win32" ? "envswitch.cmd" : "envswitch");
  const recipe = (tag: string) => {
    const receipt = { schemaVersion: 1, kind: "native", repo: s.rustUpdate.repo, appRoot, installDir, launcherPath: launcher, target: build.target, tag, payloadPath: payload(tag) };
    return [{ path: payload(tag), content: "fixture payload\n", mode: 0o755 }, { path: launcher, content: "fixture launcher\n", mode: 0o755 }, { path: join(c.paths.APP, "install.json"), content: JSON.stringify(receipt) + "\n", mode: 0o644 }];
  };
  await c.actions([...recipe(oldTag), { path: payload(canonical(build.version)), content: "fixture payload\n", mode: 0o755 }]);
  const before = await c.snapshot();
  const sourceGuard = s.files?.some((f: any) => f.path === join(c.paths.APP, ".git"));
  const success = !sourceGuard && !s.update?.installerError && !(s.update?.exitCode);
  const newTag = canonical(s.update?.updatedVersion ?? s.update?.latest ?? oldTag);
  const onInstall: any[] = success ? [...recipe(newTag), { path: join(c.paths.APP, ".version"), content: newTag + "\n" }] : [];
  // Predict only the fixture installer's writes. Production writes still have to compare exactly.
  const predicted = new Map(before.files.map((f: any) => [f.path, f]));
  for (const f of onInstall) predicted.set(c.normalizeString(f.path), { path: c.normalizeString(f.path), kind: "file", bytesBase64: c.bytes(f.content), mode: process.platform === "win32" ? null : f.mode ?? 0o644 });
  const sort = (a: any, b: any) => c.expand(a.path) < c.expand(b.path) ? -1 : c.expand(a.path) > c.expand(b.path) ? 1 : 0;
  expected.files = [...predicted.values()].sort(sort);
  expected.writtenFiles = expected.files.filter((f: any) => !before.files.some((v: any) => JSON.stringify(v) === JSON.stringify(f)));
  expected.directories = before.directories;
  return { before, onInstall, currentExe: payload(canonical(build.version)) };
}
const [action, arg, arg2, arg3] = process.argv.slice(2);
if (action === "decode-base64") {
  console.log(JSON.stringify(Array.from(Buffer.from(arg, "base64"))));
} else if (action === "actions") {
  const bundle = await Bun.file(arg).json(), c = await context(bundle.input, bundle);
  await c.actions(JSON.parse(await Bun.stdin.text()));
} else if (action === "prepare") {
  const input = await Bun.file(arg).json(), expected = await Bun.file(arg2).json(), c = await context(input);
  try {
    const transport = join(c.tmp, ".transport"); await mkdir(transport);
    const s = c.expand(input);
    const native = await nativeFixture(c, s, expected, arg3 ? JSON.parse(arg3) : { version: "0.0.0", target: "unknown" });
    const bundle = { input, tmp: c.tmp, before: native?.before ?? c.before, native, scenario: ordered(s), expected: ordered(expected), paths: c.paths, responses: responsesFor(s) };
    const path = join(transport, "bundle.json"); await writeFile(path, JSON.stringify(bundle));
    console.log(JSON.stringify({ path }));
  } catch (error) { await c.cleanup(); throw error; }
} else if (action === "finish" || action === "cleanup") {
  const bundle = JSON.parse(await readFile(arg, "utf8"));
  const c = await context(bundle.input, bundle);
  try {
    if (action === "finish") {
      const input = JSON.parse(await Bun.stdin.text());
      const rawPartial = arg2 ? await Bun.file(arg2).json() : null;
      const partial = rawPartial ? plain(rawPartial) : input;
      if (arg2) { partial.stdout = [...(partial.stdout ?? []), ...input.extraStdout]; partial.stderr = [...(partial.stderr ?? []), ...input.extraStderr]; }
      const t = Object.assign(transcript(bundle.input), partial);
      t.stdoutBase64 = c.bytes(Buffer.from(partial.stdout ?? [])); t.stderrBase64 = c.bytes(Buffer.from(partial.stderr ?? []));
      delete t.stdout; delete t.stderr;
      const encoded = ordered(await finish(t, c));
      if (rawPartial) for (const [key, value] of rawPartial[1]) {
        if (["stdout", "stderr"].includes(key)) continue;
        const entry = encoded[1].find(([name]: any) => name === key);
        if (!entry) throw new Error(`unknown native transcript field: ${key}`);
        entry[1] = normalizeOrdered(value, c.normalizeString);
      }
      console.log(JSON.stringify(encoded));
    }
  } finally { await c.cleanup(); }
} else if (action === "blackbox") {
  const input = await Bun.file(arg).json(), expected = await Bun.file(arg2).json();
  const [cli, fake] = JSON.parse(arg3) as string[];
  console.log(JSON.stringify(ordered(await runBlackbox(input, [resolve(cli)], resolve(fake), expected))));
} else throw new Error("bridge: expected prepare/finish/cleanup/blackbox");
