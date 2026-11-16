//! Shared data types (PLAN §2.3). Frozen by S1; changes need orchestrator
//! approval and a broadcast to every active worker.
//!
//! # Summary
//!
//! | Rust type             | TS origin (`src/types.ts`, `src/switcher.ts`)      |
//! |-----------------------|----------------------------------------------------|
//! | [`Config`]            | `EnvswitchConfig`                                  |
//! | [`ProjectConfig`]     | `ProjectConfig`                                    |
//! | [`RootMarker`]        | `RootMarker`                                       |
//! | [`TargetConfig`]      | `TargetConfig`                                     |
//! | [`TagTemplates`]      | `TagTemplates` (as stored, before defaults)        |
//! | [`TagParams`]         | `tagValues` `params` argument                      |
//! | [`TagValues`]         | `tagValues` return value                           |
//! | [`ItemSummary`]       | `OnePasswordItemSummary`                           |
//! | [`Vault`]             | `OnePasswordVaultSummary`                          |
//! | [`EnvMap`]            | `Record<string, string>` of env values             |
//! | [`LayerName`]         | `LayerName`                                        |
//! | [`EnvLayer`]          | `EnvLayer`                                         |
//! | [`ResolvedTarget`]    | `ResolvedTarget`                                   |
//! | [`MergeResult`]       | `MergeResult`                                      |
//! | [`ProgressStatus`]    | `ProgressStatus`                                   |
//! | [`ProgressEvent`]     | `ProgressEvent`                                    |
//! | [`SwitchRequest`]     | `switchEnvironment(config, op, SwitchOptions)` minus `op`/`log` |
//! | [`TargetSwitchResult`]| `TargetSwitchResult`                               |
//! | [`SwitchResult`]      | `TargetSwitchResult[]`                             |
//! | [`ClassifyParams`]    | `classifyItemForTarget` `params`                   |
//! | [`InspectedTarget`]   | element of `inspectProject` result                 |
//! | [`InspectReport`]     | `inspectProject` result                            |
//!
//! # Conventions
//!
//! * **Absent vs present.** TS distinguishes `undefined` from an empty value
//!   in several places, so the Rust model keeps `Option` wherever the TS stores
//!   the key only when present: `project.id = Some("")` is *not* the same as
//!   `None` (`{project}` renders `""` instead of the cwd basename), and
//!   `groups: Some(vec![])` is preserved as parsed. JSON `null` is never
//!   "absent"; the parser (C1) rejects it with the TS message.
//! * **Order.** `IndexMap` is used wherever TS relies on object insertion
//!   order (env values, flags). `Vec` keeps array order (targets, items).
//! * **Paths.** Absolute output paths are `PathBuf` and are printed with
//!   `Path::display()`. Config strings (`output`, marker `path`) stay `String`
//!   exactly as written in the config, untrimmed.
//! * **`{project}` fallback.** TS falls back to `basename(process.cwd())`.
//!   Rust passes the cwd explicitly (`Deps::cwd`, [`SwitchRequest::cwd`],
//!   and the `cwd` argument of `config::tag_values` and friends) so the
//!   library never reads process state for it.

use std::fmt;
use std::path::PathBuf;

use indexmap::IndexMap;

/// Env values in insertion order (TS object semantics for non-integer keys;
/// env keys can never be integer-like). Re-inserting an existing key keeps its
/// original position, exactly like assigning to an existing JS property.
pub type EnvMap = IndexMap<String, String>;

// ---------------------------------------------------------------------------
// Config (contract 02 §1)
// ---------------------------------------------------------------------------

/// Parsed config item (`EnvswitchConfig`). Produced only by
/// `config::parse_config`, which validates every field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `project`; `None` when the key is absent.
    pub project: Option<ProjectConfig>,
    /// `clients`; `None` when absent (any client allowed). `Some(vec![])` also
    /// allows any client, but is kept distinct for fidelity.
    pub clients: Option<Vec<String>>,
    /// `environments`; same semantics as `clients`.
    pub environments: Option<Vec<String>>,
    /// `targets`; always non-empty, config order, names unique.
    pub targets: Vec<TargetConfig>,
    /// `tags` as written (no defaults merged in); `None` when absent.
    pub tags: Option<TagTemplates>,
}

/// `config.project`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    /// `project.id`. `Some("")` is accepted and stored; it behaves as "no
    /// project tag" but renders `{project}` as `""` (contract 02 §3.2).
    pub id: Option<String>,
    /// `project.rootMarkers`; `None` when absent, `Some(vec![])` allowed.
    pub root_markers: Option<Vec<RootMarker>>,
}

/// `config.project.rootMarkers[i]`. At least one field is `Some` (and
/// non-empty after JS trim) after parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootMarker {
    /// `path`, untrimmed.
    pub path: Option<String>,
    /// `packageJsonName`, untrimmed.
    pub package_json_name: Option<String>,
}

/// `config.targets[i]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetConfig {
    /// Identifier-validated name.
    pub name: String,
    /// Output path exactly as written (untrimmed, unresolved).
    pub output: String,
    /// `groups`; `None` when absent. Duplicates allowed.
    pub groups: Option<Vec<String>>,
}

/// `config.tags` as written. Missing keys fall back to the defaults in
/// `config::tags` (`DEFAULT_*_TEMPLATE`, base `[]`) at render time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagTemplates {
    pub base: Option<Vec<String>>,
    pub project: Option<String>,
    pub env: Option<String>,
    pub client: Option<String>,
    pub target: Option<String>,
    pub group: Option<String>,
    pub layer: Option<String>,
}

/// `tagValues(config, params)` input. A param that is `None` **or** `Some("")`
/// is "not passed" (TS truthiness): its rendered tag is `None` and its
/// placeholder renders `""`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TagParams<'a> {
    pub env: Option<&'a str>,
    pub client: Option<&'a str>,
    pub target: Option<&'a str>,
    pub group: Option<&'a str>,
    pub layer: Option<LayerName>,
}

/// `tagValues(...)` output (contract 02 §3.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagValues {
    /// Rendered base tags, empty strings dropped, order and duplicates kept.
    pub base: Vec<String>,
    /// `Some` only if `config.project.id` is non-empty.
    pub project: Option<String>,
    pub env: Option<String>,
    pub client: Option<String>,
    pub target: Option<String>,
    pub group: Option<String>,
    pub layer: Option<String>,
}

// ---------------------------------------------------------------------------
// 1Password (contract 02 §5)
// ---------------------------------------------------------------------------

/// `op item list` element. `id` and `title` are `""` when missing (never both).
/// `tags` keeps only string elements, in order; `[]` when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ItemSummary {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
}

impl ItemSummary {
    /// `item.id || item.title`: the `op item get` argument and read-cache key.
    pub fn key(&self) -> &str {
        if self.id.is_empty() {
            &self.title
        } else {
            &self.id
        }
    }

    /// `item.title || item.id`: used in `reading ...` progress and labels.
    pub fn display_name(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }
}

/// `op vault list` element. After parsing, `name` is `name || id`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Vault {
    pub id: String,
    pub name: String,
}

// ---------------------------------------------------------------------------
// Layers and merge (contract 02 §4)
// ---------------------------------------------------------------------------

/// `LayerName`, in rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LayerName {
    Shared,
    Group,
    Target,
    Client,
    ClientTarget,
}

impl LayerName {
    /// `LAYER_ORDER` / CLI `LAYERS`: shared, group, target, client, client-target.
    pub const ALL: [LayerName; 5] = [
        LayerName::Shared,
        LayerName::Group,
        LayerName::Target,
        LayerName::Client,
        LayerName::ClientTarget,
    ];

    /// The TS string (`"client-target"` etc.), used for `{layer}` and output.
    pub fn as_str(self) -> &'static str {
        match self {
            LayerName::Shared => "shared",
            LayerName::Group => "group",
            LayerName::Target => "target",
            LayerName::Client => "client",
            LayerName::ClientTarget => "client-target",
        }
    }

    /// `LAYER_ORDER.indexOf(name) + 1`: shared = 1 ... client-target = 5.
    pub fn rank(self) -> u32 {
        match self {
            LayerName::Shared => 1,
            LayerName::Group => 2,
            LayerName::Target => 3,
            LayerName::Client => 4,
            LayerName::ClientTarget => 5,
        }
    }
}

impl fmt::Display for LayerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One classified item applied to a target (`EnvLayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvLayer {
    pub name: LayerName,
    /// Always `name.rank()`; kept as a field to mirror TS.
    pub rank: u32,
    pub item: ItemSummary,
    pub values: EnvMap,
}

/// A selected target with its first-resolution absolute output path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub target: TargetConfig,
    pub output_path: PathBuf,
}

/// `mergeTarget` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// Merged values in JS assignment order.
    pub values: EnvMap,
    /// Keys overwritten with a *different* value, sorted by UTF-16 code unit
    /// (equal to byte order for env keys).
    pub overwritten_keys: Vec<String>,
    /// Layers sorted by rank, then en-US collation of `item.title` (D8),
    /// stable (ties keep `op` API order).
    pub layers: Vec<EnvLayer>,
}

// ---------------------------------------------------------------------------
// Progress (contract 01 §4.2, 02 §7)
// ---------------------------------------------------------------------------

/// `ProgressStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressStatus {
    Info,
    Done,
    DryRun,
}

impl ProgressStatus {
    /// `"info"`, `"done"`, `"dry-run"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ProgressStatus::Info => "info",
            ProgressStatus::Done => "done",
            ProgressStatus::DryRun => "dry-run",
        }
    }
}

impl fmt::Display for ProgressStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `ProgressEvent { status, message }`. Libraries emit these through a
/// `&mut dyn FnMut(ProgressEvent)`; only the CLI formats or prints them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgressEvent {
    pub status: ProgressStatus,
    pub message: String,
}

impl ProgressEvent {
    pub fn new(status: ProgressStatus, message: impl Into<String>) -> Self {
        ProgressEvent {
            status,
            message: message.into(),
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(ProgressStatus::Info, message)
    }

    pub fn done(message: impl Into<String>) -> Self {
        Self::new(ProgressStatus::Done, message)
    }

    pub fn dry_run(message: impl Into<String>) -> Self {
        Self::new(ProgressStatus::DryRun, message)
    }
}

// ---------------------------------------------------------------------------
// Switcher requests and results (contract 02 §4.4–4.6)
// ---------------------------------------------------------------------------

/// Input of `switcher::switch_environment` (TS `config` + `SwitchOptions`
/// without `log`; the logger is a separate argument, the client too).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRequest {
    pub config: Config,
    /// Project root, as given (printed verbatim in `validating project root`).
    pub root: PathBuf,
    /// Process cwd stand-in for the `{project}` fallback (`Deps::cwd`).
    pub cwd: PathBuf,
    pub env: String,
    pub client: String,
    /// A target name or the literal `"all"`.
    pub target_name: String,
    pub dry_run: bool,
}

/// One entry of the `switchEnvironment` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSwitchResult {
    pub target: TargetConfig,
    /// First resolution (before the pre-write re-check).
    pub output_path: PathBuf,
    /// `!dry_run`.
    pub written: bool,
    /// Number of merged keys.
    pub key_count: usize,
    /// Sorted layers (see [`MergeResult::layers`]).
    pub layers: Vec<EnvLayer>,
    pub overwritten_keys: Vec<String>,
}

/// `switchEnvironment` result: one entry per processed target, in config
/// order (or the single selected target).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SwitchResult {
    pub targets: Vec<TargetSwitchResult>,
}

/// `classifyItemForTarget` params.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifyParams {
    pub env: String,
    pub client: String,
    pub target: String,
}

/// One entry of the `inspectProject` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedTarget {
    pub target: TargetConfig,
    pub output_path: PathBuf,
    /// `existsSync(dirname(outputPath))` (follows symlinks; errors → false).
    pub parent_exists: bool,
}

/// `inspectProject` result, all config targets in config order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InspectReport {
    pub targets: Vec<InspectedTarget>,
}
