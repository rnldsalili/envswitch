# envswitch Rust Port — Orchestrated Plan

Status: **IMPLEMENTED locally (2026-11-17).** Approved 2026-11-16; all decisions are delegated to the orchestrator (§10.1).

Done:
- **`rust-port`:** the native port (C1–C4, D3), parity harness (F1), release pipeline (D1), dual-mode installers (D2b), and README.
  - All reviewed cross-model, including whole-port reviews R1 (GPT Sol) and R2 (Opus).
  - 474/474 oracle parity on macOS and Linux; 239 Rust tests; installer suites (bash/zsh, GNU, Alpine; pwsh in Docker); release tests.
- **Bridge branches:** `bridge/m1a-checksummed-source` and `bridge/m1b-dual-mode`. M2 is dropped (D2 = Light).

Remaining (needs owner approval, outward actions): Windows CI and Windows goldens, then the release sequence in [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md).

Changes since v1 address every finding in [research/04-plan-review.md](research/04-plan-review.md); §12 maps each finding to its fix.

## Inputs

These are the source of truth for behavior:

| Doc | Author | Scope |
|---|---|---|
| [research/01-cli-contract.md](research/01-cli-contract.md) | Opus 5.5 | Argument parsing, output and streams, exit codes, guided mode, `update`, 23 CLI tests |
| [research/02-core-contract.md](research/02-core-contract.md) | Opus 5.5 | Config, path safety, tags, classify and merge, `op` calls, dotenv, 25 core tests, 26 flagged quirks |
| [research/03-distribution.md](research/03-distribution.md) | GPT Sol 6.1 | Installers and release today, Rust build matrix, checksums, migration, self-update |
| [research/04-plan-review.md](research/04-plan-review.md) | GPT Sol 6.1 | Adversarial review of plan v1 |

Errata applied to the contracts after the review:

- **01 §1:** an empty next token is not consumed as a flag value.
- **02 §3.5:** the scope wildcard must exclude all four JS line terminators (`\r`, `\n`, U+2028, U+2029), not only `\n`.

When a contract and the TS source disagree, **the TS source at the oracle commit wins.** Workers report the disagreement instead of resolving it on their own.

---

## 1. Goal, non-goals, parity policy

**Goal.** Replace the Bun/TypeScript `envswitch` with one native Rust binary per platform. The new binary must:

- produce the same user-visible behavior: commands, flags, text, streams, exit codes, `.env` bytes, and `op` argv and call counts;
- install and update without Bun;
- migrate existing Bun installs without stranding anyone.

**Non-goals.** New features. Package managers. Developer ID notarization or Authenticode signing. Changing the 1Password item or tag model. The quirk fixes in §9 come after cutover.

**Oracle.** Commit `53de029` with **Bun 1.4.0**: ICU 78.1, Unicode 17.0, default locale `en-US`. F1 records this as metadata. CI can always rebuild the oracle from that SHA in an isolated directory, including after TS is removed from `main`. Oracle behavior never changes to match Rust.

**Parity policy.** Byte parity with the oracle for every input a real config, real items, and a real terminal can produce. The **only** exceptions are rows in [`DIVERGENCES.md`](DIVERGENCES.md) that the user approved in Phase 0. Each row lists:

- an example input;
- the TS outcome;
- the Rust outcome;
- the owning task;
- the replacement assertion.

Anything not in that table that differs is a bug. The proposed rows are in §10, D1.

---

## 2. Target architecture

### 2.1 Repository layout

The TypeScript stays in `src/` until cutover. Rust lives in a Cargo workspace:

```
Cargo.toml                    # [workspace]; members below. Single writer after S1: orchestrator
Cargo.lock                    # committed. Single writer: orchestrator
rust-toolchain.toml           # 1.100.0
crates/envswitch/
  Cargo.toml                  # all approved deps pre-declared by S1 (§2.2); version 0.0.0, CI-stamped
  src/
    main.rs  lib.rs           # S1; module index frozen (workers never edit)
    model.rs                  # S1; all shared data types (config, items, layers, merge result, ProgressEvent)
    ports.rs                  # S1; traits + Deps (§2.3)
    error.rs                  # S1
    jsutil.rs                 # C1
    config/{mod,paths,markers,tags}.rs     # C1 (bodies); signatures frozen by S1
    op/{runner,client,parse}.rs            # C2
    dotenv.rs                              # C2
    switcher/{mod,matcher,merge}.rs        # C3
    cli/{mod,args,help,output,guided,commands}.rs   # C4
    cli/update.rs, install/{embed,receipt}.rs       # D3
  tests/support/              # S1: thread-safe FakeOp, ScriptedPrompter, captured writers, temp project builder
  tests/<area>_*.rs           # each worker owns files with its own area prefix
crates/fake-op/               # F1: test-only native `op` binary (op.exe on Windows), fixture-driven, argv trace
parity/
  oracle/                     # F1: script that materializes the TS oracle at 53de029 into a temp dir with pinned Bun
  adapters/ts/                # F1: in-process TS adapter that drives exported main/switchEnvironment/inspectProject/
                              #     classifyItemForTarget/parseArgs/formatProgressEvent with fake op, prompt, and update deps
  adapters/rust/              # F1: same scenario protocol, driving envswitch::run with fakes
  scenarios/*.json            # F1: shared scenario definitions (inputs only)
  goldens/<os>/*.json         # generated by the TS adapter; path placeholders like {ROOT}; bytes kept exact
  diff/                       # F1: runner that compares transcripts (see §2.4)
```

### 2.2 Dependencies (pre-declared by S1)

Workers never edit `Cargo.toml` or `Cargo.lock`. A worker who needs anything else asks the orchestrator.

| Purpose | Crate |
|---|---|
| JSON | `serde_json` (manual `Value` parsing for config; string escaping matches `JSON.stringify`) |
| Errors | `thiserror` |
| Flags | `indexmap` (hand-written parser) |
| Prompts | `inquire` behind the `Prompter` trait |
| Collation | `icu_collator` + `icu_locale`, compiled data; locale per D8 |
| Scope matching | `regex` |
| Temp names | `uuid` v4 |
| Windows | `enable-ansi-support`, `ctrlc` |
| Dev | `tempfile`, `assert_cmd`, `predicates`, `insta`, `expectrl`, `junction` (Windows) |

No async runtime, no HTTP client, no updater crate. Self-update runs the embedded installer (03 §B7).

### 2.3 Shared definitions frozen by S1

S1 commits **compiling stubs** for every cross-module function, with `todo!()` bodies. Signatures, types, and thread rules are frozen once the orchestrator approves them. Changing one needs orchestrator approval and a broadcast to every active worker.

- **Model types (`model.rs`):** `Config`, `ProjectConfig`, `RootMarker`, `TargetConfig`, `TagTemplates`, `ItemSummary`, `Vault`, `EnvLayer`, `LayerName`, `ResolvedTarget`, `MergeResult`, `ProgressEvent { status, message }`, `SwitchRequest`, `SwitchResult`, `InspectReport`.
- **Function signatures:**
  - config: `parse_config`, `validate_selection`, `select_targets`, `resolve_targets`, `resolve_output_path`, `validate_project_root`, `tag_values`, `candidate_list_tags`, `compact_unique`
  - switcher: `switch_environment(&dyn OnePassword, &SwitchRequest, &mut dyn FnMut(ProgressEvent)) -> Result<SwitchResult>`, `inspect_project`, `classify_item_for_target`
  - dotenv: `stringify_env`, `write_env_file`
- **Ports (`ports.rs`):**
  - `CommandRunner: Send + Sync`
  - `OnePassword: Send + Sync`, handed out as `Arc<dyn OnePassword>` by an `OpFactory: Send + Sync`. The guided vault scan runs on `std::thread::scope` threads.
  - Progress from worker threads goes through an `mpsc` channel and is emitted on the main thread in vault order. Loggers never cross threads.
  - `Prompter` is main-thread only. `select(&SelectSpec { message, choices: Vec<Choice{label}>, default_index }) -> Result<usize>`, and `input(&InputSpec { message, default }) -> Result<String>`. The boolean Mode prompt is a two-choice select mapped by index.
  - `InstallerRunner` takes the `InstallPlan` that D2's protocol defines.
- **`Deps`:** `cwd` (used for `{project}`) is separate from `root` (the project root override). It also holds:
  - `env: Box<dyn Fn(&str) -> Option<OsString>>`
  - `stdin_is_tty`, `stdout_is_tty`, `stderr_is_tty`
  - `out` and `err` as `Box<dyn Write>`
  - `op_factory`, `vault_lister`, `prompter`
  - `current_exe` and `app_dir`, `installer`

  There is no clock abstraction.
- **Test support:** thread-safe fakes (`Mutex`, not `RefCell`) that record argv and read counts.

### 2.4 Parity harness (F1)

- **Two adapters, one scenario protocol.**
  - The **TS adapter** imports the oracle's exported functions (`main`, `parseArgs`, `formatProgressEvent`, `switchEnvironment`, `inspectProject`, `classifyItemForTarget`). It reaches private helpers *through* those public entry points, so the oracle's source never changes.
  - The **Rust adapter** drives `envswitch::run` and the same library functions with equivalent fakes.
- **Each scenario produces a transcript** with:
  - stdout and stderr bytes, and the exit code;
  - an ordered prompt log (message, choices, default);
  - progress events;
  - `op` argv list and per-item read counts;
  - written file bytes and Unix modes;
  - leftover temp files.
- **Black-box lane:** runs the real binary with `crates/fake-op` first on PATH, for real process spawning.
  - Before every scenario it runs a **handshake** (`op __envswitch_fake_handshake`). If it fails, the lane aborts, so it can never fall through to a real `op`.
  - On Windows, the fake is a real `op.exe`, because `Command::new("op")` doesn't resolve `.cmd`.
- **Paths:** goldens use placeholders such as `{ROOT}` and `{TMP}`. They are generated per OS, and backslashes and trailing newlines are kept exactly.
- **Guided-mode and `update` rows** run through the in-process adapters. `bun src/cli.ts` can't exercise them: it isn't a TTY, and it always detects `.git`.

---

## 3. Orchestration model

**Orchestrator (main agent, Opus 5.5).** Owns:

- the plan, the decisions, and the frozen definitions;
- `Cargo.toml` and `Cargo.lock` after S1, plus `.gitignore` and `DIVERGENCES.md`;
- the `rust-port` integration branch and merge gates;
- cross-agent consistency and final acceptance.

It writes no feature code, only integration glue and small review fixes. Each of those edits gets a focused cross-model re-review (§3.3).

Every outward action needs explicit user approval: push, PR, workflow dispatch, release, or tag.

### 3.1 Workers

- **Isolation:** each worker gets its own worktree, branched from `rust-port` or from `main` for bridge work.
- **Files:** an exclusive ownership list. Everything else is read-only.
- **Brief:** a self-contained brief (§7).
- **Finish:** a branch, green checks for its *coding gate*, and a report covering what was built, deviations, open questions, and test evidence.
- **Models:** only **Claude Opus 5.5** (native `Agent`) and **GPT Sol 6.1** (`gpt-6.1-sol` via `delegate_task`).

### 3.2 Effort policy

| Effort | Used for |
|---|---|
| `xhigh` | Path safety, merge and collation, release pipeline, installers, migration, whole-port reviews |
| `high` | Module implementation, the parity harness, per-PR reviews |
| `medium` | Docs |

### 3.3 Review rules

- Every PR is reviewed by the **other** model at `high` before merge.
- Reviewers get the diff, the contract sections, the approved divergences, and test evidence. They don't get the author's summary.
- Any later change to reviewed code needs a focused cross-model re-review of that change. That includes orchestrator glue, rebases with conflict resolution, the workflow switch, and TS removal.
- **Whole-port reviews** R1 (GPT Sol) and R2 (Opus) both run at `xhigh` on the **same frozen candidate SHA**. Their briefs are independent: requirements, source, and test evidence. Neither sees the other's findings. Any change after that SHA voids their sign-off for the touched area and triggers a focused re-review.

### 3.4 Two gates per task

- **Coding gate (may start):** the task's dependencies are frozen as definitions.
- **Merge gate (may merge into `rust-port`):** the task's tests pass against the **real** merged dependencies, not stubs.

A worker can be "code complete" at the coding gate. It is "done" only at the merge gate.

**Definition of done (merge gate):**

1. `cargo fmt --check`, `clippy -D warnings`, and `cargo test` are green on Linux, macOS, and Windows CI.
2. Every parity scenario in scope passes on all three OSes, or matches an approved divergence row.
3. Each contract test row assigned to the task is ported (§6).
4. The cross-model review is clean.
5. Nothing outside the ownership list changed.

---

## 4. Work breakdown

### Phase 0 — Decisions and facts

| ID | Task | Who |
|---|---|---|
| P0.1 | Resolve D1–D9 (§10). Approve the `DIVERGENCES.md` rows. | User |
| P0.2 | Probe what `op item list` prints for an empty result (`[]` or nothing) with one **read-only** command and a nonexistent tag. AND vs OR is not needed: classification re-checks tags locally. | Orchestrator, after user approval |
| P0.3 | Confirm repo visibility and plan (attestations, D6). Confirm the number of active installs and any custom install dirs (D2). | User |

### Phase 1 — Foundations

| ID | Task | Model / effort | Owns | Coding gate → Merge gate |
|---|---|---|---|---|
| **S1** | Scaffold and shared definitions (§2.3): workspace, toolchain, pre-declared deps, `model.rs`, `ports.rs`, `error.rs` (1/2/130 mapping), compiling stubs for every cross-module function, `tests/support/`, `rust-check` CI on 3 OSes (never publishes). | Opus 5.5 / high | §2.1 S1 files, `.github/workflows/rust-check.yml` | P0 → orchestrator freezes the definitions |
| **F1** | Parity harness (§2.4): oracle materializer, TS and Rust adapters, scenario protocol, `crates/fake-op` with handshake, goldens per OS, diff runner, CI job. Must include the errata cases (empty next token; CR, LF, U+2028, U+2029 scopes) and the expanded collation set (D8). | GPT Sol 6.1 / high | `parity/**`, `crates/fake-op/**`, `.github/workflows/parity.yml` | P0 + S1 frozen → TS adapter and goldens merged first; Rust adapter merged with S1 stubs (scenarios expected to fail) |
| **D2a** | **Installer protocol spec**, docs only: the installer CLI contract shared by fresh install, native update, and the bridge. Covers flags and env, `ENVSWITCH_VERSION` / `-Version`, receipt schema, `.version` semantics, exit codes, output lines, release-kind detection (source vs native), checksum policy, and the native binary's `--version` output per D9. | GPT Sol 6.1 / xhigh | `docs/rust-port/INSTALLER-PROTOCOL.md` | P0 → orchestrator freezes it |

### Phase 2 — Core, CLI, distribution

| ID | Task | Model / effort | Owns | Coding gate | Merge gate |
|---|---|---|---|---|---|
| **C1** | `jsutil` + `config/*` bodies. JS-compatible trim. Manual parse with verbatim messages. Lexical path normalizer, symlink walk, Windows rules (keeps drive-relative parity unless D1 says otherwise). Markers and tags. Filesystem error messages per D1 row "fs errors". | Opus 5.5 / xhigh | `jsutil.rs`, `config/**`, `tests/config_*` | S1 | S1 + F1 merged |
| **C2** | `op/*` + `dotenv.rs`. Runner (no shell, 50 MiB cap, stderr-only errors, sequential), lenient parse, field → env mapping, `stringify_env` (22 golden cases), atomic write. | GPT Sol 6.1 / high | `op/**`, `dotenv.rs`, `tests/op_*`, `tests/dotenv_*` | S1 | S1 + F1 + C1 (`jsutil`) merged |
| **C3** | `switcher/*`. Matcher with the JS line-terminator wildcard, every `classify` branch, rank plus collation per D8 with stable API-order ties, overrides and `overwrittenKeys`, read cache, symlink re-check before write, progress-event order. | Opus 5.5 / xhigh | `switcher/**`, `tests/switcher_*` | S1 | C1 + C2 merged |
| **C4** | `cli/*`. Parser with all quirks including the empty token; help; streams; SGR rules (using `stderr_is_tty`); `formatProgressEvent`; summaries. Commands `switch`, `list`, `doctor`, and `--version` (D9). Guided mode: threaded order-preserving scan, auto-select, defaults, cancel → 130, Windows VT and Ctrl-C. | GPT Sol 6.1 / high | `cli/**` except `update.rs`, `tests/cli_*` | S1 (parser, help, output, guided against fakes) | C3 merged (command wiring and parity) |
| **D1** | Release pipeline. `release.yml` stays the **single publisher and version authority** (`0.0.${run_number}` of *that* workflow, so the counter continues). Native builds run in reusable `build-native.yml` (`workflow_call`, receives version and SHA). 5-target matrix (musl Linux), Cargo stamping plus lockfile reconcile, `--locked`, packaging, `SHA256SUMS`, optional attestations (D6). Draft → verify the full asset set → promote. Guard against regressing latest. Tag/SHA conflict = failure. **Transition gate:** publishing is `workflow_dispatch` only with `publish: true` until the first native release is verified (D5). Installer and parity suites are required jobs before publishing. | GPT Sol 6.1 / xhigh | `.github/workflows/release.yml`, `.github/workflows/build-native.yml` | D2a + S1 | D2b merged |
| **D2b** | Native-capable installers implementing D2a. **Dual-mode:** installs whichever kind the resolved release is (source or native), so main's installers are correct in every release state (§5). Detection (Rosetta, Windows OS architecture), checksum fail-closed, safe extraction, versioned payloads, atomic launcher switch, receipt, lock and rollback, `-WhatIf` with no side effects, same PATH guidance. Test suites: Bash (bats-free, extends `test/install-sh.test.sh`) and PowerShell. Isolation covers `HOME`, `XDG_*`, `USERPROFILE`, `LOCALAPPDATA`, and profile files, and **mocks persistent Windows User PATH writes** through a test hook. | GPT Sol 6.1 / xhigh | `scripts/install.{sh,ps1}`, `test/install-*`, `.github/workflows/installers.yml` | D2a | D2a frozen; reviewed by Opus |

### Phase 3 — Update and migration (strictly after D2b merges)

| ID | Task | Model / effort | Owns | Merge gate |
|---|---|---|---|---|
| **D3** | Rust `update`. Install located from `current_exe` plus receipt. Source-checkout and `target/` guard. Embedded D2b installer written to private temp and run with persisted settings and the compiled version. Signal exit → 128+n. Equal `.version` with a missing payload = repair. Replacement assertions for 01 §9 rows 16–22 per approved divergences. Row 23 (unknown flag fails before any work) is shared with C4. | Opus 5.5 / high | `cli/update.rs`, `install/**`, `tests/update_*` | C4 + D2b merged |
| **M1** | **TS bridge**, branched from `main`, *not* `rust-port`. The `src/cli.ts` update path pins the resolved tag and passes repo, app root, and launcher dir. It bundles D2b's dual-mode installers, which become the installers on `main`. Includes TS tests for the changed rows. **Ownership hand-off:** D2b → M1 for `scripts/install.*` on the bridge branch, by copying the merged D2b files with no edits. Changes go back to D2b's owner. | Opus 5.5 / xhigh | `src/cli.ts` (update section only), `test/cli.test.ts` (update rows), bridge copy of `scripts/install.*` | D2b merged; reviewed by GPT Sol |
| **M2** | **Compatibility archive and forwarding stub** (only if D2 = full). Dependency-free `envswitch_<ver>_source.tar.gz` built by a packaging step D1 hands off: D1 owns `release.yml`, M2 owns `.github/workflows/package-compat.yml` that `release.yml` calls. The stub bootstraps native with a pinned tag and forwards argv, stdio, and exit code. Never deletes itself on failure. | GPT Sol 6.1 / xhigh | `compat/**`, `.github/workflows/package-compat.yml`, `test/migration-*` | D1 + D2b + D3 merged; reviewed by Opus |

**Migration tests are required whichever migration depth is chosen (D2).**

- With light migration, M1's old → bridge and bridge → native tests still run. The old-updater rows check for a clear error plus reinstall guidance instead of compatibility bootstrap.
- Native targets for these tests come from a **locally simulated complete native release**: assets built by `build-native.yml` and served from a temp directory or a mock GitHub API, so they don't wait for a live release.

### Phase 4 — Integration and verification (on `rust-port`, after M1 and M2 merge)

| ID | Task | Who |
|---|---|---|
| I1 | Merge in gate order: S1 → F1 → D2a → C1 → C2 → C3 → C4 → D2b → D1 → D3 → M2. M1 lives on its bridge branch. Wire `Deps::real()`. | Orchestrator |
| I2 | Full parity suite on 3 OSes plus the installer and migration suites. Every diff gets triaged as a bug (back to its owner), a contract error (fix the contract, regenerate goldens from the oracle), or a proposed new divergence (needs user approval). | Orchestrator |
| R1 ║ R2 | Independent whole-port reviews on one frozen candidate SHA (§3.3). | GPT Sol 6.1 / xhigh ║ Opus 5.5 / xhigh |
| V1 | Real-1Password smoke on the user's machine: `doctor`, `list`, `switch --dry-run`, a real write in a scratch project, guided mode, Ctrl-C. Compared with the oracle. | User, with orchestrator guidance |
| V2 | Packaged-binary smoke: macOS arm64 and x64, glibc Linux, Alpine, WSL, Windows from 32-bit and 64-bit PowerShell. | CI, plus the user for gaps |

### Phase 5 — Release sequence (each numbered step needs explicit user approval)

1. **Bridge PR (M1) → `main`.** Its own small PR with a cross-model review. The current push-to-main pipeline publishes a **source** bridge release. That's expected, since the release kind is unchanged. Test old → bridge against the live bridge release while it's latest.
2. **`rust-port` → `main`.** `release.yml` is now in transition mode: the push builds and verifies but **doesn't publish**. That was confirmed by a dry-run dispatch before the merge.
3. **First native release:** manual dispatch with `publish: true`. It publishes the native assets, `SHA256SUMS`, the installers, and, if D2 = full, the compatibility archive. It becomes latest only after the full asset set is verified.
4. **Live migration checks:** old (pre-bridge) → compatibility archive → native (full only), bridge → native, fresh install, and native → native update.
5. **Turn push-to-main publishing back on** (D5).
6. **Docs (GPT Sol 6.1 / medium):** README install, update, and manual install; `DIVERGENCES.md`; drop the Bun prerequisites.
7. **Remove the TS app from `main`.** The oracle stays reachable by SHA for F1, and `compat/` stays if D2 = full. This change gets a focused cross-model review.

---

## 5. Release-state table

The table shows what each kind of install does in each state. "Main installer" means the README one-liner, which fetches `main/scripts/install.*`.

| State | Latest release | Main installer | Pre-bridge install runs `update` | Bridge install runs `update` | Native install runs `update` |
|---|---|---|---|---|---|
| **S0** today | source (TS) | old: Bun + source | old installer → source | — | — |
| **S0→S1 window** (bridge merged, release in flight) | source (TS) | D2b dual-mode → installs source | old installer → source | — | — |
| **S1** bridge latest | source (TS bridge) | dual-mode → installs bridge source | old installer → bridge source, *no integrity check, same as today* | up to date | — |
| **S1→S2 window** (`rust-port` merged, not published) | source (TS bridge) | dual-mode → bridge source | → bridge | up to date | — |
| **S2** native latest | native + `SHA256SUMS` (+ compat archive if full) | dual-mode → **native**, verified | **full:** old installer → compat archive → stub bootstraps native on the next run. **light:** old installer fails with "asset not found", and the user reinstalls with the one-liner | dual-mode installer, pinned tag → **native** | embedded installer → native |

---

## 6. Contract test ownership

| Contract rows | Owner | Notes |
|---|---|---|
| 01 §9 1–6 | C4 | Parser and formatting, including empty-token errata |
| 01 §9 7–15 | C4 | Scripted prompts with defaults, factory reuse, concurrent scan (barrier), cancel, non-TTY guard; thread-safe fakes from S1 |
| 01 §9 16–22 | D3 | Replacement assertions per approved update divergences |
| 01 §9 23 | C4 + D3 | Unknown update flag fails before receipt, network, or installer |
| 02 §8.2 1–9 | C1 | Includes Windows `junction` cases |
| 02 §8.2 10–11 | C3 | Includes CR, LF, U+2028, U+2029 scope cases |
| 02 §8.2 12–16 | C3 (merge gate after C1 and C2) | Merge bytes and events, no printing from the library, read cache, symlink swap before write |
| 02 §8.2 17–22 | C2 | Windows black-box via `fake-op` `op.exe` |
| 02 §8.2 23–25 | C2 | 22 serialization cases, atomic replace and cleanup, Unix modes |
| 03 §B9 Matrix, Release | D1 | Jobs required before publishing |
| 03 §B9 Detection, Integrity, Install, PATH | D2b | `installers.yml`, required |
| 03 §B9 Windows update | D3 + D2b | |
| 03 §B9 Migration | M1 (bridge rows) + M2 (compat rows); light-mode error rows go to M1 | Required for both D2 choices |

---

## 7. Worker brief template

The orchestrator fills one of these for each task:

1. Goal, plus the exact contract § numbers and approved divergence rows.
2. Ownership list, the frozen definitions, the coding gate, and the merge gate.
3. Parity scenarios in scope and the commands to run them.
4. Definition of done (§3.4).
5. Constraints:
   - never call the real `op`, except in V1, by the user;
   - never install into the real `HOME` or `USERPROFILE`, and never write the real Windows User PATH;
   - no push, PR, tag, release, or dispatch;
   - American English in user-facing text;
   - synthetic data only (`demo-suite`, `alpha`, `beta`), no customer data.
6. Report format: summary, deviations, open questions, evidence.

---

## 8. Schedule

```
Wave 0  P0.1–P0.3 + DIVERGENCES approval              user + orchestrator
Wave 1  S1 ║ F1(TS side) ║ D2a                          3 agents
Wave 2  C1 ║ C2 ║ C3(code) ║ C4(code) ║ D2b ║ F1(Rust side)   6 agents; merges follow gates
Wave 3  C3 merge → C4 merge ║ D1 ║ D3 (after D2b) ║ M1 (after D2b)   
Wave 4  M2 → I1/I2 → R1 ║ R2 → V1, V2
Wave 5  Release sequence §4 Phase 5 (user-approved, step by step)
```

**Critical path:** P0 → S1 → C1 → C2 → C3 → C4 → D3 → M2 → I2 → R1/R2 → release steps 1–3.

D2a and D2b run early and in parallel because D3, M1, M2, and D1 all depend on them.

---

## 9. After cutover (separate PRs, deliberate behavior changes)

These each get their own PR:

- timeout for `op` calls;
- friendly "`op` missing" and "not signed in" messages;
- `--` before item names;
- fix the `..env` false escape;
- transactional multi-target writes and fsync;
- reject a target named `all`;
- validate `project.id: ""`;
- revisit the collation policy (if D8 kept host-default behavior).

---

## 10. Decisions needed from the user

| ID | Decision | Recommended default |
|---|---|---|
| D1 | **Divergence table.** Approve these rows, or reject any of them. | (a) JS-engine internals: `{constructor}`-style placeholders render `""`, the `__proto__` label is kept, JSON parse and spawn error *suffixes* are matched by prefix. (b) Windows Ctrl-C → 130 through a handler. (c) Prompts render on stderr (TS uses stdout), no list wraparound, substring filter instead of prefix jump, Esc per D3. (d) Filesystem errors: Node format `CODE: description, syscall 'path'` reproduced for ENOENT, ENOTDIR, EACCES, EPERM, EEXIST, EISDIR; other codes matched by prefix. (e) `update`, rows 16–22: compiled version replaces `.version` as the current version, embedded installer, receipt-based locations, and a copied binary without a receipt gets reinstall guidance. (f) `--version` added (D9). Windows drive-relative paths keep **parity**, with no tightening. |
| D2 | Migration depth for existing Bun installs | **Light** if there are only a few installs and you can reach those users: bridge release plus a reinstall note. That removes M2. **Full** otherwise: a compatibility archive on every release. |
| D3 | Esc in guided prompts | Cancel → exit 130 (`inquire` default) |
| D4 | Argument parser | Hand-written for exact parity |
| D5 | Versioning and publishing | Keep `0.0.<run_number>` from `release.yml` as the single authority. Manual-dispatch publishing during the transition, then back to push-to-main. |
| D6 | GitHub artifact attestations | On if the repo is public or on Enterprise Cloud; otherwise SHA-256 only |
| D7 | macOS notarization and Windows signing | Deferred; documented friction |
| D8 | **Same-layer title ordering (collation).** The TS oracle uses the host's default locale. | A **fixed `en-US` ICU collation**, documented. It equals the oracle on en-US machines and is deterministic everywhere. The divergence only shows up on non-English hosts whose same-layer item titles differ by locale-sensitive letters. The alternative is host-default locale detection, which is harder to match exactly. |
| D9 | `--version` flag (installers need it to verify the staged binary) | Add it as an approved divergence. Output: `envswitch <version> <target-triple>\n`, for example `envswitch 0.0.42 aarch64-apple-darwin`, exit 0. |

---

### 10.1 Resolved (orchestrator, 2026-11-16, delegated by the user)

| ID | Resolution |
|---|---|
| D1 | Default rows (a)–(f) approved. See [DIVERGENCES.md](DIVERGENCES.md). |
| D2 | **Light.** No compatibility archive. **M2 is dropped.** Pre-bridge installs that skip the bridge get a clear README and release-note reinstall path. M1 owns the light-mode migration tests. |
| D3 | Esc = cancel → 130 |
| D4 | Hand-written parser |
| D5 | `0.0.<run_number>` from `release.yml`. Manual-dispatch publishing during the transition. |
| D6 | **On.** The repo is public, so `actions/attest` runs in the producing build jobs. |
| D7 | Deferred |
| D8 | Fixed `en-US` ICU collation |
| D9 | Add `--version` → `envswitch <version> <target-triple>\n`, exit 0 |
| D2a gate | The historical-source checksum gate (INSTALLER-PROTOCOL §1.3) is solved by splitting M1 into **M1a**: `release.yml` publishes `SHA256SUMS` and the standalone `install.sh`/`install.ps1` with the source release, installers unchanged. That push yields a checksummed source latest. Then **M1b**: dual-mode installers and the TS bridge `update`. No fail window. INSTALLER-PROTOCOL.md is **frozen** (protocol v1). |
| P0.2 | Not blocking. Rust reproduces TS behavior for whatever `op` prints (invalid JSON → the same error). |

## 11. Risk register

| # | Risk | Mitigation | Owner |
|---|---|---|---|
| 1 | Same-layer winner changes because of collation | D8 fixed policy; expanded goldens that assert the winning **value** (accents, Swedish-style letters, composed vs decomposed, punctuation, non-Latin, equal titles) | C3, F1 |
| 2 | Parity drift (text, codes, `op` calls) | Transcript diff includes argv and read counts; divergence table is the only waiver | F1, I2 |
| 3 | Windows paths and spawning | Own normalizer; native `fake-op.exe` with handshake; Windows CI | C1, C2, F1 |
| 4 | A premature or partial release | Single publisher; transition-mode manual dispatch; draft → verify → promote; latest guard | D1 |
| 5 | Stranding Bun installs | Dual-mode installers on `main` from the bridge onward; release-state table; migration tests required in both D2 modes | D2b, M1, M2 |
| 6 | Windows exe locks, partial updates | Versioned payloads; launcher switch; rollback; deferred prune | D2b, D3 |
| 7 | Stubs make workers look green | Coding gate vs merge gate | Orchestrator |
| 8 | Ownership collisions | Pre-declared deps; frozen module index; explicit hand-offs (D2b → M1, D1 → M2) | Orchestrator |
| 9 | The oracle disappears at cutover | Pinned SHA plus Bun version; materializer script in CI | F1 |
| 10 | Review sign-off goes stale | Same frozen SHA for R1 and R2; re-review on any later change | Orchestrator |

---

## 12. Plan review (04) disposition

| Finding | Fix in v2 |
|---|---|
| 1 Blocker: cutover order impossible | §4 Phase 5 reordered; §5 release-state table; dual-mode installers; locally simulated native release for migration tests |
| 2 Blocker: two run counters | `release.yml` is the single publisher and version authority; `build-native.yml` is reusable; tag/SHA conflict fails |
| 3 Interfaces not actually frozen | §2.3: S1 owns models, signatures, thread rules, prompt schema, typed `Deps` (cwd vs root, env, TTYs) |
| 4 Workers can't be green independently | §3.4 coding vs merge gates; dependency-ordered merges; §8 recomputed critical path |
| 5 Windows `op.cmd` | Native `crates/fake-op` (`op.exe`) with mandatory handshake |
| 6 Harness lacks adapters, trace, baseline | §2.4 TS and Rust adapters, transcript with argv and read counts, pinned oracle materializer, per-OS goldens |
| 7 ICU root ≠ host default | D8 decision; expanded collation goldens |
| 8 Contract errors | Errata applied to 01 §1 and 02 §3.5; cases assigned to F1, C3, C4 |
| 9 `--version` conflict | D9; C4 owns it; D2a defines the verification output, including the target |
| 10 Divergences exceed policy | Concrete divergence table D1 (a)–(f) approved before S1 and F1 |
| 11 Migration and test ownership gaps | M1 and M2 ownership and hand-offs; single manifest writer; §6 mapping incl. 03 §B9; isolation incl. `USERPROFILE` and mocked User PATH; required CI jobs |
| 12 Minor: AND/OR probe | P0.2 narrowed to the empty-output format |
| 13 Minor: review head | §3.3 frozen SHA and re-review rule |
