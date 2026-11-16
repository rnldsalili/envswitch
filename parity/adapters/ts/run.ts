#!/usr/bin/env bun
import { pathToFileURL } from "node:url";
import { resolve, join } from "node:path";
import { writeFile } from "node:fs/promises";
import { Readable, Writable } from "node:stream";
import { context, transcript, countRead, finish } from "../../context.ts";
import { responsesFor } from "../../fixture.ts";

export async function runScenario(input: any, oracle: string) {
  if (input.kind === "blackbox") throw new Error("use parity/blackbox/run.ts for blackbox scenarios");
  const c = await context(input), s = c.expand(input), t = transcript(input);
  const oldCwd = process.cwd(), oldEnv = { ...process.env }, oldLog = console.log, oldError = console.error;
  const ttyDescriptor = Object.getOwnPropertyDescriptor(process.stderr, "isTTY");
  const oldUmask = process.umask(s.umask ?? 0o022);
  let stdout = "", stderr = "", answerIndex = 0, scans = 0;
  let isUserError = (_error: unknown) => false;
  let harnessFault: Error | undefined;
  const fault = (message: string) => { harnessFault = new Error(message); return harnessFault; };
  let release!: () => void;
  const barrier = new Promise<void>(r => { release = r; });
  const responses = responsesFor(s);
  let mutationDone = false;
  const runner = { run: async (command: string, args: string[]) => {
    if (command !== "op") throw fault(`unexpected command: ${command}`);
    t.opCalls.push([...args]); countRead(t.readCounts, args);
    if (s.mutateOnRead && !mutationDone && args[0] === "item" && args[1] === "get" && args[2] === s.mutateOnRead.item) { mutationDone = true; await c.actions(s.mutateOnRead.files); }
    if (s.parallelScans && args[0] === "item" && args[1] === "list" && args.at(-1) === "envswitch-config") {
      scans++;
      if (scans === s.parallelScans) release();
      let timer: ReturnType<typeof setTimeout>;
      try { await Promise.race([barrier, new Promise((_, reject) => { timer = setTimeout(() => reject(fault("parallel vault scan barrier timed out")), 2000); })]); }
      finally { clearTimeout(timer!); }
    }
    const response = responses.find(r => JSON.stringify(r.argv) === JSON.stringify(args));
    if (!response) throw fault(`unconfigured fake-op argv: ${JSON.stringify(args)}`);
    if (response.exitCode) throw new Error(response.stderr?.trim() || `Command failed: op ${args.join(" ")}`);
    return response.stdout ?? "";
  } };

  function answer(options: any, kind: "select" | "input") {
    t.prompts.push({ kind, message: options.message, choices: options.choices ?? null, default: options.defaultValue ?? null, required: options.required ?? null });
    const value = s.prompts[answerIndex++];
    if (value?.cancel || value?.error) { const error = new Error(value.error?.message ?? "cancelled"); error.name = value.error?.name ?? "ExitPromptError"; throw error; }
    if (value === undefined || value?.default) {
      if (options.defaultValue !== undefined) return options.defaultValue;
      throw fault(`missing scripted answer: ${options.message}`);
    }
    const resolved = value && typeof value === "object" && "index" in value ? options.choices[value.index]?.value : value;
    if (kind === "select" && !options.choices.some((choice: any) => choice.value === resolved)) throw fault(`invalid scripted answer: ${options.message}`);
    return resolved;
  }
  try {
    for (const key of Object.keys(process.env)) delete process.env[key];
    Object.assign(process.env, c.env);
    c.safe(s.options?.root ?? s.root ?? c.paths.ROOT);
    process.chdir(s.cwd ? c.safe(s.cwd) : c.paths.ROOT);
    Object.defineProperty(process.stderr, "isTTY", { configurable: true, value: s.tty?.stderr ?? false });
  const cli = await import(pathToFileURL(join(resolve(oracle), "src/cli.ts")).href);
  const config = await import(pathToFileURL(join(resolve(oracle), "src/config.ts")).href);
  const switcher = await import(pathToFileURL(join(resolve(oracle), "src/switcher.ts")).href);
  const dotenv = await import(pathToFileURL(join(resolve(oracle), "src/dotenv.ts")).href);
  const { OnePasswordClient } = await import(pathToFileURL(join(resolve(oracle), "src/onepassword.ts")).href);
  const { EnvswitchError } = await import(pathToFileURL(join(resolve(oracle), "src/errors.ts")).href);
    isUserError = (error: unknown) => error instanceof EnvswitchError;
    const op = new OnePasswordClient(s.vault ?? "alpha", runner);
    console.log = (...args) => { stdout += args.map(String).join(" ") + "\n"; };
    console.error = (...args) => {
      const line = args.map(String).join(" "); stderr += c.normalizeString(line) + "\n";
      const match = line.replace(/\x1b\[[0-9;]*m/g, "").match(/^envswitch \[(info|done|dry-run)\] ([\s\S]*)$/);
      if (match) t.progress.push({ status: match[1], message: match[2] });
    };
    const parsedConfig = () => config.parseConfig(s.configRaw ?? JSON.stringify(("config" in s ? s.config : { targets: [{ name: "api", output: "apps/api/.env", groups: ["apis"] }] })));
    const log = (event: any) => t.progress.push(event);
    switch (s.kind) {
      case "parseArgs": t.result = cli.parseArgs(s.argv); break;
      case "formatProgressEvent": t.result = cli.formatProgressEvent(s.event, { color: s.color }); break;
      case "config": t.result = parsedConfig(); break;
      case "tags": {
        const parsed = parsedConfig();
        t.result = { tags: config.tagValues(parsed, s.params ?? {}), candidates: config.candidateListTags(parsed, s.params?.env ?? "local") }; break;
      }
      case "path": t.result = config.resolveOutputPath(s.root ?? c.paths.ROOT, s.output); break;
      case "classify": t.result = switcher.classifyItemForTarget(parsedConfig(), s.item, { env: "local", client: "alpha", target: "api", ...s.params }) ?? null; break;
      case "inspect": t.result = await switcher.inspectProject(parsedConfig(), s.root ?? c.paths.ROOT); break;
      case "switch": t.result = await switcher.switchEnvironment(parsedConfig(), op, { root: s.root ?? c.paths.ROOT, env: "local", client: "alpha", targetName: "all", dryRun: false, ...s.options, ...(s.log === false ? {} : { log }) }); break;
      case "dotenv":
        if (s.write) await dotenv.writeEnvFile(c.safe(s.output ?? "nested/.env"), s.values);
        else t.result = dotenv.stringifyEnv(s.values);
        break;
      case "op": {
        switch (s.method) {
          case "vaults": t.result = await OnePasswordClient.listVaults(runner); break;
          case "items": t.result = await op.listItemsByTags(s.tags ?? []); break;
          case "config": t.result = await op.readConfigItem(s.item ?? "cfg"); break;
          case "env": t.result = await op.readEnvItem(s.item ?? { id: "alpha", title: "alpha" }); break;
          case "auth": await op.assertAuthenticated(); break;
          default: throw fault(`unknown op method: ${s.method}`);
        }
        break;
      }
      case "cli": {
        const streamIn = new Readable({ read() { this.push(null); } });
        const streamOut = new Writable({ write(chunk, _, cb) { stdout += chunk.toString(); cb(); } });
        Object.assign(streamIn, { isTTY: s.tty?.stdin ?? false }); Object.assign(streamOut, { isTTY: s.tty?.stdout ?? false });
        if ((s.isInteractive ?? Boolean(s.tty?.stdin && s.tty?.stdout)) && s.prompts === undefined && !(s.argv?.length)) throw fault("interactive scenarios require scripted prompts");
        t.exitCode = await cli.main(s.argv ?? [], {
          root: s.root ?? c.paths.ROOT, appDir: c.paths.APP, input: streamIn, output: streamOut,
          ...(s.isInteractive === undefined ? {} : { isInteractive: s.isInteractive }),
          ...(s.prompts === undefined ? {} : { prompt: { select: async (o: any) => answer(o, "select"), input: async (o: any) => answer(o, "input") } }),
          listVaults: () => OnePasswordClient.listVaults(runner),
          createOnePasswordClient: (vault: string) => { t.factoryCalls.push(vault); return new OnePasswordClient(vault, runner); },
          fetchLatestVersion: async () => { t.updateCalls.push({ method: "fetchLatestVersion" }); if (s.update?.fetchError) throw new Error(s.update.fetchError); return s.update?.latest; },
          runInstaller: async (scriptPath: string, appRoot: string) => {
            t.updateCalls.push({ method: "runInstaller", scriptPath, appRoot });
            if (s.update?.installerError) throw new Error(s.update.installerError);
            if (s.update?.updatedVersion !== undefined) await writeFile(join(c.paths.APP, ".version"), s.update.updatedVersion);
            return s.update?.exitCode ?? 0;
          },
        });
        break;
      }
      default: throw fault(`unknown scenario kind: ${s.kind}`);
    }
    if (s.parallelScans && scans !== s.parallelScans) throw fault("parallel scan count did not reach barrier");
    if (s.prompts && answerIndex < s.prompts.length) throw fault(`unused scripted answers: ${s.prompts.length - answerIndex}`);
  } catch (error) {
    t.exitCode = isUserError(error) ? 1 : 2;
    const message = c.normalizeString(error instanceof Error ? error.message : String(error));
    stderr += `envswitch: ${message}\n`;
  } finally {
    console.log = oldLog; console.error = oldError; process.chdir(oldCwd); process.umask(oldUmask);
    if (ttyDescriptor) Object.defineProperty(process.stderr, "isTTY", ttyDescriptor); else delete (process.stderr as any).isTTY;
    for (const key of Object.keys(process.env)) delete process.env[key]; Object.assign(process.env, oldEnv);
  }
  try {
    if (harnessFault) throw harnessFault;
    t.stdoutBase64 = c.bytes(stdout); t.stderrBase64 = c.bytes(stderr);
    return await finish(t, c);
  } finally { await c.cleanup(); }
}
if (import.meta.main) {
  const [scenario, oracle = process.env.ENVSWITCH_ORACLE_DIR] = process.argv.slice(2);
  if (!scenario || !oracle) throw new Error("usage: bun parity/adapters/ts/run.ts <scenario.json> <oracle-dir>");
  console.log(JSON.stringify(await runScenario(await Bun.file(scenario).json(), oracle), null, 2));
}
