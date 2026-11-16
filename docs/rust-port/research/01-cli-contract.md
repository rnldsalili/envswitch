# 01 — CLI Layer Behavioral Contract (TypeScript → Rust parity)

Scope: `src/cli.ts` (742 lines), `src/errors.ts`, `src/types.ts`, `test/cli.test.ts` (23 tests), README "Commands" (README.md:82-97).
Internals of `switcher.ts` / `onepassword.ts` / `config.ts` are owned by another research doc; only the interfaces `cli.ts` calls are listed here.
Verified against Bun 1.4.0, `@inquirer/prompts` 8.5.2 (`@inquirer/core` 11.2.1, `select` 5.2.1, `input` 5.1.2). All 23 CLI tests pass. Edge cases below marked **(probed)** were confirmed by calling `main()` with fake deps (no real 1Password).

---

## 0. Entry point and top-level dispatch

- Shebang `#!/usr/bin/env bun` (cli.ts:1). Entry: `if (import.meta.main) process.exit(await main())` (cli.ts:740-742).
- `main(argv = Bun.argv.slice(2), options: MainOptions = {}) -> Promise<number>` (cli.ts:143). Returns the exit code; never calls `process.exit` itself.
- `createCliDeps(options)` runs **outside** the try/catch (cli.ts:144, 459-476).

Dispatch order (cli.ts:146-177), first match wins:

| # | Condition | Action |
|---|---|---|
| 1 | `argv.length === 0` | guided switch (`runGuidedSwitch`) → 0 |
| 2 | `argv[0] === "--help"` or `argv[0] === "-h"` | print help → 0 (rest of argv ignored, never parsed) |
| 3 | `parseArgs(argv)`; then `flags.help === true` (exact boolean) **or** `command === "help"` | print help → 0. Runs **before** command validation and before flag allowlists, so `foo --help`, `switch --unknown --help` print help **(probed)** |
| 4 | `command` ∈ `switch`/`list`/`doctor`/`update` | run handler → 0 |
| 5 | anything else | `fail("unknown command: ${command}")` → exit 1 |

Consequences **(probed)**:
- No `--version` / `-v` / `-V`: `envswitch --version` → stderr `envswitch: unknown command: --version`, exit 1. `-v` likewise.
- `-h` is recognized only as `argv[0]`. `envswitch switch -h` → `-h` becomes a positional (ignored) → `envswitch: --vault is required`, exit 1.
- `envswitch ""` → `envswitch: unknown command: ` (trailing space), exit 1.
- `envswitch --vault x switch` → command is `--vault` → `envswitch: unknown command: --vault`.
- `envswitch foo --help=1` → `help` flag is the string `"1"`, not `true` → falls through → `unknown command: foo`.
- `envswitch switch --help=false` / `switch --help me` → help not triggered; switch's validator then fails with `--help does not take a value`.

## 1. Argument parser — `parseArgs` (cli.ts:188-223)

Output shape: `{ command: string | undefined, positional: string[], flags: Record<string, string | true> }`.

Algorithm (must be reproduced exactly; it is not POSIX/GNU and not clap-compatible):
1. `command = argv[0]` (whatever it is — even `--foo` or `""`).
2. For each remaining token:
   - Does **not** start with `--` → push to `positional` (includes `-h`, `-x`, `-`).
   - Starts with `--`: `raw = token[2..]`.
     - Contains `=` → `flags[raw[..=idx]] = raw[idx+1..]` (split on **first** `=`; `--a=b=c` → `a: "b=c"`; `--vault=` → `vault: ""`).
     - Else if next token is missing, **is the empty string `""`** (source uses `!next`, cli.ts:212-218), **or starts with `--`** → `flags[raw] = true`. The empty token is then *not* consumed and falls through as an empty positional: `["switch","--help",""]` → `help: true` + positional `[""]` → help, exit 0; `switch --dry-run ""` → reaches `--vault is required`. *(Erratum from plan review 04 §8.)*
     - Else → `flags[raw] = next`; consume next. A value starting with a single `-` is consumed (`--x -y` → `x: "-y"`; `--client -x` → client `-x`).
3. Bare `--` → flag named `""` (→ later `unknown flag: --`). No end-of-options semantics.
4. No short flags, no abbreviations, case-sensitive (`--VAULT` → `unknown flag: --VAULT`).
5. Repeated flag: last value wins; error-reporting order keeps first-insertion position (JS object semantics; use `IndexMap` in Rust).
6. JS quirks **not** worth replicating (document as intentional divergence): `--__proto__ x` is silently dropped (probed); integer-like flag names (`--1`) iterate before others in `Object.entries`.
7. Boolean flags greedily consume a following non-`--` token: `switch --dry-run foo` → `dry-run: "foo"` → `--dry-run does not take a value` **(probed)**.

Test asserting parser output directly: "parses omitted optional switch flags so defaults can apply" (cli.test.ts:48-58) — absent flags are **absent keys**, not defaults.

## 2. Flag validation helpers

| Helper | Lines | Behavior / exact error |
|---|---|---|
| `validateFlags(args, allowed)` | 696-705 | Iterate flags in insertion order. Not in allowlist → `unknown flag: --<name>`. In `BOOLEAN_FLAGS` (`dry-run`, `help`; line 99) and value `!== true` → `--<name> does not take a value`. First failing flag wins. Runs before any required-flag checks. |
| `stringFlag(args, name, default)` | 707-716 | Absent → default. Present but `true` or `value.trim() === ""` → `--<name> requires a value`. Otherwise returns the value **untrimmed**. |
| `requireStringFlag(args, name)` | 688-694 | `stringFlag(..., "")`; empty → `--<name> is required`. So: absent → `is required`; `--vault` with no value / `--vault=` / `--vault="  "` → `requires a value` **(probed)**. |
| `hasFlag(args, name)` | 718-720 | `flags[name] === true`. |

Allowlists (cli.ts:94-97): `SWITCH_FLAGS = {vault, config-item, client, env, target, dry-run, help}`, `LIST_FLAGS = DOCTOR_FLAGS = {vault, config-item, help}`, `UPDATE_FLAGS = {help}`.

Positionals are ignored by `switch`, `doctor`, `update` (e.g. `update extra` proceeds). `list` reads only `positional[0]`; extras ignored.

Validation order inside each handler = order of statements (first error wins):
- switch: validateFlags → `--vault` → `--config-item` → `--client` → `--env` → `--target`. E.g. `--env` with no value only surfaces after the three required flags pass **(probed)**.

## 3. Commands

### 3.1 `envswitch switch` (cli.ts:265-286)

| Flag | Required | Default | Notes |
|---|---|---|---|
| `--vault <v>` | yes | — | passed to `createOnePasswordClient(vault)` |
| `--config-item <item>` | yes | — | name or id; also used verbatim in log lines |
| `--client <c>` | yes | — | validated later by `switchEnvironment` (identifier regex, must be in `config.clients` if non-empty) |
| `--env <e>` | no | `local` (`DEFAULT_ENV`, line 90) | |
| `--target <name\|all>` | no | `all` (`DEFAULT_TARGET`, line 91) | `all` is a sentinel; a target literally named `all` is unreachable |
| `--dry-run` | no | false | boolean; any value → error |
| `--help` | no | — | handled at dispatch |

Flow: create client → `loadConfig(op, configItem, log)` → `switchEnvironment(config, op, {root, env, client, targetName, dryRun, log})` → `printSwitchResults`.

### 3.2 `envswitch list` (cli.ts:288-310)

Flags: `--vault`, `--config-item` (both required). Subject = `positional[0] ?? "targets"`.
**Config is loaded (1Password is called) before the subject is validated** — `list ... bogus` hits `readConfigItem` first, then fails `unknown list subject: bogus` **(probed)**.

| Subject | stdout |
|---|---|
| `clients` | one client per line; `(none)` if `config.clients` absent/empty (`printList`, 449-457) |
| `targets` (default) | `<name>\t<output>` per target, `output` = raw config string (not resolved). Never empty (config requires ≥1 target). |
| `layers` | exactly 5 lines in order `shared, group, target, client, client-target` (`LAYERS`, line 93): `<layer>\t<rendered layer tag or "">` via `tagValues(config, {layer})` (442-447). Default template → `shared\tlayer:shared`. |

### 3.3 `envswitch doctor` (cli.ts:312-332)

Flags: `--vault`, `--config-item` (required). Sequence:
1. log `info: checking 1Password authentication`
2. `op.assertAuthenticated()` (runs `op whoami --format json`; failure is a plain `Error` → exit **2**)
3. log `done: 1Password authentication ok`
4. `loadConfig` (logs `info: reading config item <item>`, `done: loaded config item <item>`)
5. log `info: inspecting project root <root>`
6. `inspectProject(config, root)` (validates root markers → `EnvswitchError` exit 1; resolves target paths)
7. stdout:
```
1Password authentication: ok
project root: <root>
targets: <N>
- <target.name> -> <absolute outputPath> (ok|missing parent)
```
`ok` iff `dirname(outputPath)` exists.

### 3.4 `envswitch update` (cli.ts:334-409) — see §6.

### 3.5 Help (`printHelp`, cli.ts:722-738)

Emitted as **one** `writeLine` call containing embedded newlines and a trailing `\n`; `console.log` adds another, so stdout ends with `item.\n\n` (blank last line) **(probed via `od -c`)**. Verbatim:

```
envswitch

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

```
Same text for every help path (no per-command help). Exit 0, stdout.

## 4. Output contract

### 4.1 Streams

| Content | Stream | Mechanism |
|---|---|---|
| Help, guided header, switch summary, list, doctor report, update result lines | stdout | `writeLine` default = `console.log` (cli.ts:471) |
| Progress events | stderr | `log` default = `logProgress` → `console.error` (108-110, 470) |
| Fatal error line `envswitch: <message>` | stderr | `console.error` directly in `main` catch (183) — **not** routed through injected `log`/`writeLine` |
| Inquirer prompt UI | `deps.output` (default `process.stdout`) | cli.ts:461, 501, 517 |
| Installer child process | inherited stdin/stdout/stderr | cli.ts:402-407 |

Guided header lines use `writeLine` (console.log → stdout), not `deps.output`.

### 4.2 Progress formatting — `formatProgressEvent` (cli.ts:112-126, exported)

- No color: `envswitch [<status>] <message>`
- Color: `"\x1b[2menvswitch\x1b[0m" + " " + <statusColor> + "[<status>]" + "\x1b[0m" + " " + message`
- Status colors (132-141): `done` → `\x1b[32m` (green), `dry-run` → `\x1b[33m` (yellow), `info` → `\x1b[34m` (blue). Message is never colored.
- `ProgressStatus = "info" | "done" | "dry-run"` (types.ts:66).
- Color decision `useColor()` (128-130), evaluated **per event**: `stderr.isTTY && !process.env.CI && process.env.NO_COLOR === undefined`.
  - `CI` set to any non-empty string (including `"0"`/`"false"`) disables color; `CI=""` does not.
  - `NO_COLOR` present (even empty) disables color (stricter than no-color.org, which ignores empty).
  - `FORCE_COLOR` / `CLICOLOR*` ignored. stdout TTY irrelevant.

### 4.3 Switch / guided result summary — `printSwitchResults` (cli.ts:411-428)

```
<Dry run|Updated> <N> target(s) from <basename(root)>
- <target.name> -> <absolute outputPath>
  keys: <keyCount>
  layers: <name>:<item.title>, <name>:<item.title>   # joined ", "; "none" if empty
  overwritten: KEY_A, KEY_B                          # line omitted when no overwritten keys
```
Block repeated per target in `switchEnvironment` result order. `basename("/")` = `""` → `... from ` (trailing space).

### 4.4 Progress event catalog (CLI-originated)

| Origin | status:message |
|---|---|
| `loadConfig` (430-440) | `info:reading config item <logName>` / `done:loaded config item <logName>`. `logName` = flag value for switch/list/doctor; item **title** (fallback id) in guided mode (680-682). |
| doctor | `info:checking 1Password authentication`, `done:1Password authentication ok`, `info:inspecting project root <root>` |
| guided `askVault` (551-561) | `info:listing 1Password vaults`, `done:found <n> vault(s)`, `info:checking vaults for config items tagged envswitch-config`, per failure `info:skipping vault <vault.name> (could not list items)` (542), `done:found <n> vault(s) with config items` |
| update (355-358) | `info:updating envswitch <current or "(unknown version)"> -> <latest or "latest">` |

Switcher-originated (for reference, owned elsewhere): `validating project root <root>`, `listing candidate 1Password items`, `found <n> candidate item(s)`, `processing <name> -> <path>`, `reading <title|id>`, `writing <path>`, `done:wrote <path>`, `dry-run:would write <path>` (switcher.ts:74-108, 231).

### 4.5 Exit codes

| Code | Path |
|---|---|
| 0 | success; help; update already up to date |
| 1 | any `EnvswitchError` (all `fail(...)` calls: parse/validation, unknown command/subject, config/parse errors, root-marker failures, `op` JSON-shape errors, update preconditions, installer non-zero) |
| 2 | any other thrown value: `op` subprocess failures (`ExecCommandRunner` throws plain `Error` with op's stderr or e.g. `spawn op ENOENT`; onepassword.ts:13-26), fs errors from writing `.env`, `Bun.spawn` failure (`Executable not found in $PATH: "bash"`), non-cancel inquirer errors, `AbortPromptError` |
| 130 | prompt cancel: `error.name === "ExitPromptError"` (684-686); **nothing printed** |
| 128+N | process killed by signal outside a prompt (Bun default SIGINT handling → shell sees 130) — not produced by `main` |

`main` never returns other codes. The installer's own exit code is **not** propagated: non-zero → exit 1 with message.

## 5. Guided mode (`envswitch` with no args; cli.ts:225-263, 529-664)

### 5.1 Preconditions
- `isInteractive = options.isInteractive ?? (stdin.isTTY && stdout.isTTY)` (466, 522-527). stderr TTY not considered.
- If `!isInteractive && !prompt` → exit 1: `envswitch: plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts` (226-230). An injected `prompt` bypasses the TTY check.
- Real prompter = `createInquirerPrompt(deps.input, deps.output)` (478-520).

### 5.2 Sequence

1. stdout: `envswitch guided switch`, then `Project root: <root>` (233-234) — printed before any 1Password call.
2. **Vault** (`askVault`, 547-574):
   - log `info:listing 1Password vaults`; `listVaults()` (= `op vault list --format json`). Errors propagate (exit 2 for op failure; 1 for JSON-shape `fail`).
   - log `done:found <n> vault(s)`; if 0 → `fail("no 1Password vaults found")`.
   - log `info:checking vaults for config items tagged envswitch-config`.
   - **Parallel** (`Promise.all`) per vault, `findConfigVault` (529-545): `vaultId = vault.id || vault.name`; `op = createOnePasswordClient(vaultId)`; `op.listItemsByTags(["envswitch-config"])`. Empty → excluded. **Any** error (including `EnvswitchError`) → excluded + log `info:skipping vault <vault.name> (could not list items)`. Results keep `listVaults` order.
   - log `done:found <n> vault(s) with config items`; 0 → `fail("no 1Password vaults contain items tagged envswitch-config")`.
   - `selectOrAuto` message `Vault`, choices `{label: formatVaultLabel(vault), value: vaultId}`, **no default**.
   - The client created during the scan is **reused** for all later calls (no second `createOnePasswordClient`; asserted by test).
3. **Config item** (`askConfigItem`, 576-596): items from that vault's scan. (Empty check → `no config items tagged envswitch-config found in the selected vault` is unreachable dead code.) `selectOrAuto` message `Config item`, `{label: formatItemLabel(item), value: item.id || item.title}`, no default.
4. `loadConfig(op, itemId, log, logName=title||id)`.
5. **Client** (`askChoiceOrValue`, 598-615): `config.clients ?? []`.
   - 0 choices (absent or `[]`) → `input({message: "Client", defaultValue: undefined, required: true})`.
   - else `selectOrAuto` message `Client`, label=value=client, **no default**.
6. **Environment**: same helper, label `Environment`, choices `config.environments ?? []`, default `defaultEnvironment(config)` (658-664): `local` if list empty or contains `local`, else first entry. Input fallback: `input({message: "Environment", defaultValue: "local", required: true})`.
7. **Target** (`askTarget`, 617-630): if exactly 1 target → `all` with no prompt. Else `select` message `Target`, choices `["all", ...target names]` (label=value), default `all`.
8. **Mode** (`askBoolean`, 647-656): always prompted (2 choices, never auto). Message `Mode`, choices `[{label:"Dry run", value:true}, {label:"Write .env files", value:false}]`, default `false` → cursor starts on "Write .env files"; Enter = write.
9. `switchEnvironment(...)` then `printSwitchResults` (same as `switch`).

Label formatting:
- `formatVaultLabel` (666-671): `name` if `!id || id === name`, else `"<name> (<id>)"`.
- `formatItemLabel` (673-678): `title || id` if `!id || id === title`, else `"<title || id> (<id>)"`.

`selectOrAuto` (632-645): exactly 1 choice → return it silently (no prompt, no log, nothing shown to the user); otherwise `prompt.select`. Applies to Vault, Config item, Client, Environment, Target (target is short-circuited earlier anyway).

Selection lookup uses `find` on value; duplicate vault ids/item ids resolve to the first match.

### 5.3 `GuidedPrompt` abstraction (cli.ts:25-36)

```ts
select<T>({message, choices: {label, value: T}[], defaultValue?: T}): Promise<T>
input({message, defaultValue?: string, required?: boolean}): Promise<string>
```
Inquirer adapter mapping: `label → name`; `loop: false`; `default` only set when `defaultValue !== undefined`; `input.required` passed through; `{input, output}` streams passed as context.

### 5.4 Inquirer UX facts to mirror (or consciously drop)

- select: page size 7 (default), no wraparound (`loop: false`), ↑/↓ only (vim `j`/`k` keybindings off by default), number keys jump to Nth item, typing jumps to first item whose name starts with typed text (700 ms buffer), footer `↑↓ navigate • ⏎ select`, idle prefix blue `?`, message bold, active item cyan with `❯` pointer, done line `✔ <message> <choice label>` (green tick, cyan answer). Prompt is **not** cleared after answering (`clearPromptOnDone` false).
- input: shows default as dim `(local)`; Enter on empty → default; Tab on empty fills default into the buffer; Backspace on empty clears the default; `required` + empty answer → red `> You must provide a value`, re-prompts. **Answer is not trimmed** (whitespace-only answers pass `required`; later rejected by switcher identifier validation as `client contains unsupported characters` / `environment contains unsupported characters`, exit 1).
- Esc does **not** cancel.
- Cancellation: readline `SIGINT` (Ctrl-C in raw mode) → `ExitPromptError("User force closed the prompt with SIGINT")`; process exit signals via `signal-exit` → `ExitPromptError("User force closed the prompt with <code> <signal>")` (@inquirer/core create-prompt.js:66-73). `main` maps either to exit 130 with no stderr line.
- Ctrl-C **outside** a prompt (during `op` calls) is not caught: Bun dies from SIGINT; `op` children in the same process group also receive it.

### 5.5 How tests use `MainOptions` (cli.ts:43-56)

| Field | Used by tests | Purpose |
|---|---|---|
| `prompt` | guided tests (`FakePrompt`, `CancelPrompt`) | replaces inquirer; also bypasses TTY check |
| `isInteractive` | guided tests (`true`), non-TTY test (`false`) | override TTY detection |
| `root` | guided tests (`/tmp/envswitch-project`) | replaces `process.cwd()` |
| `listVaults` | guided tests | replaces `op vault list` |
| `createOnePasswordClient` | guided tests (`FakeClient`, `FakeClientFactory`) | replaces `new OnePasswordClient(vault)` |
| `log` | all | capture/silence progress events |
| `writeLine` | most | capture stdout lines |
| `appDir` | update tests (mkdtemp) | replaces `resolve(import.meta.dir, "..")` |
| `fetchLatestVersion` | update tests | replaces GitHub API call |
| `runInstaller` | update tests | replaces `Bun.spawn` |
| `input`, `output` | **never** | only for the real inquirer path |

stderr is captured by monkey-patching `console.error` in `runCli` (cli.test.ts:463-483); `console.log` is silenced the same way.

## 6. `update` command (cli.ts:334-409)

Steps, in order:
1. `validateFlags(args, {help})` → e.g. `update --vault x` → `unknown flag: --vault`.
2. **Source-checkout guard**: `existsSync(join(appDir, ".git"))` (file or dir — worktrees/submodules count) → `fail("envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)")`.
   - `appDir` default = `resolve(import.meta.dir, "..")` = parent of `src/`, i.e. the installed `<APP_ROOT>/current` (or repo root when run from source).
3. **Installer lookup** (OS branch #1, line 341): `process.platform === "win32"` → `install.ps1`, else `install.sh`; path `join(appDir, "scripts", name)`. Missing → `fail("installer not found at <path>; reinstall envswitch with the install script from the README")` (path uses OS separators).
4. **Current version**: `readVersionFile(join(appDir, ".version"))` (368-375): `Bun.file(path).text()`, trimmed; empty or any read error → `undefined`.
5. **Latest version**: `fetchLatestReleaseTag()` (381-396): `GET https://api.github.com/repos/rnldsalili/envswitch/releases/latest`, header `Accept: application/vnd.github+json`, plus `Authorization: Bearer $GITHUB_TOKEN` if set. Uses Bun's default `User-Agent` (GitHub rejects requests without one). **No timeout.** Non-2xx, network error, bad JSON, missing/empty/non-string `tag_name` → `undefined`. Never throws.
6. **Short-circuit**: if `latest && current && normalizeVersion(latest) === normalizeVersion(current)` → stdout `envswitch is already up to date (<current raw>)`, exit 0. `normalizeVersion` strips one leading `v` only (377-379). No semver ordering: a local version newer than the latest release still triggers reinstall of latest.
7. log `info:updating envswitch <current ?? "(unknown version)"> -> <latest ?? "latest">`.
8. **Run installer**: `runInstaller(installerPath, dirname(appDir))` — second arg is `APP_ROOT` (parent of `current`).
   - Default `runInstallScript` (398-409), OS branch #2 by **file extension**:
     - `.ps1` → `powershell -NoProfile -ExecutionPolicy Bypass -File <script> -AppRoot <appRoot>` (Windows PowerShell 5.1 binary name, not `pwsh`).
     - else → `bash <script>`.
   - `Bun.spawn` with stdin/stdout/stderr `inherit`, env = full `process.env` + `APP_ROOT=<appRoot>`. `INSTALL_DIR` is **not** passed → on Unix the launcher is rewritten to the default `$HOME/.local/bin` even if originally installed elsewhere; on Windows `-InstallDir` defaults to `<AppRoot>\bin` (install.ps1:281-283).
   - Return `proc.exited`: exit code, or `128 + signum` if killed by a signal (probed: SIGTERM → 143). Missing `bash`/`powershell` → `Bun.spawn` throws plain `Error` → exit 2.
   - The installer independently re-resolves "latest" (install.sh:198-221) and writes `<APP_ROOT>/current/.version` (install.sh:334; install.ps1:311).
9. Non-zero → `fail("update failed: installer exited with code <code>")`.
10. Re-read `.version`; stdout `updated envswitch <current ?? "(unknown)"> -> <updated ?? latest ?? "latest">`. Note the two different placeholders: `(unknown version)` in the log, `(unknown)` in the result line.

OS-specific branches summary: cli.ts:341 (installer name), cli.ts:399-401 (interpreter by extension), `path.join` separators in messages, cli.test.ts:293 (test-side installer name).

## 7. Error handling

- Single class `EnvswitchError extends Error`, `name = "EnvswitchError"` (errors.ts:1-6); `fail(message): never` throws it (errors.ts:8-10). All user-facing validation messages go through `fail`.
- `main` catch (cli.ts:178-185): cancel → 130 silently; else print `envswitch: <message>` (`error.message` for `Error`, `String(error)` otherwise) → 1 for `EnvswitchError`, 2 otherwise.
- **No stack traces are ever printed.** No `cause` chaining. Messages from `op` stderr are passed through verbatim (may contain `[ERROR] <timestamp> ...` text).
- `findConfigVault` swallows every error per vault (541-544) — the only intentional swallow besides `readVersionFile` and `fetchLatestReleaseTag`.

Complete list of CLI-layer `fail` messages:
`unknown command: <cmd>` · `unknown flag: --<name>` · `--<name> does not take a value` · `--<name> requires a value` · `--<name> is required` · `unknown list subject: <s>` · `plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts` · `no 1Password vaults found` · `no 1Password vaults contain items tagged envswitch-config` · `no config items tagged envswitch-config found in the selected vault` (unreachable) · `envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)` · `installer not found at <path>; reinstall envswitch with the install script from the README` · `update failed: installer exited with code <n>`.

## 8. Interfaces `cli.ts` consumes

```ts
// onepassword.ts
OnePasswordClient(vault: string)                                // ctor; no I/O
OnePasswordClient.listVaults(): Promise<{id,name}[]>             // op vault list --format json
.assertAuthenticated(): Promise<void>                            // op whoami --format json
.listItemsByTags(tags: string[]): Promise<{id,title,tags?}[]>    // op item list --vault V --format json [--tags a,b]
.readConfigItem(item: string): Promise<string>                   // field labeled "config"
.readEnvItem(item): Promise<Record<string,string>>               // used by switcher
// config.ts
parseConfig(raw: string): EnvswitchConfig
tagValues(config, {layer}): {layer?: string, ...}
// switcher.ts
switchEnvironment(config, op, {root, env, client, targetName, dryRun, log?}): Promise<TargetSwitchResult[]>
  // TargetSwitchResult = {target, outputPath(abs), written, keyCount, layers: EnvLayer[], overwrittenKeys}
inspectProject(config, root): Promise<{target, outputPath(abs), parentExists}[]>
```
Types (types.ts): `OnePasswordVaultSummary {id, name}`, `OnePasswordItemSummary {id, title, tags?}`, `EnvswitchConfig {project?, clients?, environments?, targets (non-empty), tags?}`, `LayerName`, `ProgressEvent {status, message}`.

`root` defaults to `process.cwd()` (467), not canonicalized; summary uses `basename(root)`.

## 9. Test inventory (`test/cli.test.ts`, 23 tests)

Fakes: `runCli` (463-483) patches `console.error`/`console.log` and calls `main` in-process. `FakePrompt` (485-540) records every select/input call and pops scripted answers; when answers run out it returns `defaultValue` or throws. `CancelPrompt` (542-552) throws `Error{name:"ExitPromptError"}`. `FakeClient` (613-675) returns a fixed config JSON, config items for `["envswitch-config"]` (or throws / defers), and one `beta-client` item for any other tag query; `readEnvItem` → `{CLIENT_ONLY: "1"}`. `FakeClientFactory` (590-611) records vault ids; only `vault-a` has a config item. `DeferredVaultScan` (554-588) holds `listItemsByTags` promises open to prove concurrency.

Legend — Rust seam: **P** = trait-injected `Prompter`; **OP** = trait-injected `OpClient`/`OpFactory`/`VaultLister` (in-process) or fake `op` on `PATH` (black-box); **U** = injected `ReleaseSource` + `InstallerRunner`; **E** = injected stdout/stderr `Write` sinks; **AC** = `assert_cmd` black-box; **SNAP** = `insta`.

| # | Test (line) | Asserts | Fakes | JS/Bun-specific? → Rust seam |
|---|---|---|---|---|
| 1 | formats plain progress output without ANSI colors (8) | `envswitch [info] reading config item demo-envswitch` | none | No. Pure fn unit test `format_progress_event(ev, color=false)`. |
| 2 | formats colored status labels when color is enabled (15) | exact ANSI bytes for `done` (green) and `dry-run` (yellow) | none | No. Pure fn; byte-exact string compare (do not use a color crate that might emit different SGR). |
| 3 | rejects unknown flags before command execution (27) | `switch --unknown` → 1, stderr `["envswitch: unknown flag: --unknown"]` | console.error patch | stderr capture is JS-specific → **E** (`run(argv, deps, &mut out, &mut err) -> i32`) or **AC**. |
| 4 | rejects boolean flags passed with values (34) | `switch --dry-run=false` → 1, `--dry-run does not take a value` (before `--vault is required`) | same | **E**/**AC**. |
| 5 | uses command-specific flag allowlists (41) | `list --client alpha` → 1, `unknown flag: --client` | same | **E**/**AC**. |
| 6 | parses omitted optional switch flags so defaults can apply (48) | `parseArgs` output: absent optional flags are absent keys, empty positional | none | Unit test on `parse_args` returning `ParsedArgs { command: Option<String>, positional: Vec<String>, flags: IndexMap<String, FlagValue> }`. |
| 7 | runs a guided switch when no command is provided (60) | exit 0; client created for `vault-a`,`vault-b` exactly once each in order (reuse); tag calls include `["envswitch-config"]` and `["envswitch","project:demo","env:dev"]`; prompts in order `Client, Environment, Target, Mode` (Vault/Config auto-selected); client choices `alpha,beta`; env default `local`; target default `all`; mode choices `Dry run:true`, `Write .env files:false`; stdout contains `envswitch guided switch`, line starting `Dry run 1 target(s) from envswitch-project`, `  layers: client:beta-client`; log `info:reading config item demo config` (title, not id); `readConfigItem` called with id `cfg`; log `info:checking vaults for config items tagged envswitch-config` | FakePrompt answers `beta, dev, api, true`; FakeClientFactory; injected root/log/writeLine | Generic `select<T>` + JS value equality. → **P** with object-safe `select(&self, msg, labels: &[String], default: Option<usize>) -> Result<usize, PromptError>`; fake records `(message, labels, default)` and answers by label/value lookup. **OP** fake factory recording created vault ids and tag queries. Use a `tempfile` root (see note A). |
| 8 | skips inaccessible vaults during guided discovery (107) | exit 0; log `info:skipping vault Bad Vault (could not list items)` | FakeClient `failConfigList` for `vault-bad`; Mode falls back to default (write) | **P**, **OP** (fake returns `Err`). Note A applies (writes files). |
| 9 | scans config-tagged vault items in parallel (129) | both vault scans start before either resolves; exit 0 | `DeferredVaultScan` with unresolved promises | Promise/microtask mechanics are JS-specific. → **OP** fake whose `list_items_by_tags` blocks on a `std::sync::Barrier::new(2)` (or channel pair) with a test timeout; a sequential implementation deadlocks/times out. Note A applies. |
| 10 | uses write mode as the guided default (156) | last select is `Mode` with default `false`; stdout line starts `Updated 2 target(s)` | FakePrompt answers run out → returns default | Fake-returns-default behavior must be reimplemented in the Rust fake prompter. Note A applies. |
| 11 | falls back to input when config omits clients or environments (175) | only select is `Mode`; inputs `[{Client, undefined, required:true}, {Environment, "local", required:true}]`; target auto (1 target); `Dry run 1 target(s)` | FakeClient custom config; input answers `alpha, local`, select `true` | **P** (`input(&self, msg, default: Option<&str>, required: bool) -> Result<String, PromptError>`). |
| 12 | shows select prompts when guided choices have multiple options (201) | prompts `Vault, Config item, Client, Environment, Target, Mode`; vault labels `Vault A (vault-a)`/`Vault B (vault-b)` with id values; config labels for **selected** vault-b: `vault-b config (cfg-b)`, `vault-b other config (cfg-b-other)`; `Updated 2 target(s)` | per-vault config items; answers `vault-b, cfg-b, alpha, local, all` + default Mode | **P**, **OP**. Note A applies. |
| 13 | rejects guided mode when no vault has a config-tagged item (249) | exit 1; stderr exactly `["envswitch: no 1Password vaults contain items tagged envswitch-config"]` (progress went to injected log) | FakeClient with `[]` items | **P**, **OP**, **E**. |
| 14 | returns 130 without an error line when the user cancels a prompt (264) | exit 130; stderr empty | `CancelPrompt` throws `ExitPromptError` at first select (Vault) | Error-by-name detection is JS-specific → `PromptError::Cancelled` variant from the fake **P**; map to 130 in `run`. |
| 15 | rejects guided mode without an interactive terminal (282) | exit 1; exact non-TTY message | `isInteractive:false`, no prompt | Inject `is_interactive: bool` (production: `std::io::IsTerminal` on stdin && stdout). Also inject a panicking `VaultLister` so a broken guard cannot hit real `op` (the TS test would). **AC** variant: run binary with piped stdin/stdout. |
| 16 | short-circuits when already on the latest version (316) | exit 0; installer not called; stdout `["envswitch is already up to date (v0.0.5)"]` | mkdtemp `<tmp>/current` with `scripts/install.sh` + `.version` `v0.0.5\n`; fetch → `v0.0.5` | **U** + `tempfile`. Installer filename via `cfg!(windows)`. |
| 17 | runs the installer and reports old -> new version (337) | installer called once with `(<appDir>/scripts/<installer>, dirname(appDir))`; stdout `["updated envswitch v0.0.4 -> v0.0.5"]` | fake installer rewrites `.version` | **U** (fake runner closure writes file). |
| 18 | rejects updates from a source checkout (359) | exit 1; installer not called; exact source-checkout message | `.git` dir in appDir | **U**, **E**. |
| 19 | fails when the bundled installer is missing (380) | exit 1; `installer not found at <path>; ...` with platform path | no `scripts/` dir | **U**, **E**. Build expected path with `Path::join` so Windows separators match. |
| 20 | fails when the installer exits non-zero (396) | exit 1; `update failed: installer exited with code 1` | runner → 1 | **U**, **E**. |
| 21 | updates when the installed version is unknown (410) | `updated envswitch (unknown) -> v0.0.5` | no `.version`; runner writes it | **U**. |
| 22 | updates even when the latest version cannot be resolved (429) | installer runs; `updated envswitch v0.0.4 -> v0.0.4` (re-read file unchanged) | fetch → `undefined` | **U** (`fetch_latest() -> Option<String>`). |
| 23 | rejects unknown update flags (450) | `update --vault x` → 1, `unknown flag: --vault` | — | **E**/**AC**. |

Note A (test hygiene bug to fix, not port): tests 8, 9, 10 and 12 run in **write** mode with `root: "/tmp/envswitch-project"` and the real `writeEnvFile`, so they create real files under `/tmp/envswitch-project/services/api/.env` and `/tmp/envswitch-project/apps/web/.env` (observed after `bun test test/cli.test.ts`). Rust tests should use `tempfile::TempDir` roots, or inject the writer.

Coverage gaps worth adding in Rust: help text snapshot (none today), `--version` behavior, `list` subjects/`(none)`, `doctor` output, `switch` happy/dry-run output, exit code 2 path, color env-var matrix (`CI`, `NO_COLOR`, non-TTY), `-h` only as argv[0], `--help` precedence, `requires a value` vs `is required`.

Recommended Rust test layering:
1. Unit: `parse_args`, `validate_flags`, `format_progress_event`, label formatters, `default_environment`, `normalize_version`.
2. In-process: `run(argv, &mut Deps) -> i32` with `Deps { prompter: Option<Box<dyn Prompter>>, is_interactive, root, op: Box<dyn OpFactory>, vaults: Box<dyn VaultLister>, log: Box<dyn FnMut(ProgressEvent)>, out: Box<dyn Write>, err: Box<dyn Write>, app_dir, release: Box<dyn ReleaseSource>, installer: Box<dyn InstallerRunner> }` — ports tests 3-23 nearly 1:1.
3. Black-box: `assert_cmd` + `insta` with a fake `op` script prepended to `PATH` (shell script on Unix, `op.cmd`/`op.exe` shim on Windows) for switch/list/doctor end-to-end; `expectrl`/`rexpect` PTY tests optional for a smoke test of real prompts and Ctrl-C → 130.

## 10. Rust-port risks (CLI layer)

1. **Arg parsing parity.** `clap` would change: exit code 2 for usage errors (TS uses 1), error text/format, auto `--version`/`-V`/`-h` everywhere, `--` handling, value-vs-flag disambiguation, help layout. Recommend a hand-rolled parser (~60 LOC) mirroring §1-2 exactly, with `IndexMap` for flag order. If clap is mandated, use `disable_version_flag`, `disable_help_flag`, `allow_hyphen_values`, custom error rendering and `exit(1)` — still unlikely to match all quirks.
2. **Prompt UX.** `@inquirer` behaviors (no wrap, type-to-jump prefix search, number-key jump, Tab-fills-default, unpersisted Esc, `✔` done line) are not reproduced 1:1 by any crate. `inquire` is closest (Select with `with_starting_cursor`, page size 7 default, Text `with_default` + `required!` validator) but: Esc yields `OperationCanceled` (inquirer ignores Esc) — decide whether Esc = 130 (recommend yes, document); filtering is substring-filter not prefix-jump; verify whether wraparound can be disabled. `dialoguer` Select has `default()`/`interact_opt()` but Ctrl-C can leave the cursor hidden unless you install a handler that calls `console::Term::show_cursor`.
3. **Prompt output stream.** Inquirer renders to stdout; `inquire` and `dialoguer` default to **stderr**. Harmless for TTY sessions (guided requires stdout TTY) but tests or users piping stderr will see a difference. `dialoguer::Select::interact_on(&Term::stdout())` can match; `inquire` stream choice is limited — verify.
4. **Ctrl-C semantics.** In raw mode, Ctrl-C is a keypress, not a signal; prompt crate must surface it as an error → return 130 silently. Outside prompts, default signal death is fine on Unix (shell shows 130) but Rust drops nothing to stderr either — matches. On Windows, Ctrl-C kills with `STATUS_CONTROL_C_EXIT` (0xC000013A), not 130; accept or install `ctrlc` handler that exits 130. Child `op` processes receive SIGINT via process group on Unix; on Windows `CREATE_NEW_PROCESS_GROUP` is not set by `std::process::Command`, so they also get the console Ctrl-C.
5. **Exit code taxonomy.** Must preserve 1 (user/validation, `EnvswitchError`) vs 2 (subprocess/IO/unexpected) vs 130 (cancel). Model as `enum CliError { User(String), Internal(String), Cancelled }`; avoid `anyhow` at the boundary or tag errors so the 1-vs-2 split survives `?`. Never print `Debug`/backtraces; print `envswitch: {msg}` only. Avoid `fn main() -> Result<..>` (prints `Error: ...` and exits 1).
6. **Color/TTY detection.** Replicate exactly: stderr is TTY (`std::io::stderr().is_terminal()`), `CI` unset-or-empty, `NO_COLOR` unset (empty counts as set). Emit hard-coded SGR bytes (tests compare bytes). On Windows, enable VT processing first (`enable-ansi-support` or `anstyle-query::windows::enable_ansi_colors`) or old consoles show raw escapes. Don't let a color crate honor `FORCE_COLOR`/`CLICOLOR` unless deliberately adopted.
7. **Parallel vault scan.** Must be concurrent (test 9) and order-preserving. `std::thread::scope` over vaults (each spawns `op`) is simplest; no async runtime needed. Bound concurrency if users have many vaults (TS is unbounded).
8. **`update` redesign.** Bun-specific assumptions break: `appDir` from `import.meta.dir`, `.version` beside sources, `bun` launcher, `.git` detection at app root. A Rust binary should derive the install dir from `std::env::current_exe()` (resolve symlinks), and either keep a `.version` file or use `env!("CARGO_PKG_VERSION")`. Running from `target/` (cargo) needs a replacement "source checkout" guard. Windows cannot overwrite a running `.exe`: installer must rename-then-replace (`self-replace` crate pattern) or the CLI must exit before the installer swaps files.
9. **GitHub API call.** Rust HTTP clients do not send a `User-Agent` by default; GitHub returns 403 without one → silently `None` → always reinstalls. Set an explicit generic UA (no user details), keep `Accept` and optional `GITHUB_TOKEN` bearer, add a timeout (TS has none and can hang).
10. **Installer invocation.** Keep `bash <script>` / `powershell -NoProfile -ExecutionPolicy Bypass -File <script> -AppRoot <root>`, inherited stdio, `APP_ROOT` env. `ExitStatus::code()` is `None` on Unix signal death — map to `128 + signal()` to match Bun's 143-style codes. Spawn failure must be exit 2 with a readable message. Consider passing `INSTALL_DIR` (TS doesn't; existing bug for custom launcher dirs).
11. **Help output.** Trailing blank line (`item.\n\n`) is part of current output; snapshot it.
12. **Path rendering.** Messages embed `path.join`/absolute paths; on Windows they use `\`. Rust `Path::display()` matches. `basename("/")` → empty string in TS; Rust `file_name()` is `None` → render `""`.

### Recommended crates

| Crate | Why |
|---|---|
| (none) hand-rolled parser + `indexmap` | Exact parity with quirky `parseArgs`; `IndexMap` keeps flag insertion order for first-error semantics. |
| `inquire` | Closest to @inquirer select/input (starting cursor, defaults, required validation, distinct Ctrl-C error). |
| `dialoguer` + `console` (alternative) | Can render on stdout via `interact_on`; needs cursor-restore handling on Ctrl-C. |
| `std::io::IsTerminal` | TTY detection for stdin/stdout/stderr; no dependency. |
| `enable-ansi-support` (Windows only) | Turns on VT processing so hard-coded SGR bytes render. |
| `ureq` (rustls) | Small blocking HTTP client for one GitHub call; easy UA/timeout. |
| `serde` / `serde_json` | `tag_name` parsing and `op` JSON. |
| `thiserror` | `CliError` enum preserving 1/2/130 mapping. |
| `ctrlc` (optional) | Normalize Windows Ctrl-C to exit 130 / restore cursor outside prompts. |
| `self-replace` (if binary self-updates) | Windows-safe replacement of the running executable. |
| `tempfile` | Temp app dirs / project roots in tests (fixes Note A). |
| `assert_cmd` + `predicates` | Black-box exit code and stream assertions. |
| `insta` | Snapshot help text, summaries, doctor/list output. |
| `expectrl` or `rexpect` (optional) | PTY smoke tests for real prompts and Ctrl-C → 130. |
