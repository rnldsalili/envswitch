# envswitch parity harness (F1, TS phase)

The frozen oracle is `53de029a25a717c8ac824ca8553035e2b0fec991`, Bun **1.4.0**,
ICU **78.1**, Unicode **17.0**, default locale **en-US**. The corpus contains **483** scenarios (474 on macOS/Linux; 480 on Windows).
No oracle source or
production test is edited. Everything uses synthetic data and fresh temporary
projects. No real 1Password invocation, installer, network update, or user profile
write is part of a scenario.

## Run

```sh
export PATH="$HOME/.cargo/bin:$PATH"
bash parity/gen.sh             # materialize, build/test/lint fake-op, generate this OS
bash parity/gen.sh --check     # independently reproduce and compare every transcript
bun test parity/self-check.test.ts
```

Windows can run the Bun scripts directly (no Bash required):

```powershell
$env:LANG = 'en_US.UTF-8'; $env:LC_ALL = 'en_US.UTF-8'
$env:BUN_RUNTIME_TRANSPILER_CACHE_PATH = ''
bun parity/oracle/materialize.ts "$env:TEMP\envswitch-oracle"
$env:ENVSWITCH_ORACLE_DIR = "$env:TEMP\envswitch-oracle"
bun parity/generate.ts
bun parity/generate.ts --check
bun test parity/self-check.test.ts
bun "$env:ENVSWITCH_ORACLE_DIR/node_modules/typescript/bin/tsc" -p parity/tsconfig.json --typeRoots "$env:ENVSWITCH_ORACLE_DIR/node_modules/@types"
```

The destination of the materializer must be empty. It runs `git archive` at the
full SHA with `core.autocrlf=false`, `bun install --frozen-lockfile`, and writes `oracle-metadata.json`.
`parity/oracle/materialize.sh` delegates to the same cross-platform Bun script.
The materializer's package cache and HOME/profile roots live inside that temporary
export. Generation creates/removes its own export unless `ENVSWITCH_ORACLE_DIR`
points to an already materialized export. Use that variable to reuse an export;
never point it at the moving repository checkout. Reused exports are verified
against the pinned src tree: filenames, Git blob hashes and Unix executable bits.

`generate.ts` copies the workspace manifests, committed Cargo.lock and crates into
an isolated temporary build and builds/tests/lints fake-op with **--locked**.
Only fake-op's workspace package entry was added to the shared lock. It never
resolves dependencies without the lock. `ENVSWITCH_PARITY_CARGO_HOME` can select
an existing private Cargo cache; otherwise the build creates/removes its own.
HOME/profile roots remain isolated. Rust 1.100.0, rustfmt and clippy must exist.

macOS and Linux goldens are committed. Every OS with a committed baseline runs
`--check` exactly once in CI. If Windows goldens are missing, CI generates and
uploads `windows-parity-goldens`, then **fails** with a message to commit them;
a fresh self-generated baseline is never accepted as parity. The workflow also
checks `build-scenarios.ts --check`, validates every golden against the schema,
and compiles/tests the Rust adapter while its integration test stays ignored.

Linux goldens were generated under Bun 1.4.0 copied from `oven/bun:1.4.0` into
Debian bookworm with Rust 1.100.0 and the en_US.UTF-8 locale. Run as a nonroot
user so the EACCES fixture remains meaningful. `parity/linux.Dockerfile` describes
the environment. When the host checkout's gitdir is inaccessible to Docker, fetch
a local `git bundle` of rust-port into a container temp repository and set
`GIT_DIR` to it; the materializer still exports/verifies the original oracle SHA.

To run one scenario or compare two transcripts:

```sh
bun parity/adapters/ts/run.ts parity/scenarios/cli-help-long.json /path/to/export
bun parity/diff/compare.ts expected.json actual.json
bun parity/blackbox/run.ts parity/scenarios/blackbox-doctor.json /path/to/fake/op bun /path/to/export/src/cli.ts
# Or pass a native CLI binary as the command in the last example.
```

Goldens live in `goldens/{macos,linux,windows}/`, one JSON per scenario plus
`_metadata.json`. Generation contains no timestamps. Reproduction compares every
field, including decoded stdout/stderr, and rejects missing/stale goldens or
runtime-metadata changes. Scenario construction is reproducible with
`bun parity/build-scenarios.ts`; that maintainer tool rewrites the input JSON
collection and removes stale scenario files. Goldens are never hand edited.

## Scenario protocol, version 1

Each `scenarios/*.json` is **inputs only**, with a filename equal to `<id>.json`.
Required fields are `id` and `kind`; optional `platforms` is an allowlist of
`macos`, `linux`, `windows`. Optional `divergence` is a row ID or a nonempty array of approved row IDs
in `docs/rust-port/DIVERGENCES.md` and never changes the captured TS outcome.

| Kind | Inputs / exported oracle entry point |
| --- | --- |
| `parseArgs` | `argv: string[]` → `cli.parseArgs` |
| `formatProgressEvent` | `event: {status,message}`, `color: boolean` → formatter |
| `cli` | `argv`, optional `prompts`, `isInteractive`, `tty`, `update` → in-process `main` |
| `switch` | `config`/`configRaw`, `options: {env,client,targetName,dryRun,root}`, optional `log: false` → `switchEnvironment` |
| `inspect` | `config`/`configRaw`, optional `root` → `inspectProject` |
| `classify` | config, `item`, `params: {env,client,target}` → `classifyItemForTarget` |
| `blackbox` | `argv`, fixtures → actual CLI child process with native fake `op` |
| `config` | `config` JSON value or verbatim `configRaw` → `parseConfig` |
| `tags` | config, `params` → `tagValues` and `candidateListTags` |
| `path` | `output`, optional `root` → `resolveOutputPath` |
| `dotenv` | `values`, optional `write`, `output`, `umask` → `stringifyEnv` / `writeEnvFile` |
| `op` | `method: auth|vaults|items|config|env`, `vault`, `tags` / `item` → exported `OnePasswordClient` methods |

Config defaults to one `api` target at `apps/api/.env`, group `apis`. The default
vault is `alpha`, config-item reference `cfg`, environment `local`, client `alpha`,
target selection `all`, and switch mode **write**. `config: null` is an explicit
null input, not a request for the default. `configRaw` takes precedence and allows
malformed JSON, duplicate keys and engine-specific diagnostics.

Common inputs:

- `files`: ordered setup operations `{path,content?,base64?,mode?,type?,link?,remove?}`.
  Relative paths are below ROOT; absolute fixture paths must remain inside TMP.
  `type: "dir"` creates a directory; `link` creates a directory symlink (a junction
  on Windows). `type: "file"` with `link` creates a file symlink (Unix scenarios).
  `remove: true` recursively removes an entry. Setup uses umask 022. A scenario's
  `umask` applies to oracle operations, then the previous mask is restored.
- `root` overrides the project root passed to exports; `cwd` changes process cwd
  independently (the directory must be created in `files`). Default cwd is ROOT.
- `env`: temporary environment overrides, with null meaning unset. CI, NO_COLOR,
  FORCE_COLOR, CLICOLOR, CLICOLOR_FORCE, GITHUB_TOKEN and known OP credentials are
  cleared by default. HOME/USERPROFILE/XDG/APPDATA roots are always private and
  cannot be overridden. LANG/LC_ALL are en_US.UTF-8 for D8.
- `tty: {stdin?,stdout?,stderr?}`: CLI stream TTY flags, default false.
  `isInteractive` is passed only when explicitly present. stderr TTY exercises
  the oracle's own default progress logger and color decision; stdout TTY has no
  effect on progress colors.
- `op`: shorthand fixture with `vaults`, `configItems`, `items`, `values` (map of
  item ref → env map), `details` (map of item ref → raw item JSON), and `byVault`
  overrides for those fields. Default vaults: `[{id:"alpha",name:"alpha"}]`;
  config items: `[{id:"cfg",title:"demo-suite",tags:["envswitch-config"]}]`;
  candidate items: empty. Config detail `cfg` is generated from the scenario.
- `op.responses`: exact-argv responses `{argv,stdout?,stderr?,exitCode?}`, taking
  precedence over shorthand. An unmatched invocation is a harness error, never
  a golden. `stdoutBase64` supplies arbitrary bytes, and
  `stdoutSize: {byte,size}` streams a repeated byte without large fixture files.
  Legacy `stdoutRepeat: {value,count}` expands into that size fixture. Both
  exactly 52428800 bytes and one byte above it exercise the real stdout cap. `candidateTags: string[]` declares exact
  comma-joined candidate queries for nonstandard templates/envs; standard local,
  dev and demo-suite queries are supplied by default. Fixture matching never
  implements classification: every returned candidate is rechecked by the oracle.
- `prompts`: ordered answers across select and input calls. A string/boolean
  answers by value; `{index:N}` selects a choice; `{default:true}` accepts the
  supplied default. Exhausted answers also accept a supplied default, otherwise
  fail the harness. A required input explicitly answered `""` is also a harness
  fault (whitespace remains valid at the prompt seam). `{cancel:true}` throws `ExitPromptError`; `{error:{name,message}}`
  throws a different scripted error. Unused answers are a harness failure.
  Exactly one choice auto-selects in the oracle and consumes **no** scripted answer.
- `parallelScans: N` holds config-tag list responses behind a 10-second barrier
  until N scans have started, proving concurrent discovery. Response order and
  factory reuse remain visible in the transcript.
- `mutateOnRead: {item,files}` applies setup operations immediately before that
  item's fake response, once (used for the symlink swap before writing).
- `update: {latest?,updatedVersion?,exitCode?,fetchError?,installerError?}` injects
  the release lookup and installer. Only the fake installer may rewrite APP's
  `.version`; it never executes the fixture script. Create `.git`, `.version`
  and `scripts/{INSTALLER}` in `files` to exercise each precondition.

Placeholders are expanded recursively in scenario strings and object keys:
`{ROOT}` (fresh TMP/demo-suite), `{TMP}` (fresh mkdtemp), `{APP}` (TMP/app/current),
`{HOME}`, `{OUTSIDE}`, `{SEP}`, `{INSTALLER}` (`install.sh`/`install.ps1`),
`{DRIVE}` (Windows temp drive including colon; `C:` on POSIX). Only known absolute
sandbox paths are replaced in transcripts, longest first. Native separators are
preserved. Random writer suffixes in filesystem diagnostics become
`.{PID}.{UUID}.tmp`; this does not hide the syscall, errno, destination or cleanup.
Binary streams and file bytes are base64 **after** replacing known UTF-8 path
sequences; CR/LF, ANSI, NUL, Unicode and final newlines otherwise remain exact.

## Transcript schema

The machine-readable schema is [transcript.schema.json](transcript.schema.json).
All fields always exist; absent function results are null (an undefined property
inside a returned JS object is omitted, as with `parseArgs([]).command`).

```text
{schemaVersion:1, id, kind, divergence:string|string[]|null,
 stdoutBase64, stderrBase64, exitCode:int,
 prompts:[{kind,message,choices:[{label,value}]|null,default,required:bool|null}],
 progress:[{status,message}], opCalls:string[][], readCounts:{"vault/item":int},
 factoryCalls:string[], updateCalls:[{method,scriptPath?,appRoot?}],
 files:[{path,kind:"file"|"symlink",bytesBase64?,target?,mode:int|null}],
 writtenFiles:[same], directories:[{path,mode:int|null}],
 leftoverTempFiles:string[], result:any|null}
```

`opCalls` excludes the black-box handshake and includes every actual argv after
`op`; `readCounts` includes config reads as well as env reads. The adapter injects
only the command transport, so argv and parsing come from the real exported
`OnePasswordClient`. `factoryCalls` proves the guided scan's clients are reused.
`files` is the complete final sandbox state (including unchanged setup files and
symlinks, excluding `.transport`); `writtenFiles` is its new/changed subset.
`directories` captures parent modes and absence of premature mkdir. Modes are
numeric Unix permission bits (0600 = 384, 0750 = 488), null on Windows. Symlink
modes are retained as observed on each OS. Directory traversal never follows links.
The Bun runtime transpiler cache is disabled, so it is never mistaken for
application writes. `leftoverTempFiles` lists final `.tmp` files. Failed writes retain observable old
file bytes in `files`.

Pure/library calls have no console output unless the export prints; thrown errors
are rendered with `envswitch: <message>\n` and the oracle's 1/2 error taxonomy.
Library progress goes only to the injected callback; `log:false` proves silence.
CLI output uses the oracle's real default logger/writeLine functions. Scripted
prompts record calls instead of synthesizing terminal UI. A cancelled prompt
returns 130 without a **fatal** line; earlier header/progress output remains.
Black-box `result` is null and `factoryCalls` is empty (these are in-process seams).

## Safety and diff policy

The native fake binary is named `op` / `op.exe`, has only `serde_json` as a direct
dependency, reads `ENVSWITCH_FAKE_OP_FIXTURE` as `{responses:[...]}`, and appends
argv JSONL to `ENVSWITCH_FAKE_OP_TRACE`. Missing or unknown fixtures fail closed
(exit 97). `op __envswitch_fake_handshake` prints `envswitch-fake-op-v1\n` and is
also traced. No subprocess is ever started by the fake.

The black-box runner requires an existing native fake at the given absolute path,
prepends its directory to PATH, verifies PATH resolution, and calls that exact
binary's handshake **before every CLI child**. Each native black-box run copies the CLI and fake into **separate private
directories**, so the missing-op case cannot resolve a sibling op.exe through
Windows executable-directory lookup. The copies are removed after the run.
An alternate `op`/`op.exe` in the CLI executable directory (or Windows System32)
is rejected before any handshake or CLI launch. CLI executables must be absolute.
The a6 scenario verifies the fake first, then uses a separate empty PATH directory
for the absolute CLI. A golden expecting opCalls cannot reproduce with an empty
invocation trace. A wrong token, stderr, nonzero
status, missing trace, missing fake or unknown invocation aborts the harness.
The CLI then uses that PATH to exercise real process spawning. Both successful
and failed command outputs, stderr trimming, secret stdout exclusion and the
50 MiB stdout limit are covered.

The diff runner compares **object key order recursively**, including result,
flags, env maps and readCounts. It prints field paths and decodes stdout/stderr for readable diffs;
any mismatch exits nonzero. Approved divergence markers are reported separately,
including when the TS baseline matches. They are **not blanket waivers**: the
Rust adapter explicitly validates each row's replacement assertion.
No arbitrary diff may be discarded merely because a scenario has a marker.

## Coverage and source-vs-contract notes

- 01 §9 rows 1–6: `parseArgs-*`, `formatProgressEvent-*`, flag/help CLI cases,
  including empty-next-token errata and JS object-key quirks (a3/a4).
- 01 §9 rows 7–15: `cli-guided-*`, including single-choice auto-selection,
  defaults, input fallback, all prompts/labels, factory reuse, parallel barrier,
  skipped unreadable/invalid vaults, cancellation and non-TTY guards.
- 01 §9 rows 16–23: `cli-update-*` plus `cli-unknown-update`. Original TS outcomes
  remain recorded with e1/e2/e3/e5 (and e6 for nonzero installer status); receipt, compiled-version and embedded-installer
  replacement assertions belong to the next Rust phase/D3.
- 02 §8.2 rows 1–9: `config-*`, `path-*`, `inspect-*`, `switch-*` validation,
  JS whitespace/null/presence/check order, all reachable config diagnostics,
  lexical traversal, `..env`, symlink/junction parents/root, ENOTDIR, root markers.
- Rows 10–11: `classify-*` for all five branches and rejects, foreign/multiple
  scopes, regex literals, bare/constant/repeated templates, common-tag checks,
  and CR/LF/U+2028/U+2029 at both the middle and end of each scope.
- Rows 12–16: merge/write/dry-run, overwritten-key ordering and identical/reverted
  values, API read order, duplicate/title cache keys, all-target read counts,
  no library printing, nontransactional writes, and the symlink swap hook.
- Rows 17–22: `op-*` lenient response parsing/field mapping/argv, plus black-box
  failure secrecy/maxBuffer. The source accepts lone-surrogate JSON strings;
  `op-lone-surrogate-value` records approved divergence a7 (02 §5.3).
- Rows 23–25: all 22 dotenv table entries plus empty map, key sort/errors, U+2029,
  create/replace/serialization-before-mkdir, modes/umask, rename cleanup and file
  symlink replacement on Unix. Expanded collation writes `WINNER` bytes for
  accents, canonical equivalence, punctuation, case, numeric, non-Latin and API ties.

The corrected empty-token and four-line-terminator contracts agree with source.
One remaining textual inconsistency must not guide the Rust implementation:
02 §2.3's suggested rejection of Windows `C:x` contradicts TS and the explicit
approved **parity** decision; Windows-only scenarios retain Node behavior.
DIVERGENCES c2 now correctly records TS no-wrap/prefix jump versus Rust
wrap/substring filtering; real navigation remains a separate PTY smoke lane.
The lone-surrogate value is marked **a7**: Rust rejects it through the normal
JSON parse-error path (exit 1 and `op item get <item> output is not valid JSON: `),
rather than silently replacing it.
Collation scenarios have **no g1 marker**: en-US is already fixed on the oracle,
so Rust must match their winners and stable order exactly.

## Rust adapter and integration gate

`crates/envswitch/tests/parity.rs` loads every applicable input and OS golden.
It drives all 12 kinds through the real Rust library/`cli::run`; the blackbox kind
uses the built native binary and the shared guarded runner. It builds fake-op
beside the CLI (same profile directory) when it is missing, so a clean checkout
needs no separate build step. The ignored test is:

```sh
# Optional implemented-S1 transport smoke, without running stubbed cli::run:
cargo test --locked --test parity scaffold_version_transcript -- --ignored --nocapture
ENVSWITCH_PARITY_FILTER=formatProgressEvent cargo test --locked --test parity -- --ignored --nocapture
# Filter can be an exact kind or a scenario-id prefix; omit it to run everything.
```

The runner catches each panic separately, cleans up its sandbox, continues, and
prints per-kind passed/failed totals. Each library case runs in an isolated Rust
test subprocess, capturing stray process stdout/stderr as well as injected
writers, with private HOME/PATH and private cwd/umask changes. It fails if nothing matches or any selected
scenario fails. Stubs are failures, never skips/passes. JSON objects use a small
ordered transport representation so serde_json's default map sorting cannot hide
ordering regressions. The bridge under `adapters/rust/` only shares setup, fixture
expansion, normalization, byte encoding and filesystem snapshots; **no TS
production function is called by the Rust lane**. `ENVSWITCH_PARITY_BUN` can select
an absolute Bun executable. `SharedBuf`, `EventLog`, `EnvVars`, `ScriptedPrompter`,
`ConcurrencyGate`, `FakeReleaseSource` and `FakeInstaller` are reused unchanged.

Each marked row has an explicit replacement assertion in `parity/divergences.rs`:
prototype fields/flags and integer order get exact replacement objects/messages;
JSON/spawn suffix rules check prefixes and exit status; a7 checks exit 1; a8
checks config/package parse-error prefixes and exit 1. e6 forwards installer
status (including Unix 128+signal); h1 preserves the full deterministic fake
transcript without relaxing message order. Listed
d1 filesystem codes remain exact; f1 checks `version_line()` bytes. No marker
waives unrelated fields. Prompt UX/signal/installer-only rows require their
separate lanes instead of granting a blanket exemption.

Native update fixtures (e1/e2/e3/e5 together) have explicit `rustUpdate` receipt
settings in the input. The bridge creates a protocol-1 install with a receipt, canonical `.version`,
versioned payload, executable launcher, and bundled script fixtures. It includes
that state in the expected filesystem snapshot and predicts only the injected
installer's declared writes. `rustUpdate.disposition` chooses installed,
unchanged, or repaired; unchanged makes no writes, and repairs restore declared
missing metadata/scripts while reusing identical payloads. The adapter forwards those settings and
compiled version through the real frozen ports. Assertions require one embedded
installer-port call, no separate latest lookup, the exact persisted settings,
compiled-version/latest progress, status/summary, and unchanged cleanup checks.
The wrapper implements `run_with_output`: it forwards and returns the exact §6
terminal line and location/guidance suffix, then D3 checks the committed state
before writing its §7.4 summary. Installer errors model an unexpected spawn
failure as exit 2 with `update failed: could not run installer: ...`; nonzero
installer statuses are forwarded under e6. The injected PATH already includes
the private launcher directory, so Unix emits the no-profile-change guidance.
Windows emits its required three guidance lines without writing User PATH.
A TS `fetchError` has no throwing equivalent in frozen `ReleaseSource -> Option`;
the fake records any lookup without throwing; replacement assertions reject
that call because the embedded installer owns resolution. No support or production file was changed.

## Result shapes and defaults

The following are exact successful `result` shapes, with object keys in the shown
order. Error results stay null. Absent optional properties are omitted, not null.
`<env>` denotes a string-valued object in insertion order.

| Kind | `result` |
| --- | --- |
| parseArgs | `{command?, positional:string[], flags:{[name]:string|true}}` |
| formatProgressEvent | string; no appended newline |
| cli | null (exit code/streams/prompts carry the result) |
| blackbox | null (no factory/update introspection) |
| config | `{targets:[target], project?, clients?, environments?, tags?}`; target `{name,output,groups?}`, project `{id?,rootMarkers?:[{path?,packageJsonName?}]}`, tags `{base?,project?,env?,client?,target?,group?,layer?}` |
| tags | `{tags:{base:string[],project?,env?,client?,target?,group?,layer?},candidates:string[]}` |
| path | resolved absolute output string |
| classify | `"shared"|"group"|"target"|"client"|"client-target"|null` |
| inspect | `[{target,outputPath:string,parentExists:boolean}]` |
| switch | `[{target,outputPath,written:boolean,keyCount:number,layers:[{name,rank,item:{id,title,tags},values:<env>}],overwrittenKeys:string[]}]` |
| dotenv | serialized string, or null when `write:true` |
| op | auth→null; vaults→`[{id,name}]`; items→`[{id,title,tags}]`; config→untrimmed string; env→`<env>` |

Defaults: root/cwd ROOT, vault alpha, config-item cfg, env local, client alpha,
target all, switch dryRun false, log enabled, formatter color supplied explicitly,
TTY flags false. Tags params default `{}` but their candidate query env defaults
to local. `op.items` tags default `[]`; `op.config` item defaults cfg; `op.env`
item defaults `{id:"alpha",title:"alpha"}`. Dotenv output defaults `nested/.env`,
write defaults false, operation umask defaults 022. An absent config means the
single api target/groups shown above; explicit null does not default. An absent
update latest is undefined, installer code defaults 0, and `.version` is rewritten
only when TS `updatedVersion` is supplied. Native replacement setup/installer
rules are explicit above. Prompt scripts may accept an existing default when
exhausted; no answer plus no default and unused answers are harness faults.

The in-process transport returns configured stdout (UTF-8 decoding replaces bad
bytes); successful stderr is ignored. A configured nonzero exit throws exactly
`Error(stderr.trim() || "Command failed: op <args joined by space>")`, exit 2.
This intentionally models the runner seam; only blackbox exercises real spawn,
raw bytes, stdout-cap enforcement and the runtime-specific empty-stderr suffix.
`transcript.schema.json` has per-kind `$defs` and conditional result validation;
the self-check validates **every committed OS golden** against it.

The a8 corpus covers overflow (`1e400`), total nesting depth 129, and lone UTF-16
surrogates in config item JSON, through both parseConfig and CLI config loading.
No i1 TS scenario is generated: that source case hangs; C1 owns its Rust-only test.

For native **update** parity on Linux, build/test for the corresponding packaged
musl target (`aarch64-unknown-linux-musl` or `x86_64-unknown-linux-musl`), for example:

```sh
cargo build --locked --workspace --bins --target aarch64-unknown-linux-musl
LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 cargo test --locked --test parity --target aarch64-unknown-linux-musl -- --ignored --nocapture
```

CI does the same: the Linux Rust lane runs with `--target $(uname -m)-unknown-linux-musl`
and `musl-gcc` as the linker. Under any non-packaged target, receipt-backed update
scenarios fail with a hint naming the target.
`parity/linux.Dockerfile` installs the host's musl target and linker. A GNU debug
target is not one of the protocol's five supported receipt targets. The fake-op
builder uses its own target directory even when a caller exports CARGO_TARGET_DIR.
