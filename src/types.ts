export interface RootMarker {
  path?: string;
  packageJsonName?: string;
}

export interface TargetConfig {
  name: string;
  output: string;
  groups?: string[];
}

export interface TagTemplates {
  base?: string[];
  project?: string;
  env?: string;
  client?: string;
  target?: string;
  group?: string;
  layer?: string;
}

export interface ProjectConfig {
  id?: string;
  rootMarkers?: RootMarker[];
}

export interface EnvswitchConfig {
  project?: ProjectConfig;
  clients?: string[];
  environments?: string[];
  targets: TargetConfig[];
  tags?: TagTemplates;
}

export interface OnePasswordItemSummary {
  id: string;
  title: string;
  tags?: string[];
}

export interface OnePasswordVaultSummary {
  id: string;
  name: string;
}

export interface EnvLayer {
  name: string;
  rank: number;
  item: OnePasswordItemSummary;
  values: Record<string, string>;
}

export interface ResolvedTarget {
  target: TargetConfig;
  outputPath: string;
}

export interface MergeResult {
  values: Record<string, string>;
  overwrittenKeys: string[];
  layers: EnvLayer[];
}

export type LayerName = "shared" | "group" | "target" | "client" | "client-target";

export type ProgressStatus = "info" | "done" | "dry-run";

export interface ProgressEvent {
  status: ProgressStatus;
  message: string;
}
