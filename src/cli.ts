#!/usr/bin/env bun
import { input as inquirerInput, select as inquirerSelect } from "@inquirer/prompts";
import {
  lstatSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { constants as osConstants, tmpdir } from "node:os";
import { basename, join, posix, resolve, win32 } from "node:path";
import type { Readable, Writable } from "node:stream";
import { parseConfig, tagValues } from "./config.ts";
import { EnvswitchError, fail } from "./errors.ts";
import { OnePasswordClient } from "./onepassword.ts";
import { inspectProject, switchEnvironment } from "./switcher.ts";
import type {
  EnvswitchConfig,
  LayerName,
  OnePasswordItemSummary,
  OnePasswordVaultSummary,
  ProgressEvent,
  ProgressStatus,
} from "./types.ts";

interface ParsedArgs {
  command: string | undefined;
  positional: string[];
  flags: Record<string, string | boolean>;
}

interface GuidedPrompt {
  select<T>(options: {
    message: string;
    choices: Array<ChoiceOption<T>>;
    defaultValue?: T | undefined;
  }): Promise<T>;
  input(options: {
    message: string;
    defaultValue?: string | undefined;
    required?: boolean;
  }): Promise<string>;
}

type EnvswitchOnePasswordClient = Pick<
  OnePasswordClient,
  "assertAuthenticated" | "listItemsByTags" | "readConfigItem" | "readEnvItem"
>;

interface MainOptions {
  input?: Readable;
  output?: Writable;
  prompt?: GuidedPrompt;
  isInteractive?: boolean;
  root?: string;
  listVaults?: () => Promise<OnePasswordVaultSummary[]>;
  createOnePasswordClient?: (vault: string) => EnvswitchOnePasswordClient;
  log?: (event: ProgressEvent) => void;
  writeLine?: (line: string) => void;
  appDir?: string;
  env?: UpdateEnv;
  fetchLatestVersion?: (repo: string) => Promise<string | undefined>;
  runInstaller?: (plan: InstallPlan) => Promise<InstallerRun>;
}

interface CliDeps {
  input: Readable;
  output: Writable;
  prompt: GuidedPrompt | undefined;
  isInteractive: boolean;
  root: string;
  listVaults: () => Promise<OnePasswordVaultSummary[]>;
  createOnePasswordClient: (vault: string) => EnvswitchOnePasswordClient;
  log: (event: ProgressEvent) => void;
  writeLine: (line: string) => void;
  appDir: string;
  env: UpdateEnv;
  fetchLatestVersion: (repo: string) => Promise<string | undefined>;
  runInstaller: (plan: InstallPlan) => Promise<InstallerRun>;
}

interface ChoiceOption<T> {
  label: string;
  value: T;
}

interface ConfigVaultChoice {
  vault: OnePasswordVaultSummary;
  vaultId: string;
  op: EnvswitchOnePasswordClient;
  configItems: OnePasswordItemSummary[];
}

interface ConfigItemChoice {
  itemId: string;
  logName: string;
}

const DEFAULT_ENV = "local";
const DEFAULT_TARGET = "all";
const CONFIG_ITEM_TAG = "envswitch-config";
const LAYERS: LayerName[] = ["shared", "group", "target", "client", "client-target"];
const SWITCH_FLAGS = new Set(["vault", "config-item", "client", "env", "target", "dry-run", "help"]);
const LIST_FLAGS = new Set(["vault", "config-item", "help"]);
const DOCTOR_FLAGS = new Set(["vault", "config-item", "help"]);
const UPDATE_FLAGS = new Set(["help"]);
const BOOLEAN_FLAGS = new Set(["dry-run", "help"]);
const ANSI = {
  reset: "\x1b[0m",
  muted: "\x1b[2m",
  blue: "\x1b[34m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
} as const;

function logProgress(event: ProgressEvent): void {
  console.error(formatProgressEvent(event, { color: useColor() }));
}

export function formatProgressEvent(
  event: ProgressEvent,
  options: { color: boolean },
): string {
  const label = `[${event.status}]`;
  if (!options.color) {
    return `envswitch ${label} ${event.message}`;
  }

  return [
    `${ANSI.muted}envswitch${ANSI.reset}`,
    `${statusColor(event.status)}${label}${ANSI.reset}`,
    event.message,
  ].join(" ");
}

function useColor(): boolean {
  return Boolean(process.stderr.isTTY) && !process.env.CI && process.env.NO_COLOR === undefined;
}

function statusColor(status: ProgressStatus): string {
  switch (status) {
    case "done":
      return ANSI.green;
    case "dry-run":
      return ANSI.yellow;
    case "info":
      return ANSI.blue;
  }
}

export async function main(argv = Bun.argv.slice(2), options: MainOptions = {}): Promise<number> {
  const deps = createCliDeps(options);
  try {
    if (argv.length === 0) {
      await runGuidedSwitch(deps);
      return 0;
    }

    if (argv[0] === "--help" || argv[0] === "-h") {
      printHelp(deps.writeLine);
      return 0;
    }

    const args = parseArgs(argv);
    if (hasFlag(args, "help") || args.command === "help") {
      printHelp(deps.writeLine);
      return 0;
    }

    switch (args.command) {
      case "switch":
        await runSwitch(args, deps);
        return 0;
      case "list":
        await runList(args, deps);
        return 0;
      case "doctor":
        await runDoctor(args, deps);
        return 0;
      case "update":
        await runUpdate(args, deps);
        return 0;
      default:
        fail(`unknown command: ${args.command}`);
    }
  } catch (error) {
    if (isPromptCancelError(error)) {
      return 130;
    }
    const message = error instanceof Error ? error.message : String(error);
    console.error(`envswitch: ${message}`);
    return error instanceof EnvswitchError ? 1 : 2;
  }
}

export function parseArgs(argv: string[]): ParsedArgs {
  const result: ParsedArgs = {
    command: undefined,
    positional: [],
    flags: {},
  };

  const tokens = [...argv];
  result.command = tokens.shift();

  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index] ?? "";
    if (!token.startsWith("--")) {
      result.positional.push(token);
      continue;
    }

    const raw = token.slice(2);
    const equalIndex = raw.indexOf("=");
    if (equalIndex !== -1) {
      result.flags[raw.slice(0, equalIndex)] = raw.slice(equalIndex + 1);
      continue;
    }

    const next = tokens[index + 1];
    if (!next || next.startsWith("--")) {
      result.flags[raw] = true;
      continue;
    }

    result.flags[raw] = next;
    index += 1;
  }

  return result;
}

async function runGuidedSwitch(deps: CliDeps): Promise<void> {
  if (!deps.isInteractive && !deps.prompt) {
    fail(
      "plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts",
    );
  }

  const prompt = deps.prompt ?? createInquirerPrompt(deps.input, deps.output);
  deps.writeLine("envswitch guided switch");
  deps.writeLine(`Project root: ${deps.root}`);

  const vaultChoice = await askVault(prompt, deps);
  const op = vaultChoice.op;
  const configItem = await askConfigItem(prompt, vaultChoice.configItems);
  const config = await loadConfig(op, configItem.itemId, deps.log, configItem.logName);

  const client = await askChoiceOrValue(prompt, {
    label: "Client",
    choices: config.clients ?? [],
  });
  const env = await askChoiceOrValue(prompt, {
    label: "Environment",
    choices: config.environments ?? [],
    defaultValue: defaultEnvironment(config),
  });
  const targetName = await askTarget(prompt, config);
  const dryRun = await askBoolean(prompt, false);

  const results = await switchEnvironment(config, op, {
    root: deps.root,
    env,
    client,
    targetName,
    dryRun,
    log: deps.log,
  });

  printSwitchResults(results, deps.root, dryRun, deps.writeLine);
}

async function runSwitch(args: ParsedArgs, deps: CliDeps): Promise<void> {
  validateFlags(args, SWITCH_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");
  const client = requireStringFlag(args, "client");
  const env = stringFlag(args, "env", DEFAULT_ENV);
  const targetName = stringFlag(args, "target", DEFAULT_TARGET);
  const dryRun = hasFlag(args, "dry-run");

  const op = deps.createOnePasswordClient(vault);
  const config = await loadConfig(op, configItem, deps.log);
  const results = await switchEnvironment(config, op, {
    root: deps.root,
    env,
    client,
    targetName,
    dryRun,
    log: deps.log,
  });

  printSwitchResults(results, deps.root, dryRun, deps.writeLine);
}

async function runList(args: ParsedArgs, deps: CliDeps): Promise<void> {
  validateFlags(args, LIST_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");
  const subject = args.positional[0] ?? "targets";

  const op = deps.createOnePasswordClient(vault);
  const config = await loadConfig(op, configItem, deps.log);

  switch (subject) {
    case "clients":
      printList(config.clients ?? [], deps.writeLine);
      return;
    case "targets":
      printList(config.targets.map((target) => `${target.name}\t${target.output}`), deps.writeLine);
      return;
    case "layers":
      printLayers(config, deps.writeLine);
      return;
    default:
      fail(`unknown list subject: ${subject}`);
  }
}

async function runDoctor(args: ParsedArgs, deps: CliDeps): Promise<void> {
  validateFlags(args, DOCTOR_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");

  const op = deps.createOnePasswordClient(vault);
  deps.log({ status: "info", message: "checking 1Password authentication" });
  await op.assertAuthenticated();
  deps.log({ status: "done", message: "1Password authentication ok" });
  const config = await loadConfig(op, configItem, deps.log);
  deps.log({ status: "info", message: `inspecting project root ${deps.root}` });
  const targets = await inspectProject(config, deps.root);

  deps.writeLine("1Password authentication: ok");
  deps.writeLine(`project root: ${deps.root}`);
  deps.writeLine(`targets: ${targets.length}`);
  for (const target of targets) {
    const status = target.parentExists ? "ok" : "missing parent";
    deps.writeLine(`- ${target.target.name} -> ${target.outputPath} (${status})`);
  }
}

// ---------------------------------------------------------------------------
// update: the final TypeScript bridge (docs/rust-port/INSTALLER-PROTOCOL.md
// §4.1, §7.1, §7.3, §7.4). The bundled dual-mode installer owns release
// selection details, integrity, health, repair, and rollback; this command
// locates the installation, pins the latest tag once, hands off, and accepts
// only an anchored installer result that agrees with the committed metadata.
// ---------------------------------------------------------------------------

export type UpdateEnv = Record<string, string | undefined>;

export interface InstallPlan {
  /** Bundled installer under current/scripts; it is copied before it runs. */
  installer: string;
  repo: string;
  appRoot: string;
  installDir: string;
  /** Canonical pinned tag (v0.0.N); the bridge always pins. */
  version: string;
  /** Canonical running tag, or undefined when the installed version is unknown. */
  currentVersion: string | undefined;
}

export interface InstallerRun {
  code: number;
  stdout: string;
}

export interface InstallReceipt {
  kind: "source" | "native";
  repo: string;
  appRoot: string;
  installDir: string;
  launcherPath: string;
  target: string | null;
  tag: string;
  payloadPath: string;
}

export type InstallerOutcome = "installed" | "repaired" | "unchanged";

interface InstallLocation {
  repo: string;
  appRoot: string;
  installDir: string;
}

const DEFAULT_REPO = "rnldsalili/envswitch";
const GITHUB_API = "https://api.github.com";
const CANONICAL_TAG = /^v0\.0\.(0|[1-9][0-9]*)$/;
const VERSION_INPUT = /^v?0\.0\.(0|[1-9][0-9]*)$/;
const REPO_PATTERN = /^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/;
const LOOPBACK_API = /^http:\/\/(127\.0\.0\.1|localhost|\[::1\])(:[0-9]+)?$/;
const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/;
const NATIVE_TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-musl",
  "aarch64-unknown-linux-musl",
  "x86_64-pc-windows-msvc",
] as const;
const IS_WINDOWS = process.platform === "win32";
const SOURCE_CHECKOUT_MESSAGE =
  "envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)";
const RECEIPT_MESSAGE =
  "cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README";
const LAUNCHER_DIR_MESSAGE =
  "cannot recover this installation's launcher directory; reinstall envswitch with explicit APP_ROOT and INSTALL_DIR (PowerShell: -AppRoot and -InstallDir) using the README instructions";
const POSTCONDITION_MESSAGE =
  "update failed: installer result does not match the installed receipt and version marker; reinstall envswitch with the install script from the README";
const GUIDANCE_RESTART = "info: restart your terminal, or run this command now:";
const GUIDANCE_VERIFY = "info: run 'envswitch --help' to verify the installation";
const GUIDANCE_THEN_VERIFY = "info: then run 'envswitch --help' to verify the installation";

async function runUpdate(args: ParsedArgs, deps: CliDeps): Promise<void> {
  validateFlags(args, UPDATE_FLAGS);

  if (pathEntryExists(join(deps.appDir, ".git"))) {
    fail(SOURCE_CHECKOUT_MESSAGE);
  }

  const installer = join(deps.appDir, "scripts", IS_WINDOWS ? "install.ps1" : "install.sh");
  if (!isRegularFile(installer)) {
    fail(`installer not found at ${installer}; reinstall envswitch with the install script from the README`);
  }

  const current = readCurrentTag(join(deps.appDir, ".version"));
  const location = locateInstallation(deps.appDir, deps.env, IS_WINDOWS);
  const pin = await deps.fetchLatestVersion(location.repo);
  if (pin === undefined || !CANONICAL_TAG.test(pin)) {
    fail(`no GitHub Release found for ${location.repo}, or access is denied`);
  }

  deps.log({ status: "info", message: `updating envswitch ${current ?? "(unknown version)"} -> ${pin}` });
  const plan: InstallPlan = { installer, ...location, version: pin, currentVersion: current };
  const run = await deps.runInstaller(plan);
  if (run.code !== 0) {
    fail(`update failed: installer exited with code ${run.code}`);
  }

  const result = verifyInstallerResult(plan, run.stdout, deps.env, IS_WINDOWS);
  if (!result.exactLocations) {
    console.error(
      "envswitch: warning: installer locations could not be matched after console decoding; validated the installed receipt instead",
    );
  }
  deps.writeLine(updateSummary(current, result.tag, result.outcome));
}

/** §7.4: the CLI summary line, classified against the running (old) version. */
function updateSummary(old: string | undefined, tag: string, outcome: InstallerOutcome): string {
  if ((old !== undefined && tag !== old) || outcome === "installed") {
    return `updated envswitch ${old ?? "(unknown)"} -> ${tag}`;
  }
  if (outcome === "repaired") {
    return `repaired envswitch ${tag}`;
  }
  return `envswitch is already up to date (${tag})`;
}

/** Canonical tag from current/.version; legacy BOM/CRLF markers are tolerated (§4). */
function readCurrentTag(path: string): string | undefined {
  let text: string;
  try {
    text = readFileSync(path).toString("utf8");
  } catch {
    return undefined;
  }
  if (text.startsWith("\uFEFF")) {
    text = text.slice(1);
  }
  text = text.replace(/\r?\n$/, "");
  return VERSION_INPUT.test(text) ? `v${text.replace(/^v/, "")}` : undefined;
}

/**
 * §7.3: a valid receipt for this root is authoritative. Without one, adopt the
 * known app root and identify exactly one managed launcher without scanning.
 */
function locateInstallation(appDir: string, env: UpdateEnv, windows: boolean): InstallLocation {
  const paths = pathApi(windows);
  if (!samePath(paths.basename(appDir), "current", windows)) {
    fail(LAUNCHER_DIR_MESSAGE);
  }
  const appRoot = paths.dirname(appDir);
  if (pathEntryExists(paths.join(appDir, "install.json"))) {
    const receipt = readInstallReceipt(appRoot, env, windows);
    if (!receipt) {
      fail(RECEIPT_MESSAGE);
    }
    return { repo: receipt.repo, appRoot: receipt.appRoot, installDir: receipt.installDir };
  }
  return adoptInstallation(appRoot, env, windows);
}

function adoptInstallation(appRoot: string, env: UpdateEnv, windows: boolean): InstallLocation {
  const paths = pathApi(windows);
  // Old Windows installs never stored -Repo/-InstallDir, and PowerShell does
  // not read these variables, so only Unix honors explicit REPO/INSTALL_DIR.
  const repo = (windows ? undefined : nonEmpty(env.REPO)) ?? DEFAULT_REPO;
  if (!validRepo(repo)) {
    fail(`invalid repository format: ${repo}; expected owner/repo`);
  }
  const explicitDir = windows ? undefined : nonEmpty(env.INSTALL_DIR);
  if (explicitDir !== undefined) {
    return { repo, appRoot, installDir: normalizeUnixDirectory(explicitDir) ?? explicitDir };
  }

  const candidates: string[] = [];
  if (windows) {
    candidates.push(win32.join(appRoot, "bin"));
  } else if (nonEmpty(env.HOME) !== undefined) {
    candidates.push(posix.join(env.HOME!, ".local", "bin"));
  }
  for (const entry of (pathEnv(env, windows) ?? "").split(windows ? ";" : ":")) {
    const dir = windows ? entry.replace(/^"(.*)"$/, "$1") : entry;
    if (dir !== "" && paths.isAbsolute(dir)) {
      candidates.push(dir);
    }
  }

  const matches = new Map<string, string>();
  for (const dir of candidates) {
    let real: string;
    let content: Buffer;
    try {
      real = realpathSync(paths.join(dir, launcherName(windows)));
      if (!statSync(real).isFile()) {
        continue;
      }
      content = readFileSync(real);
    } catch {
      continue;
    }
    if (isManagedLauncher(content, appRoot, windows)) {
      matches.set(windows ? real.toUpperCase() : real, real);
    }
  }
  const [launcher, ...others] = [...matches.values()];
  if (launcher === undefined || others.length > 0) {
    fail(LAUNCHER_DIR_MESSAGE);
  }
  return { repo, appRoot, installDir: paths.dirname(launcher) };
}

/**
 * Recognize launchers the dual-mode installer will adopt for this root
 * (§5 step 1), mirroring its comparisons: the historical Bun launcher, the
 * protocol-1 source launcher, or a protocol-1 native launcher for this root.
 */
export function isManagedLauncher(content: Uint8Array, appRoot: string, windows: boolean): boolean {
  let text = Buffer.from(content).toString("utf8");
  if (windows) {
    if (text.startsWith("\uFEFF")) {
      text = text.slice(1);
    }
    const app = win32.join(appRoot, "current");
    const cli = win32.join(app, "src", "cli.ts");
    if (isWindowsLauncher(text, "source", cli)) {
      return true;
    }
    const legacy = [
      "@echo off",
      "setlocal",
      "where bun >NUL 2>NUL",
      "if %ERRORLEVEL% EQU 0 (",
      `  bun "${cli}" %*`,
      "  exit /b %ERRORLEVEL%",
      ")",
      'if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" (',
      `  "%USERPROFILE%\\.bun\\bin\\bun.exe" "${cli}" %*`,
      "  exit /b %ERRORLEVEL%",
      ")",
      "echo envswitch: bun is not installed or not on PATH 1>&2",
      "exit /b 127",
    ].join("\n");
    if (text.replace(/\r\n/g, "\n").replace(/\n+$/, "") === legacy) {
      return true;
    }
    const native = /envswitch_(0\.0\.(?:0|[1-9][0-9]*))_x86_64-pc-windows-msvc\.exe/i.exec(text);
    return (
      native !== null &&
      isWindowsLauncher(text, "native", payloadFor(appRoot, "native", `v${native[1]}`, "x86_64-pc-windows-msvc", true))
    );
  }

  // Bash compares `$(cat launcher)`, which drops trailing newlines.
  const trimmed = text.replace(/\n+$/, "");
  const cli = posix.join(appRoot, "current", "src", "cli.ts");
  if (trimmed === unixLauncher("source", cli).replace(/\n+$/, "")) {
    return true;
  }
  const legacy = unixLauncher("source", cli, true).replace(/\n+$/, "");
  if (trimmed.split("\n").filter((line) => line !== "").join("\n") === legacy) {
    return true;
  }
  const native = /envswitch_(0\.0\.(?:0|[1-9][0-9]*))_/.exec(trimmed);
  if (native === null) {
    return false;
  }
  return NATIVE_TARGETS.some(
    (target) =>
      trimmed === unixLauncher("native", payloadFor(appRoot, "native", `v${native[1]}`, target, false)).replace(/\n+$/, ""),
  );
}

function unixLauncher(kind: "source" | "native", payload: string, legacy = false): string {
  let text = "#!/usr/bin/env bash\nset -euo pipefail\n";
  if (kind === "source") {
    text += [
      "if command -v bun >/dev/null 2>&1; then",
      '  BUN_BIN="$(command -v bun)"',
      'elif [[ -x "${HOME}/.bun/bin/bun" ]]; then',
      '  BUN_BIN="${HOME}/.bun/bin/bun"',
      "else",
      "  printf 'envswitch: bun is not installed or not on PATH\\n' >&2",
      "  exit 127",
      "fi",
      'exec "$BUN_BIN" ',
    ].join("\n");
  } else {
    text += "exec ";
  }
  text += legacy ? `"${payload}"` : `'${payload.replace(/'/g, "'\\''")}'`;
  return `${text} "$@"\n`;
}

function isWindowsLauncher(content: string, kind: "source" | "native", payload: string): boolean {
  const path = payload.replace(/%/g, "%%");
  const lines = [
    "@echo off",
    "setlocal DisableDelayedExpansion",
    'for /f "tokens=2 delims=:." %%C in (\'chcp\') do set "_ENVSWITCH_CP=%%C"',
    "chcp 65001 >nul",
  ];
  if (kind === "native") {
    lines.push(`"${path}" %*`, 'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"');
  } else {
    lines.push(
      "where bun >NUL 2>NUL",
      "if %ERRORLEVEL% EQU 0 goto envswitch_bun_path",
      'if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" goto envswitch_bun_home',
      "echo envswitch: bun is not installed or not on PATH 1>&2",
      'set "_ENVSWITCH_EXIT=127"',
      "goto envswitch_finish",
      ":envswitch_bun_path",
      `bun "${path}" %*`,
      'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"',
      "goto envswitch_finish",
      ":envswitch_bun_home",
      `"%USERPROFILE%\\.bun\\bin\\bun.exe" "${path}" %*`,
      'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"',
      ":envswitch_finish",
    );
  }
  lines.push("chcp %_ENVSWITCH_CP% >nul 2>&1", "exit /b %_ENVSWITCH_EXIT%");
  const expected = `${lines.join("\r\n")}\r\n`;
  // The protocol text's earlier code-page lines are also accepted by the installer.
  const previous = expected
    .replace("tokens=2 delims=:.", "tokens=2 delims=:")
    .replace("chcp %_ENVSWITCH_CP% >nul 2>&1", "chcp %_ENVSWITCH_CP% >nul");
  return content === expected || content === previous;
}

function payloadFor(
  appRoot: string,
  kind: "source" | "native",
  tag: string,
  target: string | null,
  windows: boolean,
): string {
  const paths = pathApi(windows);
  if (kind === "source") {
    return paths.join(appRoot, "current", "src", "cli.ts");
  }
  return paths.join(appRoot, "current", "bin", `envswitch_${tag.slice(1)}_${target}${windows ? ".exe" : ""}`);
}

/** Read and fully validate APP_ROOT/current/install.json (§4.1); undefined when invalid. */
export function readInstallReceipt(appRoot: string, env: UpdateEnv, windows = IS_WINDOWS): InstallReceipt | undefined {
  const file = pathApi(windows).join(appRoot, "current", "install.json");
  if (!validInstallPath(appRoot, windows) || !hasNoLinks(file, windows)) {
    return undefined;
  }
  let bytes: Buffer;
  try {
    bytes = readFileSync(file);
  } catch {
    return undefined;
  }
  const receipt = parseInstallReceipt(bytes, appRoot, env, windows);
  if (
    !receipt ||
    ![receipt.appRoot, receipt.installDir, receipt.launcherPath, receipt.payloadPath].every((path) =>
      hasNoLinks(path, windows),
    )
  ) {
    return undefined;
  }
  return receipt;
}

/** Lexical receipt validation for the expected app root; no filesystem access. */
export function parseInstallReceipt(
  bytes: Uint8Array,
  appRoot: string,
  env: UpdateEnv,
  windows: boolean,
): InstallReceipt | undefined {
  let text: string;
  try {
    // Keep a BOM so it is rejected, and reject invalid UTF-8.
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return undefined;
  }
  const data = parseStrictJson(text);
  if (!(data instanceof Map)) {
    return undefined;
  }
  const schema = data.get("schemaVersion");
  if (!(schema instanceof JsonNumber) || schema.raw !== "1") {
    return undefined;
  }
  const field = (name: string): string | undefined => {
    const value = data.get(name);
    return typeof value === "string" && value.isWellFormed() ? value : undefined;
  };
  const kind = field("kind");
  const repo = field("repo");
  const root = field("appRoot");
  const installDir = field("installDir");
  const launcherPath = field("launcherPath");
  const tag = field("tag");
  const payloadPath = field("payloadPath");
  const rawTarget = data.get("target");
  if (
    (kind !== "source" && kind !== "native") ||
    repo === undefined ||
    root === undefined ||
    installDir === undefined ||
    launcherPath === undefined ||
    tag === undefined ||
    payloadPath === undefined ||
    (rawTarget !== null && typeof rawTarget !== "string")
  ) {
    return undefined;
  }
  const target = rawTarget;
  if (
    !validRepo(repo) ||
    !CANONICAL_TAG.test(tag) ||
    !samePath(root, appRoot, windows) ||
    ![root, installDir, launcherPath, payloadPath].every((path) => validInstallPath(path, windows)) ||
    samePath(installDir, root, windows)
  ) {
    return undefined;
  }
  for (const home of ["HOME", "USERPROFILE"]) {
    const value = nonEmpty(env[home]);
    if (value !== undefined && samePath(root, value, windows)) {
      return undefined;
    }
  }
  const paths = pathApi(windows);
  const current = paths.join(root, "current");
  if (
    ancestors(installDir, windows).some((path) => samePath(path, current, windows)) ||
    [installDir, root].some((path) =>
      ancestors(path, windows).some((ancestor) => paths.basename(ancestor).startsWith(".install-")),
    ) ||
    !samePath(launcherPath, paths.join(installDir, launcherName(windows)), windows) ||
    ancestors(root, windows).some((path) => samePath(path, launcherPath, windows))
  ) {
    return undefined;
  }
  if (kind === "source" ? target !== null : !isNativeTarget(target, windows)) {
    return undefined;
  }
  if (!samePath(payloadPath, payloadFor(root, kind, tag, target, windows), windows)) {
    return undefined;
  }
  return { kind, repo, appRoot: root, installDir, launcherPath, target, tag, payloadPath };
}

function isNativeTarget(target: string | null, windows: boolean): boolean {
  return (
    target !== null &&
    (NATIVE_TARGETS as readonly string[]).includes(target) &&
    target.endsWith("windows-msvc") === windows
  );
}

/** §7.1 postconditions: committed receipt + marker + anchored terminal line. */
function verifyInstallerResult(
  plan: InstallPlan,
  stdout: string,
  env: UpdateEnv,
  windows: boolean,
): { tag: string; outcome: InstallerOutcome; exactLocations: boolean } {
  const paths = pathApi(windows);
  const after = readInstallReceipt(plan.appRoot, env, windows);
  if (!after) {
    fail(POSTCONDITION_MESSAGE);
  }
  const markerPath = paths.join(after.appRoot, "current", ".version");
  let marker: Buffer | undefined;
  try {
    marker = readFileSync(markerPath);
  } catch {
    marker = undefined;
  }
  if (
    after.repo !== plan.repo ||
    after.tag !== plan.version ||
    !samePath(after.installDir, plan.installDir, windows) ||
    !samePath(after.launcherPath, paths.join(plan.installDir, launcherName(windows)), windows) ||
    marker === undefined ||
    !marker.equals(Buffer.from(`${after.tag}\n`)) ||
    !hasNoLinks(markerPath, windows) ||
    !isRegularFile(after.payloadPath) ||
    !isRegularFile(after.launcherPath)
  ) {
    fail(POSTCONDITION_MESSAGE);
  }
  const result = parseInstallerOutcome(stdout, after, windows);
  if (!result) {
    fail(POSTCONDITION_MESSAGE);
  }
  return { tag: after.tag, ...result };
}

/**
 * §6.1: accept only the installer-owned terminal line (the last `success: `
 * line) followed by exactly the defined location and PATH-guidance suffix.
 */
export function parseInstallerOutcome(
  stdout: string,
  receipt: InstallReceipt,
  windows: boolean,
): { outcome: InstallerOutcome; exactLocations: boolean } | undefined {
  if (!stdout.endsWith("\n") || !CANONICAL_TAG.test(receipt.tag)) {
    return undefined;
  }
  const lines = stdout
    .slice(0, -1)
    .split("\n")
    .map((line) => (line.endsWith("\r") ? line.slice(0, -1) : line));
  let index = lines.length - 1;
  while (index >= 0 && !lines[index]!.startsWith("success: ")) {
    index -= 1;
  }
  if (index < 0) {
    return undefined;
  }
  const tag = receipt.tag;
  const line = lines[index];
  let outcome: InstallerOutcome;
  if (line === `success: envswitch is already up to date (${tag})`) {
    return index === lines.length - 1 ? { outcome: "unchanged", exactLocations: true } : undefined;
  } else if (line === `success: installed envswitch ${tag}`) {
    outcome = "installed";
  } else if (line === `success: repaired envswitch ${tag}`) {
    outcome = "repaired";
  } else {
    return undefined;
  }

  const locationPrefix = receipt.kind === "native" ? "info: binary: " : "info: source: ";
  const location =
    locationPrefix + (receipt.kind === "native" ? receipt.payloadPath : pathApi(windows).join(receipt.appRoot, "current"));
  const launcher = `info: launcher: ${receipt.launcherPath}`;
  const suffix = lines.slice(index + 1);
  const exactLocations = suffix[0] === location && suffix[1] === launcher;
  const guidance = suffix.slice(2);
  if (windows) {
    // The console code page can replace characters PowerShell writes to a
    // pipe; the receipt and filesystem checks above stay authoritative.
    const shaped =
      suffix[0]?.startsWith(locationPrefix) === true &&
      suffix[1]?.startsWith("info: launcher: ") === true &&
      guidance.length === 3 &&
      guidance[0] === GUIDANCE_RESTART &&
      guidance[1]!.startsWith('info: $env:Path = "') &&
      guidance[1]!.endsWith(';$env:Path"') &&
      guidance[2] === GUIDANCE_THEN_VERIFY;
    return shaped ? { outcome, exactLocations } : undefined;
  }
  if (!exactLocations) {
    return undefined;
  }
  if (guidance.length === 1 && guidance[0] === GUIDANCE_VERIFY) {
    return { outcome, exactLocations };
  }
  const escaped = receipt.installDir
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\$/g, "\\$")
    .replace(/`/g, "\\`");
  const shaped =
    guidance.length === 3 &&
    guidance[0] === GUIDANCE_RESTART &&
    guidance[1] === `info: export PATH="${escaped}:$PATH"` &&
    guidance[2] === GUIDANCE_THEN_VERIFY;
  return shaped ? { outcome, exactLocations } : undefined;
}

export interface LatestReleaseOptions {
  env?: UpdateEnv;
  fetch?: (url: string, init: RequestInit) => Promise<Response>;
  sleep?: (ms: number) => Promise<void>;
}

/**
 * Resolve the latest release tag exactly once (§7.3). Only a canonical,
 * non-draft, non-prerelease tag is returned; any failure is undefined, and
 * the caller never falls back to an unpinned installer lookup.
 */
export async function fetchLatestReleaseTag(
  repo: string,
  options: LatestReleaseOptions = {},
): Promise<string | undefined> {
  const env = options.env ?? process.env;
  const doFetch = options.fetch ?? ((url: string, init: RequestInit) => fetch(url, init));
  const sleep = options.sleep ?? ((ms: number) => Bun.sleep(ms));
  const base = githubApiBase(env);
  const headers: Record<string, string> = { Accept: "application/vnd.github+json" };
  const token = nonEmpty(env.GITHUB_TOKEN);
  if (token !== undefined) {
    headers.Authorization = `Bearer ${token}`;
  }

  for (let attempt = 1; attempt <= 3; attempt += 1) {
    let url = `${base}/repos/${repo}/releases/latest`;
    let retryDelay: number | undefined;
    try {
      const signal = AbortSignal.timeout(60_000);
      for (let redirects = 0; ; redirects += 1) {
        const response = await doFetch(url, { headers, redirect: "manual", signal });
        const location = response.headers.get("location");
        if ([301, 302, 307, 308].includes(response.status) && location !== null && redirects < 5) {
          // Follow only same-origin API redirects (renamed repositories); the
          // token is never sent to another origin.
          const next = new URL(location, url);
          if (next.origin !== new URL(base).origin) {
            return undefined;
          }
          url = next.href;
          continue;
        }
        if (response.status === 200) {
          return latestTagFromBody(await response.text());
        }
        if (response.status !== 408 && response.status !== 429 && response.status < 500) {
          return undefined;
        }
        const retryAfter = Number(response.headers.get("retry-after"));
        retryDelay = Number.isFinite(retryAfter) && retryAfter > 0 ? Math.min(retryAfter, 60) * 1000 : undefined;
        break;
      }
    } catch {
      // Transport failure or timeout: retry below.
    }
    if (attempt < 3) {
      await sleep(Math.max(retryDelay ?? 0, 1000 * 2 ** (attempt - 1)));
    }
  }
  return undefined;
}

/** Parsing failures are final, not retried as another request. */
function latestTagFromBody(text: string): string | undefined {
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return undefined;
  }
  const release = body as { tag_name?: unknown; draft?: unknown; prerelease?: unknown };
  const tag = release.tag_name;
  return typeof tag === "string" && CANONICAL_TAG.test(tag) && release.draft !== true && release.prerelease !== true
    ? tag
    : undefined;
}

/**
 * The same unsupported, explicitly gated test hook the installers honor
 * (§2.2): only ENVSWITCH_INSTALL_TEST_MODE=install with a loopback HTTP origin
 * replaces the GitHub API; an invalid setting is an error, never production.
 */
function githubApiBase(env: UpdateEnv): string {
  const mode = nonEmpty(env.ENVSWITCH_INSTALL_TEST_MODE);
  if (mode === undefined) {
    return GITHUB_API;
  }
  if (mode !== "install") {
    fail("invalid installer setting: ENVSWITCH_INSTALL_TEST_MODE");
  }
  const base = env.ENVSWITCH_INSTALL_TEST_API_BASE_URL ?? "";
  if (!LOOPBACK_API.test(base)) {
    fail("invalid installer setting: ENVSWITCH_INSTALL_TEST_API_BASE_URL");
  }
  return base;
}

/** §7.1 handoff argv and environment (argument array; never a command string). */
export function buildInstallerInvocation(
  plan: InstallPlan,
  script: string,
  baseEnv: UpdateEnv,
  windows: boolean,
): { argv: string[]; env: Record<string, string> } {
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(baseEnv)) {
    if (value !== undefined) {
      env[name] = value;
    }
  }
  const unset = (name: string) => {
    for (const key of Object.keys(env)) {
      if (windows ? key.toUpperCase() === name : key === name) {
        delete env[key];
      }
    }
  };
  const set = (name: string, value: string) => {
    unset(name);
    env[name] = value;
  };

  let argv: string[];
  if (windows) {
    argv = [
      "powershell",
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      script,
      "-AppRoot",
      plan.appRoot,
      "-InstallDir",
      plan.installDir,
      "-Repo",
      plan.repo,
      "-Version",
      plan.version,
    ];
    if (plan.currentVersion !== undefined) {
      argv.push("-CurrentVersion", plan.currentVersion);
    }
  } else {
    argv = ["bash", script];
    // Receipt/identified settings take precedence over unrelated shell values.
    set("REPO", plan.repo);
    set("APP_ROOT", plan.appRoot);
    set("INSTALL_DIR", plan.installDir);
    set("ENVSWITCH_VERSION", plan.version);
    if (plan.currentVersion !== undefined) {
      set("ENVSWITCH_CURRENT_VERSION", plan.currentVersion);
    }
  }
  // Never let a caller's stale current-version label or attestation request
  // leak into an ordinary update; test hooks and GITHUB_TOKEN are preserved.
  if (plan.currentVersion === undefined) {
    unset("ENVSWITCH_CURRENT_VERSION");
  }
  unset("ENVSWITCH_VERIFY_ATTESTATIONS");
  return { argv, env };
}

export interface RunInstallScriptOptions {
  env?: UpdateEnv;
  windows?: boolean;
  /** Parent of the private temporary directory; defaults to os.tmpdir(). */
  tempRoot?: string;
  /** Receives the installer's stdout bytes as they arrive. */
  stdout?: { write(chunk: Uint8Array): unknown };
}

/**
 * Copy the bundled installer into a new private directory outside `current`
 * (the installer may replace `current/scripts`), run it there with stdin and
 * stderr inherited and stdout teed, then remove the directory.
 */
export async function runInstallScript(plan: InstallPlan, options: RunInstallScriptOptions = {}): Promise<InstallerRun> {
  const env = options.env ?? process.env;
  const windows = options.windows ?? IS_WINDOWS;
  const out = options.stdout ?? process.stdout;
  const launchFailure = (error: unknown): never =>
    fail(`update failed: could not run installer: ${error instanceof Error ? error.message : String(error)}`);

  let bytes: Buffer;
  let dir: string;
  try {
    bytes = readFileSync(plan.installer);
    const parent = realpathSync(options.tempRoot ?? tmpdir());
    const current = pathApi(windows).join(plan.appRoot, "current");
    if (ancestors(parent, windows).some((path) => samePath(path, current, windows))) {
      fail("update failed: could not run installer: temporary directory is inside current");
    }
    dir = mkdtempSync(join(parent, "envswitch-update-"));
  } catch (error) {
    if (error instanceof EnvswitchError) {
      throw error;
    }
    return launchFailure(error);
  }

  let code: number | undefined;
  try {
    const script = join(dir, windows ? "install.ps1" : "install.sh");
    try {
      if (windows) {
        restrictWindowsDirectory(dir, env);
      }
      writeFileSync(script, bytes, { flag: "wx", mode: 0o600 });
    } catch (error) {
      if (error instanceof EnvswitchError) {
        throw error;
      }
      launchFailure(error);
    }

    const invocation = buildInstallerInvocation(plan, script, env, windows);
    let child: ReturnType<typeof spawnInstaller>;
    try {
      child = spawnInstaller(invocation, dir);
    } catch (error) {
      // Like the TypeScript oracle, a spawn failure is not a controlled error.
      throw new Error(`update failed: could not run installer: ${error instanceof Error ? error.message : String(error)}`);
    }

    // The terminal delivers Ctrl-C to the installer too; wait for its rollback
    // instead of exiting first, and forward a TERM sent to this process.
    const ignore = () => {};
    const forward = () => {
      child.kill("SIGTERM");
    };
    process.on("SIGINT", ignore);
    process.on("SIGTERM", forward);
    const chunks: Uint8Array[] = [];
    try {
      for await (const chunk of child.stdout) {
        chunks.push(chunk);
        try {
          out.write(chunk);
        } catch {
          // Keep capturing for the result even if forwarding fails.
        }
      }
    } catch {
      // An unreadable stdout leaves an unaccepted result; still wait below.
    } finally {
      // Never remove the private script while the installer is running.
      await child.exited;
      process.off("SIGINT", ignore);
      process.off("SIGTERM", forward);
    }
    const signal = child.signalCode;
    code = signal
      ? 128 + (osConstants.signals[signal as keyof typeof osConstants.signals] ?? 0)
      : (child.exitCode ?? 1);
    return { code, stdout: Buffer.concat(chunks).toString("utf8") };
  } finally {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch (error) {
      if (code === 0) {
        console.error(
          `envswitch: warning: could not remove installer temporary directory ${dir}: ${error instanceof Error ? error.message : String(error)}`,
        );
      }
    }
  }
}

function spawnInstaller(invocation: { argv: string[]; env: Record<string, string> }, cwd: string) {
  return Bun.spawn(invocation.argv, {
    cwd,
    env: invocation.env,
    stdin: "inherit",
    stdout: "pipe",
    stderr: "inherit",
  });
}

/** Owner-only ACL (plus SYSTEM and Administrators) before writing the script. */
function restrictWindowsDirectory(dir: string, env: UpdateEnv): void {
  const script =
    "$ErrorActionPreference='Stop'; $p=$env:ENVSWITCH_INSTALLER_TEMP; $a=New-Object System.Security.AccessControl.DirectorySecurity; $a.SetAccessRuleProtection($true,$false); $u=[System.Security.Principal.WindowsIdentity]::GetCurrent().User; foreach($s in @($u,(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'),(New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'))) { $r=New-Object System.Security.AccessControl.FileSystemAccessRule($s,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $a.AddAccessRule($r) }; Set-Acl -LiteralPath $p -AclObject $a";
  const childEnv: Record<string, string> = {};
  for (const [name, value] of Object.entries(env)) {
    if (value !== undefined) {
      childEnv[name] = value;
    }
  }
  childEnv.ENVSWITCH_INSTALLER_TEMP = dir;
  const result = Bun.spawnSync(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script], {
    env: childEnv,
    stdout: "ignore",
    stderr: "ignore",
  });
  if (result.exitCode !== 0) {
    fail("update failed: could not run installer: could not secure temporary directory");
  }
}

class JsonNumber {
  constructor(readonly raw: string) {}
}

type JsonValue = null | boolean | string | JsonNumber | JsonValue[] | Map<string, JsonValue>;

/** Strict RFC 8259 JSON with duplicate-key rejection and raw number tokens. */
function parseStrictJson(text: string): JsonValue | undefined {
  let pos = 0;
  const invalid = (): never => {
    throw new SyntaxError("invalid JSON");
  };
  const skipWhitespace = () => {
    while (pos < text.length && " \t\r\n".includes(text[pos]!)) {
      pos += 1;
    }
  };
  const parseString = (): string => {
    const start = pos;
    if (text[pos] !== '"') {
      invalid();
    }
    pos += 1;
    while (pos < text.length && text[pos] !== '"') {
      pos += text[pos] === "\\" ? 2 : 1;
    }
    if (pos >= text.length) {
      invalid();
    }
    pos += 1;
    return JSON.parse(text.slice(start, pos)) as string;
  };
  const number = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y;
  const parseValue = (depth: number): JsonValue => {
    if (depth > 64) {
      invalid();
    }
    skipWhitespace();
    const c = text[pos];
    if (c === "{") {
      pos += 1;
      const object = new Map<string, JsonValue>();
      skipWhitespace();
      if (text[pos] === "}") {
        pos += 1;
        return object;
      }
      for (;;) {
        skipWhitespace();
        const key = parseString();
        if (object.has(key)) {
          invalid();
        }
        skipWhitespace();
        if (text[pos] !== ":") {
          invalid();
        }
        pos += 1;
        object.set(key, parseValue(depth + 1));
        skipWhitespace();
        const separator = text[pos];
        pos += 1;
        if (separator === "}") {
          return object;
        }
        if (separator !== ",") {
          invalid();
        }
      }
    }
    if (c === "[") {
      pos += 1;
      const array: JsonValue[] = [];
      skipWhitespace();
      if (text[pos] === "]") {
        pos += 1;
        return array;
      }
      for (;;) {
        array.push(parseValue(depth + 1));
        skipWhitespace();
        const separator = text[pos];
        pos += 1;
        if (separator === "]") {
          return array;
        }
        if (separator !== ",") {
          invalid();
        }
      }
    }
    if (c === '"') {
      return parseString();
    }
    for (const [literal, value] of [["true", true], ["false", false], ["null", null]] as const) {
      if (text.startsWith(literal, pos)) {
        pos += literal.length;
        return value;
      }
    }
    number.lastIndex = pos;
    const match = number.exec(text);
    if (!match) {
      invalid();
    }
    pos = number.lastIndex;
    return new JsonNumber(match![0]);
  };
  try {
    const value = parseValue(0);
    skipWhitespace();
    return pos === text.length ? value : undefined;
  } catch {
    return undefined;
  }
}

function pathApi(windows: boolean): typeof posix {
  return windows ? win32 : posix;
}

function launcherName(windows: boolean): string {
  return windows ? "envswitch.cmd" : "envswitch";
}

function pathEnv(env: UpdateEnv, windows: boolean): string | undefined {
  if (!windows) {
    return env.PATH;
  }
  const key = Object.keys(env).find((name) => name.toUpperCase() === "PATH");
  return key === undefined ? undefined : env[key];
}

function nonEmpty(value: string | undefined): string | undefined {
  return value === undefined || value === "" ? undefined : value;
}

function validRepo(repo: string): boolean {
  if (!REPO_PATTERN.test(repo)) {
    return false;
  }
  return repo.split("/").every((part) => part !== "." && part !== "..");
}

/** Paths compare exactly on Unix and ordinal case-insensitively on Windows. */
function samePath(a: string, b: string, windows: boolean): boolean {
  return windows ? a.length === b.length && a.toUpperCase() === b.toUpperCase() : a === b;
}

/** The path itself and each parent up to the filesystem root. */
function ancestors(path: string, windows: boolean): string[] {
  const paths = pathApi(windows);
  const result = [path];
  for (let parent = paths.dirname(path); parent !== result.at(-1); parent = paths.dirname(parent)) {
    result.push(parent);
  }
  return result;
}

/** Effective, normalized absolute path rules shared with the installers (§2.1, §4.1). */
function validInstallPath(path: string, windows: boolean): boolean {
  if (CONTROL_CHARACTERS.test(path)) {
    return false;
  }
  if (windows) {
    if (!/^[A-Za-z]:\\/.test(path) || path.includes("/") || path.slice(2).includes(":") || path.includes('"')) {
      return false;
    }
    const parts = path.split("\\").slice(1);
    return parts.length > 0 && parts.every((part) => part !== "" && !part.endsWith(".") && !part.endsWith(" "));
  }
  if (!path.startsWith("/") || path.includes(":") || path.includes("..")) {
    return false;
  }
  const parts = path.split("/").slice(1);
  return parts.length > 0 && parts.every((part) => part !== "" && part !== ".");
}

/** Lexically normalize an explicit Unix INSTALL_DIR the way the installer does. */
function normalizeUnixDirectory(dir: string): string | undefined {
  if (!dir.startsWith("/") || dir.includes("..") || dir.includes(":") || CONTROL_CHARACTERS.test(dir)) {
    return undefined;
  }
  const parts = dir.split("/").filter((part) => part !== "" && part !== ".");
  return parts.length === 0 ? undefined : `/${parts.join("/")}`;
}

/** No existing component of the path is a symbolic link or reparse point. */
function hasNoLinks(path: string, windows: boolean): boolean {
  for (const entry of ancestors(path, windows)) {
    try {
      if (lstatSync(entry).isSymbolicLink()) {
        return false;
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        return false;
      }
    }
  }
  return true;
}

function pathEntryExists(path: string): boolean {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

function isRegularFile(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function printSwitchResults(
  results: Awaited<ReturnType<typeof switchEnvironment>>,
  root: string,
  dryRun: boolean,
  writeLine: (line: string) => void,
): void {
  const action = dryRun ? "Dry run" : "Updated";
  writeLine(`${action} ${results.length} target(s) from ${basename(root)}`);
  for (const result of results) {
    const layerTitles = result.layers.map((layer) => `${layer.name}:${layer.item.title}`).join(", ");
    writeLine(`- ${result.target.name} -> ${result.outputPath}`);
    writeLine(`  keys: ${result.keyCount}`);
    writeLine(`  layers: ${layerTitles || "none"}`);
    if (result.overwrittenKeys.length > 0) {
      writeLine(`  overwritten: ${result.overwrittenKeys.join(", ")}`);
    }
  }
}

async function loadConfig(
  op: Pick<OnePasswordClient, "readConfigItem">,
  configItem: string,
  log?: (event: ProgressEvent) => void,
  logName = configItem,
): Promise<EnvswitchConfig> {
  log?.({ status: "info", message: `reading config item ${logName}` });
  const config = parseConfig(await op.readConfigItem(configItem));
  log?.({ status: "done", message: `loaded config item ${logName}` });
  return config;
}

function printLayers(config: EnvswitchConfig, writeLine: (line: string) => void): void {
  for (const layer of LAYERS) {
    const tags = tagValues(config, { layer });
    writeLine(`${layer}\t${tags.layer ?? ""}`);
  }
}

function printList(items: string[], writeLine: (line: string) => void): void {
  if (items.length === 0) {
    writeLine("(none)");
    return;
  }
  for (const item of items) {
    writeLine(item);
  }
}

function createCliDeps(options: MainOptions): CliDeps {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const env = options.env ?? process.env;
  return {
    input,
    output,
    prompt: options.prompt,
    isInteractive: options.isInteractive ?? isInteractiveTerminal(input, output),
    root: options.root ?? process.cwd(),
    listVaults: options.listVaults ?? (() => OnePasswordClient.listVaults()),
    createOnePasswordClient: options.createOnePasswordClient ?? ((vault) => new OnePasswordClient(vault)),
    log: options.log ?? logProgress,
    writeLine: options.writeLine ?? ((line) => console.log(line)),
    appDir: options.appDir ?? resolve(import.meta.dir, ".."),
    env,
    fetchLatestVersion: options.fetchLatestVersion ?? ((repo) => fetchLatestReleaseTag(repo, { env })),
    runInstaller: options.runInstaller ?? ((plan) => runInstallScript(plan, { env })),
  };
}

function createInquirerPrompt(input: Readable, output: Writable): GuidedPrompt {
  return {
    select: async <T>(options: {
      message: string;
      choices: Array<ChoiceOption<T>>;
      defaultValue?: T | undefined;
    }) => {
      const config: {
        message: string;
        choices: Array<{ name: string; value: T }>;
        default?: T;
        loop: boolean;
      } = {
        message: options.message,
        choices: options.choices.map((choice) => ({
          name: choice.label,
          value: choice.value,
        })),
        loop: false,
      };
      if (options.defaultValue !== undefined) {
        config.default = options.defaultValue;
      }
      return inquirerSelect(config, { input, output });
    },
    input: async (options) => {
      const config: {
        message: string;
        default?: string;
        required?: boolean;
      } = {
        message: options.message,
      };
      if (options.defaultValue !== undefined) {
        config.default = options.defaultValue;
      }
      if (options.required !== undefined) {
        config.required = options.required;
      }
      return inquirerInput(config, { input, output });
    },
  };
}

function isInteractiveTerminal(input: Readable, output: Writable): boolean {
  return Boolean(
    (input as Readable & { isTTY?: boolean }).isTTY &&
      (output as Writable & { isTTY?: boolean }).isTTY,
  );
}

async function findConfigVault(
  vault: OnePasswordVaultSummary,
  deps: Pick<CliDeps, "createOnePasswordClient" | "log">,
): Promise<ConfigVaultChoice | undefined> {
  const vaultId = vault.id || vault.name;
  const op = deps.createOnePasswordClient(vaultId);
  try {
    const configItems = await op.listItemsByTags([CONFIG_ITEM_TAG]);
    if (configItems.length === 0) {
      return undefined;
    }
    return { vault, vaultId, op, configItems };
  } catch {
    deps.log({ status: "info", message: `skipping vault ${vault.name} (could not list items)` });
    return undefined;
  }
}

async function askVault(
  prompt: GuidedPrompt,
  deps: Pick<CliDeps, "createOnePasswordClient" | "listVaults" | "log">,
): Promise<ConfigVaultChoice> {
  deps.log({ status: "info", message: "listing 1Password vaults" });
  const vaults = await deps.listVaults();
  deps.log({ status: "done", message: `found ${vaults.length} vault(s)` });
  if (vaults.length === 0) {
    fail("no 1Password vaults found");
  }

  deps.log({ status: "info", message: `checking vaults for config items tagged ${CONFIG_ITEM_TAG}` });
  const configVaults = (await Promise.all(vaults.map((vault) => findConfigVault(vault, deps))))
    .filter((choice): choice is ConfigVaultChoice => choice !== undefined);
  deps.log({ status: "done", message: `found ${configVaults.length} vault(s) with config items` });
  if (configVaults.length === 0) {
    fail(`no 1Password vaults contain items tagged ${CONFIG_ITEM_TAG}`);
  }

  const selectedVaultId = await selectOrAuto(prompt, {
    message: "Vault",
    choices: configVaults.map((choice) => ({
      label: formatVaultLabel(choice.vault),
      value: choice.vaultId,
    })),
  });
  return configVaults.find((choice) => choice.vaultId === selectedVaultId)!;
}

async function askConfigItem(
  prompt: GuidedPrompt,
  items: OnePasswordItemSummary[],
): Promise<ConfigItemChoice> {
  if (items.length === 0) {
    fail(`no config items tagged ${CONFIG_ITEM_TAG} found in the selected vault`);
  }

  const selectedItemId = await selectOrAuto(prompt, {
    message: "Config item",
    choices: items.map((item) => ({
      label: formatItemLabel(item),
      value: item.id || item.title,
    })),
  });
  const selectedItem = items.find((item) => (item.id || item.title) === selectedItemId);
  return {
    itemId: selectedItemId,
    logName: selectedItem ? configItemLogName(selectedItem) : selectedItemId,
  };
}

async function askChoiceOrValue(
  prompt: GuidedPrompt,
  options: { label: string; choices: string[]; defaultValue?: string | undefined },
): Promise<string> {
  if (options.choices.length === 0) {
    return prompt.input({
      message: options.label,
      defaultValue: options.defaultValue,
      required: true,
    });
  }

  return selectOrAuto(prompt, {
    message: options.label,
    choices: options.choices.map((choice) => ({ label: choice, value: choice })),
    defaultValue: options.defaultValue,
  });
}

async function askTarget(prompt: GuidedPrompt, config: EnvswitchConfig): Promise<string> {
  if (config.targets.length === 1) {
    return DEFAULT_TARGET;
  }

  return selectOrAuto(prompt, {
    message: "Target",
    choices: [DEFAULT_TARGET, ...config.targets.map((target) => target.name)].map((choice) => ({
      label: choice,
      value: choice,
    })),
    defaultValue: DEFAULT_TARGET,
  });
}

async function selectOrAuto<T>(
  prompt: GuidedPrompt,
  options: {
    message: string;
    choices: Array<ChoiceOption<T>>;
    defaultValue?: T | undefined;
  },
): Promise<T> {
  if (options.choices.length === 1) {
    return options.choices[0]!.value;
  }

  return prompt.select(options);
}

async function askBoolean(prompt: GuidedPrompt, defaultValue: boolean): Promise<boolean> {
  return prompt.select({
    message: "Mode",
    choices: [
      { label: "Dry run", value: true },
      { label: "Write .env files", value: false },
    ],
    defaultValue,
  });
}

function defaultEnvironment(config: EnvswitchConfig): string {
  const environments = config.environments ?? [];
  if (environments.length === 0 || environments.includes(DEFAULT_ENV)) {
    return DEFAULT_ENV;
  }
  return environments[0] ?? DEFAULT_ENV;
}

function formatVaultLabel(vault: OnePasswordVaultSummary): string {
  if (!vault.id || vault.id === vault.name) {
    return vault.name;
  }
  return `${vault.name} (${vault.id})`;
}

function formatItemLabel(item: OnePasswordItemSummary): string {
  if (!item.id || item.id === item.title) {
    return item.title || item.id;
  }
  return `${item.title || item.id} (${item.id})`;
}

function configItemLogName(item: OnePasswordItemSummary): string {
  return item.title || item.id;
}

function isPromptCancelError(error: unknown): boolean {
  return error instanceof Error && error.name === "ExitPromptError";
}

function requireStringFlag(args: ParsedArgs, name: string): string {
  const value = stringFlag(args, name, "");
  if (value === "") {
    fail(`--${name} is required`);
  }
  return value;
}

function validateFlags(args: ParsedArgs, allowed: Set<string>): void {
  for (const [name, value] of Object.entries(args.flags)) {
    if (!allowed.has(name)) {
      fail(`unknown flag: --${name}`);
    }
    if (BOOLEAN_FLAGS.has(name) && value !== true) {
      fail(`--${name} does not take a value`);
    }
  }
}

function stringFlag(args: ParsedArgs, name: string, defaultValue: string): string {
  const value = args.flags[name];
  if (value === undefined) {
    return defaultValue;
  }
  if (typeof value !== "string" || value.trim() === "") {
    fail(`--${name} requires a value`);
  }
  return value;
}

function hasFlag(args: ParsedArgs, name: string): boolean {
  return args.flags[name] === true;
}

function printHelp(writeLine: (line: string) => void): void {
  writeLine(`envswitch

Usage:
  envswitch
  envswitch switch --vault <vault> --config-item <item> --client <client> [--env local] [--target all|name] [--dry-run]
  envswitch list --vault <vault> --config-item <item> [clients|targets|layers]
  envswitch doctor --vault <vault> --config-item <item>
  envswitch update

Notes:
  Run envswitch with no arguments for guided switch prompts.
  Run envswitch update to reinstall the latest GitHub release in place.
  Run envswitch from the root of the project you want to update.
  Project metadata, target paths, and tag rules are loaded from the 1Password config item.
`);
}

if (import.meta.main) {
  process.exit(await main());
}
