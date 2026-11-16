# envswitch parity harness (F1, TS phase)

The frozen oracle is `53de029a25a717c8ac824ca8553035e2b0fec991`, Bun **1.4.0**,
ICU **78.1**, Unicode **17.0**, default locale **en-US**. The corpus contains **463** scenarios (454 on macOS/Linux; 462 on Windows).
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
full SHA, `bun install --frozen-lockfile`, and writes `oracle-metadata.json`.
`parity/oracle/materialize.sh` delegates to the same cross-platform Bun script.
The materializer's package cache and HOME/profile roots live inside that temporary
export. Generation creates/removes its own export unless `ENVSWITCH_ORACLE_DIR`
points to an already materialized export. Use that variable to reuse an export;
never point it at the moving repository checkout.

`generate.ts` builds `crates/fake-op` in a temporary copy with a temporary
`[workspace]` table, then runs its integration test, rustfmt and clippy. It does
not create a workspace manifest or modify the shared Cargo lockfile. Rustup's
installed toolchain is located read-only; Cargo caches, HOME and profile paths for
the actual build are isolated in the temporary build. Rust 1.100.0, rustfmt and
clippy must already be installed. Linux/Windows CI generates the baseline on that
OS and immediately reproduces it; macOS CI also checks the committed baseline.
There is a deliberately explicit Rust-adapter placeholder in the workflow.

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
`macos`, `linux`, `windows`. Optional `divergence` names exactly one approved row
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
  a golden. `stdoutRepeat: {value,count}` is a compact **input-only** expansion
  used for the 50 MiB maxBuffer case. `candidateTags: string[]` declares exact
  comma-joined candidate queries for nonstandard templates/envs; standard local,
  dev and demo-suite queries are supplied by default. Fixture matching never
  implements classification: every returned candidate is rechecked by the oracle.
- `prompts`: ordered answers across select and input calls. A string/boolean
  answers by value; `{index:N}` selects a choice; `{default:true}` accepts the
  supplied default. Exhausted answers also accept a supplied default, otherwise
  fail the harness. `{cancel:true}` throws `ExitPromptError`; `{error:{name,message}}`
  throws a different scripted error. Unused answers are a harness failure.
  Exactly one choice auto-selects in the oracle and consumes **no** scripted answer.
- `parallelScans: N` holds config-tag list responses behind a bounded barrier
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
{schemaVersion:1, id, kind, divergence:string|null,
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
binary's handshake **before every CLI child**. A wrong token, stderr, nonzero
status, missing trace, missing fake or unknown invocation aborts the harness.
The CLI then uses that PATH to exercise real process spawning. Both successful
and failed command outputs, stderr trimming, secret stdout exclusion and the
50 MiB stdout limit are covered.

The diff runner prints field paths and decodes stdout/stderr for readable diffs;
any mismatch exits nonzero. Approved divergence markers are reported separately,
including when the TS baseline matches. They are **not blanket waivers**: the
later Rust adapter must explicitly validate each row's replacement assertion.
No arbitrary diff may be discarded merely because a scenario has a marker.

## Coverage and source-vs-contract notes

- 01 §9 rows 1–6: `parseArgs-*`, `formatProgressEvent-*`, flag/help CLI cases,
  including empty-next-token errata and JS object-key quirks (a3/a4).
- 01 §9 rows 7–15: `cli-guided-*`, including single-choice auto-selection,
  defaults, input fallback, all prompts/labels, factory reuse, parallel barrier,
  skipped unreadable/invalid vaults, cancellation and non-TTY guards.
- 01 §9 rows 16–23: `cli-update-*` plus `cli-unknown-update`. Original TS outcomes
  remain recorded with e1/e2/e3; receipt, compiled-version and embedded-installer
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
  `op-lone-surrogate-value` records this unapproved portability edge (02 §5.3).
- Rows 23–25: all 22 dotenv table entries plus empty map, key sort/errors, U+2029,
  create/replace/serialization-before-mkdir, modes/umask, rename cleanup and file
  symlink replacement on Unix. Expanded collation writes `WINNER` bytes for
  accents, canonical equivalence, punctuation, case, numeric, non-Latin and API ties.

The corrected empty-token and four-line-terminator contracts agree with source.
Two remaining textual inconsistencies must not guide the Rust implementation:
02 §2.3's suggested rejection of Windows `C:x` contradicts TS and the explicit
approved **parity** decision; Windows-only scenarios retain Node behavior.
DIVERGENCES c2 describes TS list wraparound, whereas `createInquirerPrompt`
actually passes `loop:false` (01 §5.4 agrees with source). Prompt navigation is
outside this scripted lane; c2 remains a PTY smoke/replacement responsibility.
No production behavior was changed to resolve either wording issue.

## Rust adapter handoff

Implement every kind above against the frozen crate API, with equivalent fake
runner, factory, prompt and update ports. Reuse these inputs, transport fixtures,
normalization rules and transcript schema; preserve result key presence, ordered
prompts/progress/argv and read counts. Create the same setup/filesystem snapshots,
including mode/cleanup/error checks, and translate generic prompt values to the
crate's choice indices without losing labels/defaults. Use the real native CLI in
the shared black-box runner. Add row-specific replacement assertions for approved
divergences (including `--version` and update); keep en-US collation. A Rust JSON
parser rejecting lone surrogates is **not** currently an approved divergence.
No Rust adapter, receipt model, prompt renderer or new production abstraction is
included in this phase.
