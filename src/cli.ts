#!/usr/bin/env bun
import { input as inquirerInput, select as inquirerSelect } from "@inquirer/prompts";
import { existsSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
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
  fetchLatestVersion?: () => Promise<string | undefined>;
  runInstaller?: (scriptPath: string, appRoot: string) => Promise<number>;
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
  fetchLatestVersion: () => Promise<string | undefined>;
  runInstaller: (scriptPath: string, appRoot: string) => Promise<number>;
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
const GITHUB_REPO = "rnldsalili/envswitch";
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

async function runUpdate(args: ParsedArgs, deps: CliDeps): Promise<void> {
  validateFlags(args, UPDATE_FLAGS);

  if (existsSync(join(deps.appDir, ".git"))) {
    fail("envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)");
  }

  const installerName = process.platform === "win32" ? "install.ps1" : "install.sh";
  const installer = join(deps.appDir, "scripts", installerName);
  if (!existsSync(installer)) {
    fail(`installer not found at ${installer}; reinstall envswitch with the install script from the README`);
  }

  const versionFile = join(deps.appDir, ".version");
  const current = await readVersionFile(versionFile);
  const latest = await deps.fetchLatestVersion();
  if (latest && current && normalizeVersion(latest) === normalizeVersion(current)) {
    deps.writeLine(`envswitch is already up to date (${current})`);
    return;
  }

  deps.log({
    status: "info",
    message: `updating envswitch ${current ?? "(unknown version)"} -> ${latest ?? "latest"}`,
  });
  const code = await deps.runInstaller(installer, dirname(deps.appDir));
  if (code !== 0) {
    fail(`update failed: installer exited with code ${code}`);
  }

  const updated = await readVersionFile(versionFile);
  deps.writeLine(`updated envswitch ${current ?? "(unknown)"} -> ${updated ?? latest ?? "latest"}`);
}

async function readVersionFile(path: string): Promise<string | undefined> {
  try {
    const version = (await Bun.file(path).text()).trim();
    return version === "" ? undefined : version;
  } catch {
    return undefined;
  }
}

function normalizeVersion(version: string): string {
  return version.replace(/^v/, "");
}

async function fetchLatestReleaseTag(): Promise<string | undefined> {
  try {
    const headers: Record<string, string> = { Accept: "application/vnd.github+json" };
    if (process.env.GITHUB_TOKEN) {
      headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
    }
    const response = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`, { headers });
    if (!response.ok) {
      return undefined;
    }
    const body = (await response.json()) as { tag_name?: unknown };
    return typeof body.tag_name === "string" && body.tag_name !== "" ? body.tag_name : undefined;
  } catch {
    return undefined;
  }
}

async function runInstallScript(scriptPath: string, appRoot: string): Promise<number> {
  const command = scriptPath.endsWith(".ps1")
    ? ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath, "-AppRoot", appRoot]
    : ["bash", scriptPath];
  const proc = Bun.spawn(command, {
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    env: { ...process.env, APP_ROOT: appRoot },
  });
  return proc.exited;
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
    fetchLatestVersion: options.fetchLatestVersion ?? fetchLatestReleaseTag,
    runInstaller: options.runInstaller ?? runInstallScript,
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
