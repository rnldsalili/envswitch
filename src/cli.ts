#!/usr/bin/env bun
import { basename } from "node:path";
import { parseConfig, tagValues } from "./config.ts";
import { EnvswitchError, fail } from "./errors.ts";
import { OnePasswordClient } from "./onepassword.ts";
import { inspectProject, switchEnvironment } from "./switcher.ts";
import type { EnvswitchConfig, LayerName, ProgressEvent, ProgressStatus } from "./types.ts";

interface ParsedArgs {
  command: string | undefined;
  positional: string[];
  flags: Record<string, string | boolean>;
}

const DEFAULT_ENV = "local";
const DEFAULT_TARGET = "all";
const LAYERS: LayerName[] = ["shared", "group", "target", "client", "client-target"];
const SWITCH_FLAGS = new Set(["vault", "config-item", "client", "env", "target", "dry-run", "help"]);
const LIST_FLAGS = new Set(["vault", "config-item", "help"]);
const DOCTOR_FLAGS = new Set(["vault", "config-item", "help"]);
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

export async function main(argv = Bun.argv.slice(2)): Promise<number> {
  try {
    if (argv.length === 0 || argv[0] === "--help" || argv[0] === "-h") {
      printHelp();
      return 0;
    }

    const args = parseArgs(argv);
    if (hasFlag(args, "help") || args.command === "help") {
      printHelp();
      return 0;
    }

    switch (args.command) {
      case "switch":
        await runSwitch(args);
        return 0;
      case "list":
        await runList(args);
        return 0;
      case "doctor":
        await runDoctor(args);
        return 0;
      default:
        fail(`unknown command: ${args.command}`);
    }
  } catch (error) {
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

async function runSwitch(args: ParsedArgs): Promise<void> {
  validateFlags(args, SWITCH_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");
  const client = requireStringFlag(args, "client");
  const env = stringFlag(args, "env", DEFAULT_ENV);
  const targetName = stringFlag(args, "target", DEFAULT_TARGET);
  const dryRun = hasFlag(args, "dry-run");
  const root = process.cwd();

  const op = new OnePasswordClient(vault);
  const config = await loadConfig(op, configItem, logProgress);
  const results = await switchEnvironment(config, op, {
    root,
    env,
    client,
    targetName,
    dryRun,
    log: logProgress,
  });

  const action = dryRun ? "Dry run" : "Updated";
  console.log(`${action} ${results.length} target(s) from ${basename(root)}`);
  for (const result of results) {
    const layerTitles = result.layers.map((layer) => `${layer.name}:${layer.item.title}`).join(", ");
    console.log(`- ${result.target.name} -> ${result.outputPath}`);
    console.log(`  keys: ${result.keyCount}`);
    console.log(`  layers: ${layerTitles || "none"}`);
    if (result.overwrittenKeys.length > 0) {
      console.log(`  overwritten: ${result.overwrittenKeys.join(", ")}`);
    }
  }
}

async function runList(args: ParsedArgs): Promise<void> {
  validateFlags(args, LIST_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");
  const subject = args.positional[0] ?? "targets";

  const op = new OnePasswordClient(vault);
  const config = await loadConfig(op, configItem, logProgress);

  switch (subject) {
    case "clients":
      printList(config.clients ?? []);
      return;
    case "targets":
      printList(config.targets.map((target) => `${target.name}\t${target.output}`));
      return;
    case "layers":
      printLayers(config);
      return;
    default:
      fail(`unknown list subject: ${subject}`);
  }
}

async function runDoctor(args: ParsedArgs): Promise<void> {
  validateFlags(args, DOCTOR_FLAGS);
  const vault = requireStringFlag(args, "vault");
  const configItem = requireStringFlag(args, "config-item");
  const root = process.cwd();

  const op = new OnePasswordClient(vault);
  logProgress({ status: "info", message: "checking 1Password authentication" });
  await op.assertAuthenticated();
  logProgress({ status: "done", message: "1Password authentication ok" });
  const config = await loadConfig(op, configItem, logProgress);
  logProgress({ status: "info", message: `inspecting project root ${root}` });
  const targets = await inspectProject(config, root);

  console.log("1Password authentication: ok");
  console.log(`project root: ${root}`);
  console.log(`targets: ${targets.length}`);
  for (const target of targets) {
    const status = target.parentExists ? "ok" : "missing parent";
    console.log(`- ${target.target.name} -> ${target.outputPath} (${status})`);
  }
}

async function loadConfig(
  op: OnePasswordClient,
  configItem: string,
  log?: (event: ProgressEvent) => void,
): Promise<EnvswitchConfig> {
  log?.({ status: "info", message: `reading config item ${configItem}` });
  const config = parseConfig(await op.readConfigItem(configItem));
  log?.({ status: "done", message: `loaded config item ${configItem}` });
  return config;
}

function printLayers(config: EnvswitchConfig): void {
  for (const layer of LAYERS) {
    const tags = tagValues(config, { layer });
    console.log(`${layer}\t${tags.layer ?? ""}`);
  }
}

function printList(items: string[]): void {
  if (items.length === 0) {
    console.log("(none)");
    return;
  }
  for (const item of items) {
    console.log(item);
  }
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

function printHelp(): void {
  console.log(`envswitch

Usage:
  envswitch switch --vault <vault> --config-item <item> --client <client> [--env local] [--target all|name] [--dry-run]
  envswitch list --vault <vault> --config-item <item> [clients|targets|layers]
  envswitch doctor --vault <vault> --config-item <item>

Notes:
  Run envswitch from the root of the project you want to update.
  Project metadata, target paths, and tag rules are loaded from the 1Password config item.
`);
}

if (import.meta.main) {
  process.exit(await main());
}
