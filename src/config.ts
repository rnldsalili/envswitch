import { basename, isAbsolute, join, normalize, relative, resolve } from "node:path";
import { readFile, stat } from "node:fs/promises";
import { fail } from "./errors.ts";
import { isRecord, parseJson, requireRecord } from "./validation.ts";
import type {
  EnvswitchConfig,
  LayerName,
  ResolvedTarget,
  RootMarker,
  TagTemplates,
  TargetConfig,
} from "./types.ts";

const DEFAULT_TAGS: Required<TagTemplates> = {
  base: [],
  project: "project:{project}",
  env: "env:{env}",
  client: "client:{client}",
  target: "target:{target}",
  group: "group:{group}",
  layer: "layer:{layer}",
};

const IDENTIFIER_RE = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;

export function parseConfig(raw: string): EnvswitchConfig {
  const parsed = requireRecord(parseJson(raw, "config field"), "config JSON must be an object");

  const targets = parsed.targets;
  if (!Array.isArray(targets) || targets.length === 0) {
    fail("config.targets must be a non-empty array");
  }

  const config: EnvswitchConfig = {
    targets: targets.map((target, index) => parseTarget(target, index)),
  };
  const project = parseProject(parsed.project);
  const clients = parseStringArray(parsed.clients, "config.clients", false);
  const environments = parseStringArray(parsed.environments, "config.environments", false);
  const tags = parseTags(parsed.tags);
  if (project !== undefined) {
    config.project = project;
  }
  if (clients !== undefined) {
    config.clients = clients;
  }
  if (environments !== undefined) {
    config.environments = environments;
  }
  if (tags !== undefined) {
    config.tags = tags;
  }

  const names = new Set<string>();
  for (const target of config.targets) {
    if (names.has(target.name)) {
      fail(`config.targets contains duplicate target ${target.name}`);
    }
    names.add(target.name);
  }

  return config;
}

export async function validateProjectRoot(config: EnvswitchConfig, root: string): Promise<void> {
  const markers = config.project?.rootMarkers ?? [];
  for (const marker of markers) {
    await validateMarker(root, marker);
  }
}

export function validateSelection(config: EnvswitchConfig, env: string, client: string): void {
  validateIdentifier(env, "environment");
  validateIdentifier(client, "client");

  if (config.environments && config.environments.length > 0 && !config.environments.includes(env)) {
    fail(`environment ${env} is not listed in config.environments`);
  }

  if (config.clients && config.clients.length > 0 && !config.clients.includes(client)) {
    fail(`client ${client} is not listed in config.clients`);
  }
}

export function selectTargets(config: EnvswitchConfig, targetName: string): TargetConfig[] {
  if (targetName === "all") {
    return config.targets;
  }

  validateIdentifier(targetName, "target");
  const target = config.targets.find((candidate) => candidate.name === targetName);
  if (!target) {
    fail(`target ${targetName} is not defined in config.targets`);
  }
  return [target];
}

export function resolveTargets(root: string, targets: TargetConfig[]): ResolvedTarget[] {
  return targets.map((target) => ({
    target,
    outputPath: resolveOutputPath(root, target.output),
  }));
}

export function resolveOutputPath(root: string, output: string): string {
  if (output.trim() === "") {
    fail("target output path cannot be empty");
  }
  if (isAbsolute(output)) {
    fail(`target output path must be relative to the project root: ${output}`);
  }

  const rootAbs = resolve(root);
  const outputAbs = resolve(rootAbs, normalize(output));
  const rel = relative(rootAbs, outputAbs);
  if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) {
    fail(`target output path escapes the project root: ${output}`);
  }

  return outputAbs;
}

export function tagValues(
  config: EnvswitchConfig,
  params: {
    env?: string;
    client?: string;
    target?: string;
    group?: string;
    layer?: LayerName;
  },
): {
  base: string[];
  project: string | undefined;
  env: string | undefined;
  client: string | undefined;
  target: string | undefined;
  group: string | undefined;
  layer: string | undefined;
} {
  const templates = { ...DEFAULT_TAGS, ...(config.tags ?? {}) };
  const project = config.project?.id ?? basename(process.cwd());
  const context = {
    project,
    env: params.env ?? "",
    client: params.client ?? "",
    target: params.target ?? "",
    group: params.group ?? "",
    layer: params.layer ?? "",
  };

  return {
    base: templates.base.map((template) => renderTemplate(template, context)).filter(Boolean),
    project: config.project?.id ? renderTemplate(templates.project, context) : undefined,
    env: params.env ? renderTemplate(templates.env, context) : undefined,
    client: params.client ? renderTemplate(templates.client, context) : undefined,
    target: params.target ? renderTemplate(templates.target, context) : undefined,
    group: params.group ? renderTemplate(templates.group, context) : undefined,
    layer: params.layer ? renderTemplate(templates.layer, context) : undefined,
  };
}

export function candidateListTags(config: EnvswitchConfig, env: string): string[] {
  const tags = tagValues(config, { env });
  return compactUnique([...tags.base, tags.project, tags.env]);
}

export function compactUnique(values: Array<string | undefined>): string[] {
  return Array.from(new Set(values.filter((value): value is string => Boolean(value))));
}

function parseProject(value: unknown): EnvswitchConfig["project"] {
  if (value === undefined) {
    return undefined;
  }
  if (!isRecord(value)) {
    fail("config.project must be an object");
  }

  const rawId = value.id;
  if (rawId !== undefined && typeof rawId !== "string") {
    fail("config.project.id must be a string");
  }
  if (rawId) {
    validateIdentifier(rawId, "project id");
  }

  let rootMarkers: RootMarker[] | undefined;
  if (value.rootMarkers !== undefined) {
    if (!Array.isArray(value.rootMarkers)) {
      fail("config.project.rootMarkers must be an array");
    }
    rootMarkers = value.rootMarkers.map((marker, index) => parseRootMarker(marker, index));
  }

  const project: NonNullable<EnvswitchConfig["project"]> = {};
  if (rawId !== undefined) {
    project.id = rawId;
  }
  if (rootMarkers !== undefined) {
    project.rootMarkers = rootMarkers;
  }
  return project;
}

function parseRootMarker(value: unknown, index: number): RootMarker {
  if (!isRecord(value)) {
    fail(`config.project.rootMarkers[${index}] must be an object`);
  }

  const marker: RootMarker = {};
  if (value.path !== undefined) {
    if (typeof value.path !== "string" || value.path.trim() === "") {
      fail(`config.project.rootMarkers[${index}].path must be a non-empty string`);
    }
    marker.path = value.path;
  }
  if (value.packageJsonName !== undefined) {
    if (typeof value.packageJsonName !== "string" || value.packageJsonName.trim() === "") {
      fail(`config.project.rootMarkers[${index}].packageJsonName must be a non-empty string`);
    }
    marker.packageJsonName = value.packageJsonName;
  }
  if (!marker.path && !marker.packageJsonName) {
    fail(`config.project.rootMarkers[${index}] must define path or packageJsonName`);
  }
  return marker;
}

function parseTarget(value: unknown, index: number): TargetConfig {
  if (!isRecord(value)) {
    fail(`config.targets[${index}] must be an object`);
  }
  if (typeof value.name !== "string" || value.name.trim() === "") {
    fail(`config.targets[${index}].name must be a non-empty string`);
  }
  if (typeof value.output !== "string" || value.output.trim() === "") {
    fail(`config.targets[${index}].output must be a non-empty string`);
  }

  validateIdentifier(value.name, `target ${value.name}`);

  const target: TargetConfig = {
    name: value.name,
    output: value.output,
  };
  const groups = parseStringArray(value.groups, `config.targets[${index}].groups`, false);
  if (groups) {
    for (const group of groups) {
      validateIdentifier(group, `target ${value.name} group ${group}`);
    }
    target.groups = groups;
  }

  return target;
}

function parseTags(value: unknown): TagTemplates | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!isRecord(value)) {
    fail("config.tags must be an object");
  }

  const tags: TagTemplates = {};
  if (value.base !== undefined) {
    const baseTags = parseStringArray(value.base, "config.tags.base", true);
    if (baseTags !== undefined) {
      tags.base = baseTags;
    }
  }
  for (const key of ["project", "env", "client", "target", "group", "layer"] as const) {
    if (value[key] !== undefined) {
      if (typeof value[key] !== "string" || value[key].trim() === "") {
        fail(`config.tags.${key} must be a non-empty string`);
      }
      tags[key] = value[key];
    }
  }
  return tags;
}

function parseStringArray(value: unknown, name: string, required: boolean): string[] | undefined {
  if (value === undefined) {
    if (required) {
      fail(`${name} is required`);
    }
    return undefined;
  }
  if (!Array.isArray(value)) {
    fail(`${name} must be an array`);
  }
  const result: string[] = [];
  for (const [index, item] of value.entries()) {
    if (typeof item !== "string" || item.trim() === "") {
      fail(`${name}[${index}] must be a non-empty string`);
    }
    result.push(item);
  }
  return result;
}

function validateIdentifier(value: string, name: string): void {
  if (!IDENTIFIER_RE.test(value)) {
    fail(`${name} contains unsupported characters`);
  }
}

async function validateMarker(root: string, marker: RootMarker): Promise<void> {
  if (marker.path) {
    const markerPath = resolveOutputPath(root, marker.path);
    if (!(await pathExists(markerPath))) {
      fail(`project root marker path does not exist: ${marker.path}`);
    }
  }

  if (marker.packageJsonName) {
    const packageJsonPath = join(root, "package.json");
    if (!(await pathExists(packageJsonPath))) {
      fail("project root marker packageJsonName requires package.json in the current directory");
    }
    let packageJson: unknown;
    try {
      packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
    } catch (error) {
      fail(`failed to read package.json for project root validation: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (!isRecord(packageJson) || packageJson.name !== marker.packageJsonName) {
      fail(`current directory package.json name does not match ${marker.packageJsonName}`);
    }
  }
}

function renderTemplate(template: string, context: Record<string, string>): string {
  return template.replace(/\{([A-Za-z0-9_-]+)\}/g, (_, key: string) => context[key] ?? "");
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}
