#!/usr/bin/env bun
import { mkdtemp, mkdir, readdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { materialize, verifyOracle } from "./oracle/materialize.ts";
import { buildFake } from "./build-fake.ts";
import { runScenario } from "./adapters/ts/run.ts";
import { runBlackbox } from "./blackbox/run.ts";
import { compare } from "./diff/compare.ts";
process.env.BUN_RUNTIME_TRANSPILER_CACHE_PATH = "";
const repo = fileURLToPath(new URL("../", import.meta.url));
const args = process.argv.slice(2), check = args.includes("--check");
if (args.some(a => a !== "--check")) throw new Error("usage: bun parity/generate.ts [--check]");
if (Bun.version !== "1.4.0" || Intl.DateTimeFormat().resolvedOptions().locale !== "en-US") throw new Error("run with Bun 1.4.0 and LANG/LC_ALL=en_US.UTF-8");
const os = ({ darwin: "macos", linux: "linux", win32: "windows" } as Record<string, string>)[process.platform];
if (!os) throw new Error(`unsupported platform: ${process.platform}`);
const scratch = await mkdtemp(join(tmpdir(), "envswitch-oracle-"));
let fake: Awaited<ReturnType<typeof buildFake>> | undefined;
try {
  const oracle = process.env.ENVSWITCH_ORACLE_DIR ? resolve(process.env.ENVSWITCH_ORACLE_DIR) : await materialize(join(scratch, "oracle"));
  await verifyOracle(oracle);
  const metadata = JSON.parse(await readFile(join(oracle, "oracle-metadata.json"), "utf8"));
  if (metadata.commit !== "53de029a25a717c8ac824ca8553035e2b0fec991" || metadata.bun !== "1.4.0" || metadata.locale !== "en-US") throw new Error("incorrect oracle metadata");
  fake = await buildFake();
  const output = join(repo, "parity", "goldens", os); await mkdir(output, { recursive: true });
  const counts: Record<string, number> = {}; let ok = true;
  const scenarioFiles = (await readdir(join(repo, "parity", "scenarios"))).filter(p => p.endsWith(".json")).sort();
  const generated = new Set(["_metadata.json"]);
  for (const file of scenarioFiles) {
    const s = await Bun.file(join(repo, "parity", "scenarios", file)).json();
    if (s.platforms && !s.platforms.includes(os)) continue;
    if (s.id + ".json" !== file) throw new Error(`scenario filename/id mismatch: ${file}`);
    generated.add(file);
    const t = s.kind === "blackbox" ? await runBlackbox(s, [process.execPath, join(oracle, "src", "cli.ts")], fake.binary, check && await Bun.file(join(output, file)).exists() ? await Bun.file(join(output, file)).json() : undefined) : await runScenario(s, oracle);
    if (check) {
      if (!await Bun.file(join(output, file)).exists()) { console.error(`missing golden: ${os}/${file}`); ok = false; }
      else ok = compare(await Bun.file(join(output, file)).json(), t) && ok;
    } else await writeFile(join(output, file), JSON.stringify(t, null, 2) + "\n");
    counts[s.kind] = (counts[s.kind] ?? 0) + 1;
  }
  const metadataText = JSON.stringify({ ...metadata, os }, null, 2) + "\n";
  if (check) {
    if (!await Bun.file(join(output, "_metadata.json")).exists() || await readFile(join(output, "_metadata.json"), "utf8") !== metadataText) { console.error("oracle runtime metadata differs"); ok = false; }
    for (const file of (await readdir(output)).filter(f => f.endsWith(".json"))) if (!generated.has(file)) { console.error(`stale golden: ${file}`); ok = false; }
  } else {
    await writeFile(join(output, "_metadata.json"), metadataText);
    for (const file of (await readdir(output)).filter(f => f.endsWith(".json"))) if (!generated.has(file)) await rm(join(output, file));
  }
  console.log(`${check ? ok ? "Verified" : "Mismatched" : "Generated"} ${os} goldens: ${JSON.stringify(counts)}`);
  if (!ok) process.exitCode = 1;
} finally { await fake?.cleanup(); await rm(scratch, { recursive: true, force: true }); }
