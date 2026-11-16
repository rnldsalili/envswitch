#!/usr/bin/env bun
// Transport only: shared sandbox/fixture/snapshot rules, never substitute TS for Rust behavior.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { context, transcript, finish } from "../../context.ts";
import { responsesFor } from "../../fixture.ts";
import { runBlackbox } from "../../blackbox/run.ts";
import { ordered, plain, normalizeOrdered } from "./ordered.ts";

import { nativeFixture } from "./update.ts";

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
