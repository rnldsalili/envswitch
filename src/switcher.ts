import { existsSync } from "node:fs";
import { dirname } from "node:path";
import {
  candidateListTags,
  compactUnique,
  resolveTargets,
  selectTargets,
  tagValues,
  validateProjectRoot,
  validateSelection,
} from "./config.ts";
import { writeEnvFile } from "./dotenv.ts";
import type { OnePasswordClient } from "./onepassword.ts";
import type {
  EnvswitchConfig,
  EnvLayer,
  LayerName,
  MergeResult,
  OnePasswordItemSummary,
  ProgressEvent,
  ResolvedTarget,
  TargetConfig,
} from "./types.ts";

const LAYER_ORDER: LayerName[] = ["shared", "group", "target", "client", "client-target"];

type OnePasswordEnvironmentClient = Pick<OnePasswordClient, "listItemsByTags" | "readEnvItem">;

interface TagMatcher {
  commonTags: string[];
  allTargetTags: Set<string>;
  allClientTags: Set<string>;
  allGroupTags: Set<string>;
  currentClientTag: string | undefined;
  currentTargetTagsByTarget: Map<string, Set<string>>;
  currentGroupTagsByTarget: Map<string, Set<string>>;
  layerTags: Map<LayerName, string>;
}

interface MergeTargetParams {
  resolved: ResolvedTarget;
  matcher: TagMatcher;
  itemCache: Map<string, Promise<Record<string, string>>>;
  log?: (event: ProgressEvent) => void;
}

export interface SwitchOptions {
  root: string;
  env: string;
  client: string;
  targetName: string;
  dryRun: boolean;
  log?: (event: ProgressEvent) => void;
}

export interface TargetSwitchResult {
  target: TargetConfig;
  outputPath: string;
  written: boolean;
  keyCount: number;
  layers: EnvLayer[];
  overwrittenKeys: string[];
}

export async function switchEnvironment(
  config: EnvswitchConfig,
  op: OnePasswordEnvironmentClient,
  options: SwitchOptions,
): Promise<TargetSwitchResult[]> {
  options.log?.({ status: "info", message: `validating project root ${options.root}` });
  await validateProjectRoot(config, options.root);
  validateSelection(config, options.env, options.client);

  const selectedTargets = selectTargets(config, options.targetName);
  const resolvedTargets = resolveTargets(options.root, selectedTargets);
  options.log?.({ status: "info", message: "listing candidate 1Password items" });
  const candidates = await op.listItemsByTags(candidateListTags(config, options.env));
  options.log?.({ status: "done", message: `found ${candidates.length} candidate item(s)` });
  const matcher = buildTagMatcher(config, options.env, options.client);
  const itemCache = new Map<string, Promise<Record<string, string>>>();

  const results: TargetSwitchResult[] = [];
  for (const resolved of resolvedTargets) {
    options.log?.({
      status: "info",
      message: `processing ${resolved.target.name} -> ${resolved.outputPath}`,
    });
    const mergeParams: MergeTargetParams = {
      resolved,
      matcher,
      itemCache,
    };
    if (options.log) {
      mergeParams.log = options.log;
    }
    const merge = await mergeTarget(op, candidates, mergeParams);

    if (!options.dryRun) {
      options.log?.({ status: "info", message: `writing ${resolved.outputPath}` });
      await writeEnvFile(resolved.outputPath, merge.values);
      options.log?.({ status: "done", message: `wrote ${resolved.outputPath}` });
    } else {
      options.log?.({ status: "dry-run", message: `would write ${resolved.outputPath}` });
    }

    results.push({
      target: resolved.target,
      outputPath: resolved.outputPath,
      written: !options.dryRun,
      keyCount: Object.keys(merge.values).length,
      layers: merge.layers,
      overwrittenKeys: merge.overwrittenKeys,
    });
  }

  return results;
}

export async function inspectProject(
  config: EnvswitchConfig,
  root: string,
): Promise<Array<{ target: TargetConfig; outputPath: string; parentExists: boolean }>> {
  await validateProjectRoot(config, root);
  return resolveTargets(root, config.targets).map((resolved) => ({
    target: resolved.target,
    outputPath: resolved.outputPath,
    parentExists: existsSync(dirname(resolved.outputPath)),
  }));
}

export function classifyItemForTarget(
  config: EnvswitchConfig,
  item: OnePasswordItemSummary,
  params: { env: string; client: string; target: string },
): LayerName | undefined {
  return classifyItemWithMatcher(item, params.target, buildTagMatcher(config, params.env, params.client));
}

function buildTagMatcher(config: EnvswitchConfig, env: string, client: string): TagMatcher {
  const common = tagValues(config, { env });
  const layerTags = new Map<LayerName, string>();
  for (const layer of LAYER_ORDER) {
    const layerTag = tagValues(config, { layer }).layer;
    if (layerTag) {
      layerTags.set(layer, layerTag);
    }
  }

  const currentGroupTagsByTarget = new Map<string, Set<string>>();
  const currentTargetTagsByTarget = new Map<string, Set<string>>();
  for (const target of config.targets) {
    currentTargetTagsByTarget.set(
      target.name,
      new Set(compactUnique([tagValues(config, { target: target.name }).target])),
    );
    currentGroupTagsByTarget.set(
      target.name,
      new Set(compactUnique((target.groups ?? []).map((group) => tagValues(config, { group }).group))),
    );
  }

  return {
    commonTags: compactUnique([...common.base, common.project, common.env]),
    allTargetTags: new Set(compactUnique(config.targets.map((target) =>
      tagValues(config, { target: target.name }).target,
    ))),
    allClientTags: new Set(compactUnique((config.clients ?? [client]).map((knownClient) =>
      tagValues(config, { client: knownClient }).client,
    ))),
    allGroupTags: new Set(compactUnique(config.targets.flatMap((target) =>
      (target.groups ?? []).map((group) => tagValues(config, { group }).group),
    ))),
    currentClientTag: tagValues(config, { client }).client,
    currentTargetTagsByTarget,
    currentGroupTagsByTarget,
    layerTags,
  };
}

function classifyItemWithMatcher(
  item: OnePasswordItemSummary,
  target: string,
  matcher: TagMatcher,
): LayerName | undefined {
  const tags = new Set(item.tags ?? []);
  for (const required of matcher.commonTags) {
    if (!tags.has(required)) {
      return undefined;
    }
  }

  const hasAnyTarget = hasAny(tags, matcher.allTargetTags);
  const hasAnyClient = hasAny(tags, matcher.allClientTags);
  const hasAnyGroup = hasAny(tags, matcher.allGroupTags);
  const hasCurrentTarget = hasAny(tags, matcher.currentTargetTagsByTarget.get(target) ?? new Set());
  const hasCurrentGroup = hasAny(tags, matcher.currentGroupTagsByTarget.get(target) ?? new Set());
  const hasCurrentClient = matcher.currentClientTag ? tags.has(matcher.currentClientTag) : false;

  if (hasLayerTag(matcher, tags, "shared") && !hasAnyTarget && !hasAnyClient && !hasAnyGroup) {
    return "shared";
  }
  if (hasLayerTag(matcher, tags, "group") && hasCurrentGroup && !hasAnyTarget && !hasAnyClient) {
    return "group";
  }
  if (hasLayerTag(matcher, tags, "target") && hasCurrentTarget && !hasAnyClient && !hasAnyGroup) {
    return "target";
  }
  if (hasLayerTag(matcher, tags, "client") && hasCurrentClient && !hasAnyTarget) {
    return "client";
  }
  if (hasLayerTag(matcher, tags, "client-target") && hasCurrentClient && hasCurrentGroup && !hasAnyTarget) {
    return "client-target";
  }

  return undefined;
}

async function mergeTarget(
  op: OnePasswordEnvironmentClient,
  candidates: OnePasswordItemSummary[],
  params: MergeTargetParams,
): Promise<MergeResult> {
  const layers: EnvLayer[] = [];
  for (const item of candidates) {
    const layerName = classifyItemWithMatcher(item, params.resolved.target.name, params.matcher);
    if (!layerName) {
      continue;
    }

    const itemKey = cacheKey(item);
    if (!params.itemCache.has(itemKey)) {
      params.log?.({ status: "info", message: `reading ${item.title || item.id}` });
      params.itemCache.set(itemKey, op.readEnvItem(item));
    }
    layers.push({
      name: layerName,
      rank: LAYER_ORDER.indexOf(layerName) + 1,
      item,
      values: await params.itemCache.get(itemKey)!,
    });
  }

  layers.sort((left, right) => {
    if (left.rank !== right.rank) {
      return left.rank - right.rank;
    }
    return left.item.title.localeCompare(right.item.title);
  });

  const values: Record<string, string> = {};
  const overwritten = new Set<string>();

  for (const layer of layers) {
    for (const [key, value] of Object.entries(layer.values)) {
      if (Object.prototype.hasOwnProperty.call(values, key) && values[key] !== value) {
        overwritten.add(key);
      }
      values[key] = value;
    }
  }

  return {
    values,
    overwrittenKeys: Array.from(overwritten).sort(),
    layers,
  };
}

function hasLayerTag(matcher: TagMatcher, tags: Set<string>, layer: LayerName): boolean {
  const layerTag = matcher.layerTags.get(layer);
  return Boolean(layerTag && tags.has(layerTag));
}

function hasAny(tags: Set<string>, candidates: Set<string>): boolean {
  for (const candidate of candidates) {
    if (tags.has(candidate)) {
      return true;
    }
  }
  return false;
}

function cacheKey(item: OnePasswordItemSummary): string {
  return item.id || item.title;
}
