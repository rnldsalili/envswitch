# Installer and update protocol, version 1

Status: D2a specification for orchestrator freeze. Applies to D1 packaging, D2b's two installers, D3's native updater, and M1's TypeScript bridge. **Light migration** is the approved policy: no compatibility source archive on native releases and no forwarding stub. This document specializes [PLAN.md](PLAN.md) §4–5 and §10.1 and [DIVERGENCES.md](DIVERGENCES.md) e1–e5/f1. It supersedes the proposed full-migration portions of research 03 §B6. No implementation or publication is performed by this document.

MUST, MUST NOT, and SHOULD are normative. `<ver>` excludes `v`; `<tag>` includes it. Paths in messages are effective absolute paths, rendered in the platform's separators. Every listed message ends with one newline; no color is emitted by installers. Angle-bracket placeholders are not literal. Archive and receipt paths use case-sensitive comparisons on Unix and ordinal case-insensitive comparisons on Windows, after normalization.

## 1. Releases, resolution, and integrity

### 1.1 One publisher and one release identity

`.github/workflows/release.yml` alone allocates `0.0.${GITHUB_RUN_NUMBER}` and `v0.0.${GITHUB_RUN_NUMBER}`. A reusable build workflow receives that version and SHA; it does not allocate another version. Transition publishing requires manual dispatch with explicit publication enabled. Publishing checks the complete asset set in draft before promotion and rejects a tag pointing at another SHA or promotion behind a newer published run number. Gaps in run numbers are valid.

The supported version grammar is `0.0.N`, where N is `0` or a positive decimal integer without leading zeros. Zero is useful for local fixtures/development; published run numbers are positive. A pin or current-version input accepts exactly this form or `v0.0.N`, without surrounding whitespace. Normalize to `<tag> = v<ver>`. A release response must use the canonical tag, exactly `v0.0.N`. Other SemVer versions, prereleases, branch names, URLs, slashes, whitespace, and a lone `v` are rejected. No installer computes a version from a local package manifest.

With no pin, request `GET /repos/<repo>/releases/latest`. With a pin, request `GET /repos/<repo>/releases/tags/<tag>`. Retain that response's ID, canonical tag, and assets for the entire invocation. Never resolve latest again, use `latest/download` for assets, or fall back from a missing pin to latest. Reject draft/prerelease responses and a pinned response with a different tag. Equality is normalized tag equality; installers do not impose numeric upgrade ordering. An explicit pin can reinstall an older published version. Publication's latest guard, rather than an installer anti-rollback policy, prevents accidental latest regression.

### 1.2 Exact assets and kind selection

| Kind | Required release asset basenames |
| --- | --- |
| `source` | `envswitch_<ver>_source.tar.gz`, `install.sh`, `install.ps1`, `SHA256SUMS` |
| `native` | `envswitch_<ver>_aarch64-apple-darwin.tar.gz`, `envswitch_<ver>_x86_64-apple-darwin.tar.gz`, `envswitch_<ver>_x86_64-unknown-linux-musl.tar.gz`, `envswitch_<ver>_aarch64-unknown-linux-musl.tar.gz`, `envswitch_<ver>_x86_64-pc-windows-msvc.zip`, `install.sh`, `install.ps1`, `SHA256SUMS` |

Inspect the asset list, not an attempted download's failure. Presence of **any** of the five exact native basenames selects `native`; all five must then be present exactly once, and a source asset must be absent. Otherwise the exact source basename selects `source`. Neither kind, a partial native set, duplicate required names, or a mixture is an invalid release. After selecting an otherwise valid kind, a missing `SHA256SUMS` uses the specific missing-checksum error before the generic missing-script/asset error. Do not silently install source after a native download, checksum, extraction, target, or signature failure. Unrelated assets do not affect selection. Native releases never publish `envswitch_<ver>_source.tar.gz` in Light mode.

The standalone scripts are byte-identical to the scripts bundled in the corresponding archive. A native archive has **no enclosing directory** and contains exactly:

```text
envswitch                         # Unix only; regular executable file
envswitch.exe                     # Windows only, instead of envswitch
scripts/install.sh
scripts/install.ps1
README.md
```

Directory entries for `scripts/` are allowed. No other entries are permitted by protocol 1; adding a license or another packaged file requires a coordinated allowlist change. Installers do not copy the packaged README into a project.

Source archives keep the existing `envswitch/` top directory and the current release workflow's contents: `package.json`, `bun.lock`, `README.md`, `tsconfig.json`, `src/**`, and `scripts/**`. Required regular files are `envswitch/package.json`, `envswitch/bun.lock`, `envswitch/src/cli.ts`, and both `envswitch/scripts/install.*`. The package name must be `envswitch`. No `.git`, `node_modules`, `.env`, `.env.*`, receipt, version marker, executable special file, or path outside that allowlist is accepted. The source installer writes its own marker and receipt and installs production dependencies with Bun in staging.

Before extraction, list and validate **all** members, including type and link metadata. Reject absolute paths, Windows drive/UNC forms, backslashes in member names, empty/interior `.` or `..` components, control characters, duplicate normalized paths, case collisions on Windows, hard links, symbolic links, devices, FIFOs, and reparse-point equivalents. An optional leading `./` on tar names may be removed before validation. Only regular files and necessary directories are allowed. Extraction must not restore archive ownership, privileged modes, or traverse an existing link. Extract into a newly created private empty directory. Validate the resulting tree again before execution; a name-only `tar -t` check is insufficient to validate link types.

### 1.3 SHA-256 is mandatory

Download `SHA256SUMS` and the selected archive from the **same retained release**. The manifest is UTF-8/ASCII, no BOM, LF lines of exactly `64 lowercase hex digits`, two ASCII spaces, and a basename. A final LF is required. Reject malformed lines and duplicate filenames anywhere in the manifest. It covers all required archives and both standalone installers, excludes itself, and must contain exactly one selected archive entry and one entry for each script. SHA-256 mismatches, missing entries, missing manifest, unavailable hashing tools, and malformed manifests are fatal before extraction or execution. Use `sha256sum`, else `shasum -a 256`, or PowerShell `Get-FileHash -Algorithm SHA256`; normalize computed hex to lowercase. No skip-checksum option exists.

After verifying the archive and extracting it, compare the bundled installer files' hashes to their entries in the same manifest. The executing bootstrap/embedded installer is already trusted input; do not replace/re-execute it from the network mid-transaction. Only verified bundled scripts are installed into `current/scripts`.

**Historical-source gate:** today's source releases have no manifest. Mandatory verification means a dual-mode installer cannot consume those unchanged releases. Before exposing the dual-mode scripts at the mutable README URLs, D1/orchestrator must arrange a source latest with this complete asset/manifest contract: either promote the bridge first using an approved staged rollout, or add verified checksum/script assets to the historical source release before the URL switch. This is a release-sequencing prerequisite, not a checksum exception. If the prerequisite is missed, fail closed with the exact missing-checksum error in §6. Existing old installers remain unverified; their old → bridge behavior is not retroactively secured.

### 1.4 Transport and optional attestation

Prefer authenticated `gh` for release metadata and assets, with HTTP fallback on failure on both platforms. Use `gh api repos/<repo>/releases/...` for metadata and `gh release download <tag> --repo <repo> --pattern <exact-name> --dir <private-dir>` for a file. After fallback, consume the saved response's exact asset API `.url` with `Accept: application/octet-stream`; metadata uses `Accept: application/vnd.github+json`. JSON must be parsed structurally, including escaped strings and arbitrary field order: a single `sed` tag extractor does not satisfy this contract. Do not add a mandatory third-party JSON installation as part of installation. D2b must test its chosen Bash parser/available-tool strategy with `gh` absent and malformed responses.

`GITHUB_TOKEN`, if nonempty, supplies a bearer header only to the trusted GitHub API origin. `gh` retains its normal authentication behavior. Download redirects may go to GitHub's HTTPS asset storage; never forward the bearer token to a different origin. Production API asset URLs must belong to `https://api.github.com/repos/<repo>/releases/assets/<decimal-id>`. Validate API fields and URLs before use. Never log headers, tokens, or signed redirect query strings. Bound each request to 60 seconds and at most three attempts total; retry only transport failures, 408, 429, and 5xx. Do not retry authentication, 404, parsing, or integrity failures as another release/kind.

Attestations are **ON at publication**, produced in the native build jobs for their archives; the source packaging job attests its source archive. Local verification is opt-in via §2. When requested, require `gh` and run `gh attestation verify <absolute-archive> --repo <repo>` after SHA verification and before extraction, requiring exit zero. An absent verifier or failed verification is fatal; never print a verified line on failure. Default installation does not call attestation verification. Provenance verification is bound to the configured repository by that command; protocol 1 does not promise a separately pinned signer/ref policy or independent signing key.

## 2. Installer inputs and destination validation

### 2.1 Supported public inputs

Unix entry point is `bash install.sh`, with no positional arguments. PowerShell supports Windows PowerShell 5.1+ and retains `[CmdletBinding(SupportsShouldProcess = $true)]`.

| Unix environment | PowerShell parameter | Meaning/default |
| --- | --- | --- |
| `REPO` | `-Repo <owner/repo>` | `rnldsalili/envswitch`; Unix unset/empty uses default. PS omitted uses default; explicitly empty is invalid. Grammar `^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$`, neither component `.` nor `..`. |
| `INSTALL_DIR` | `-InstallDir <absolute-path>` | Unix unset/empty: `$HOME/.local/bin`. PS omitted/empty: `<effective AppRoot>\bin`. |
| `APP_ROOT` | `-AppRoot <absolute-path>` | Unix unset/empty: `${XDG_DATA_HOME:-$HOME/.local/share}/envswitch`. PS omitted/empty: `%LOCALAPPDATA%\Programs\envswitch`. |
| `XDG_DATA_HOME` | — | Unix app-root default only; macOS uses the same XDG-style default. |
| `GITHUB_TOKEN` | shared environment | Optional HTTP authentication. Never persisted. |
| `ENVSWITCH_VERSION` | `-Version <version-or-tag>` | Unset/empty means latest. Otherwise §1.1 grammar; normalized tag pin. |
| `ENVSWITCH_CURRENT_VERSION` | `-CurrentVersion <version-or-tag>` | Unset/empty means unknown/fresh invocation. Otherwise §1.1 grammar; caller's running version, **not** a claim that files are healthy. |
| `ENVSWITCH_VERIFY_ATTESTATIONS` | `-VerifyAttestations` | Unix unset/empty/`0`: off; `1`: on; other values invalid. PS switch defaults off. |
| — | `-WhatIf` | Validate local inputs/platform and print §6.4; no side effects, network, subprocess verification, or lock. |

PowerShell does **not** read `REPO`, `INSTALL_DIR`, `APP_ROOT`, `ENVSWITCH_VERSION`, `ENVSWITCH_CURRENT_VERSION`, or `ENVSWITCH_VERIFY_ATTESTATIONS` as parameter substitutes. Updaters pass explicit parameters. `HOME`, `SHELL`, `PATH`, `LOCALAPPDATA`, `USERPROFILE`, `TMPDIR`/`TEMP`/`TMP`, and source-only `BUN_INSTALL` retain their platform roles. Explicit `-AppRoot` makes `LOCALAPPDATA` unnecessary, including when `-InstallDir` defaults from it. Unknown Unix arguments/PS parameters are rejected before work. `-Confirm` remains a common parameter: one transaction-level `ShouldProcess` approval precedes **all** mutations; refusal has no mutation and prints no success.

Normalize directory trailing separators and lexical components before persisting; preserve `/` or a drive-root only to reject it. Unix must be absolute and continues to reject the substring `..` anywhere, matching the old validator. Reject control characters and `:` in Unix destination paths (PATH cannot represent those entries). Windows accepts fully qualified local drive paths, rejects relative/drive-relative paths, UNC/device paths, `.`/`..` components, alternate data streams, quotes, control characters, and trailing-dot/space aliases. Reject filesystem roots and selecting a home/profile directory itself as app root. Unicode, spaces, apostrophes, shell metacharacters, and Windows `%`, `!`, `&` in otherwise valid directory names must be safely represented, not evaluated.

`current` is a real directory. Check existing path components for symlinks/reparse points; reject traversal through them for app root, launcher directory, current, payload, receipt, lock, and staging. A user may specify a resolved real path instead. Reject an install directory inside `current` or installer-private paths, an app root equal to the install directory, or a launcher path that collides with app root/data. `<AppRoot>\bin` on Windows is allowed. Do not recursively treat the install directory as owned; it may be a shared bin directory. Verify existing destinations and access before downloads where possible, and recheck before renames/cleanup. Do not elevate privileges.

### 2.2 Unsupported test hooks

These are not user configuration, mirrors, or security bypasses. Every new hook is ignored unless `ENVSWITCH_INSTALL_TEST_MODE` is nonempty; that mode must be explicitly selected by the suite. Invalid mode/value is an error, never a production fallback.

| Hook | Test-only contract |
| --- | --- |
| `ENVSWITCH_INSTALL_TEST_MODE=install` | Full isolated installer transaction with enabled hooks. |
| `ENVSWITCH_INSTALL_TEST_MODE=ensure-profile-path` | Unix helper only: exactly two arguments, install directory and profile path; print helper lines and return before release work. |
| `ENVSWITCH_INSTALL_TEST_MODE=shell-profile-path` | Unix helper only: no arguments; print selected profile path and return. |
| `ENVSWITCH_INSTALL_TEST_API_BASE_URL` | In `install` mode, replace `https://api.github.com` with a loopback HTTP origin (`127.0.0.1`/`localhost`/`[::1]` plus optional port). Preserve `/repos/...` paths. Asset URLs must use that same test origin. No arbitrary external origin. Test mode considers `gh` only when its resolved executable is a suite-owned fake beneath isolated HOME/USERPROFILE; a host `gh` is treated as absent. This permits fake-gh fallback tests without invoking authenticated host tools. |
| `ENVSWITCH_INSTALL_TEST_USER_PATH_FILE` | Windows `install` mode: required absolute file under isolated USERPROFILE; replaces **both** persistent User PATH read and write with UTF-8 file read/atomic write. Missing file means empty User PATH. Never call the registry/User environment APIs in this mode; fail if the hook is missing. Process PATH remains isolated and may be updated. |
| `ENVSWITCH_INSTALL_TEST_FAIL_AT` | `after-payload`, `after-launcher`, or `after-metadata`: inject a controlled failure at that transaction boundary to test rollback. Never bypass checks. |

For existing Bash helper tests only, keep `ENVS_SWITCH_INSTALL_TEST_MODE=ensure-profile-path|shell-profile-path` as a deprecated alias; reject simultaneous use of the old and new variable. The old variable cannot enable URL/PATH/failure hooks. New suites use the namespaced spelling. Fake `uname`, `sysctl`, `bun`, `gh`, download tools, and filesystem failures are supplied by an isolated test PATH, not public target/skip-validation options.

## 3. Platform and payload verification

Unix accepts `uname -s` Darwin or Linux (WSL is Linux). Map `uname -m` `aarch64|arm64` to aarch64 and `x86_64|amd64` to x86_64. On Darwin, `sysctl -n hw.optional.arm64` returning `1` selects aarch64 even under Rosetta; otherwise use the architecture mapping. An unavailable/nonmatching capability query alone is not proof of ARM. Darwin maps to `*-apple-darwin`; Linux always maps to `*-unknown-linux-musl`. No libc selection. Reject other CPUs/OSes before downloads.

PowerShell requires Windows and uses `RuntimeInformation.OSArchitecture`, not process architecture: `X64` selects `x86_64-pc-windows-msvc` even from 32-bit PowerShell. Reject Arm64, X86, and unknown OS architectures; protocol 1 does not rely on Windows ARM emulation. Source installs apply the same supported platform checks so the bridge has a defined future native target.

For native installation, first verify hashes and archive structure, then stage the executable under its final versioned basename on the destination filesystem with Unix mode `0755`. Run it by absolute path:

1. `--version`: exit 0, empty stderr, and **exactly** `envswitch <ver> <selected-target>\n` on stdout. No leading `v`, extra fields, CR, or extra lines. Example: `envswitch 0.0.42 aarch64-apple-darwin\n`.
2. `--help`: exit 0; suppress captured help from normal installer output. It must not access `op`, sign in, or write project files.

Capture bytes for the version check rather than trimming output (shell command substitution strips LF). Apply both checks again through the final absolute launcher before declaring success. Never invoke the command by a potentially shadowed PATH name. A source payload has no `--version` contract: validate source shape, require successful `bun --version` and `bun install --production --frozen-lockfile` in staging, then test its launcher with `--help`. Only the source branch discovers/bootstraps Bun; native installation never searches for or runs Bun.

## 4. Installed files and receipt

```text
APP_ROOT/
  .install.lock                         # exclusive lock file while running
  current/                              # real directory, never symlink
    .version                            # UTF-8 without BOM: v<ver> + LF
    install.json                        # UTF-8 JSON without BOM, final LF
    scripts/install.sh
    scripts/install.ps1
    bin/envswitch_<ver>_<target>[.exe]   # native payloads, possibly older ones too
    package.json, bun.lock, ...          # source only; see residual-source policy below
    src/cli.ts, node_modules/...         # source only
INSTALL_DIR/envswitch                   # Unix launcher, 0755
INSTALL_DIR/envswitch.cmd               # Windows launcher; no competing envswitch.exe
```

Source `payloadPath` is `APP_ROOT/current/src/cli.ts`; native `payloadPath` is the versioned binary path above. Both kinds bundle both scripts. Existing pre-bridge markers may have CRLF/BOM from PowerShell; readers may tolerate that legacy encoding and trim a single surrounding newline when recovering a source version. Every new writer uses canonical UTF-8/LF. Native update's running version is compiled in and never derived from `.version`.

### 4.1 Receipt schema and acceptance

Example native receipt (JSON property order is not significant):

```json
{
  "schemaVersion": 1,
  "kind": "native",
  "repo": "rnldsalili/envswitch",
  "appRoot": "/home/demo/.local/share/envswitch",
  "installDir": "/home/demo/.local/bin",
  "launcherPath": "/home/demo/.local/bin/envswitch",
  "target": "x86_64-unknown-linux-musl",
  "tag": "v0.0.42",
  "payloadPath": "/home/demo/.local/share/envswitch/current/bin/envswitch_0.0.42_x86_64-unknown-linux-musl"
}
```

All nine fields are required; reject duplicate keys. `schemaVersion` is the integer 1, `kind` is `source` or `native`, `repo` and `tag` obey §1–2. All path strings are effective normalized absolute paths satisfying §2, with no NUL/control characters. `launcherPath` must equal `installDir` plus the platform launcher basename. `payloadPath` must equal the exact path derived from app root, kind, tag, and target; a mere string prefix check does not prove containment. Source uses `target: null`; native uses exactly one of the five triples and the platform's executable suffix. Unknown fields may be ignored but must not authorize additional writes/deletions. Never store a token, account data, project path, or secret.

For D3, derive a candidate root only from canonical `current_exe`: require `.../current/bin/envswitch_<running-ver>_<running-target>[.exe]`, then inspect that root's `current/install.json`. The receipt must point back to that root. A healthy current receipt must identify the running executable. A receipt for another tag in the **same** root may describe an update installed while the old process remains running; validate its own exact paths and use it only as that installation's settings. Do not force its payload to match the old process or overwrite a newer marker from the old compiled version. The installer resolves/validates what is actually installed under its lock. Never search the invoking project/CWD, arbitrary parent directories, or PATH for a receipt.

Missing or invalid receipt on a copied native binary is a controlled reinstall-guidance error (§7); do not guess defaults or reconstruct it from environment overrides. A structurally valid receipt with a missing payload can be repaired by an installer. Missing receipt repair is supported by a **direct reinstall** with explicit/default destinations, after positive ownership checks; D3 itself requires a receipt to know those destinations. An invalid receipt never grants cleanup authority: a direct installer also fails on it rather than trusting or deleting it.

### 4.2 Launcher contents (protocol v1, amendment 1)

Unix native launcher, LF, replacing `<Q(payloadPath)>` with a single-quoted Bash literal (escape each embedded apostrophe as `'\''`):

```bash
#!/usr/bin/env bash
set -euo pipefail
exec <Q(payloadPath)> "$@"
```

Unix source keeps the current lookup behavior, with only the CLI path safely quoted:

```bash
#!/usr/bin/env bash
set -euo pipefail
if command -v bun >/dev/null 2>&1; then
  BUN_BIN="$(command -v bun)"
elif [[ -x "${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\n' >&2
  exit 127
fi
exec "$BUN_BIN" <Q(payloadPath)> "$@"
```

Windows launchers are UTF-8 without BOM, CRLF. The ASCII prefix changes the code page before reading a Unicode payload line, then restores it before exit. `<CMD(path)>` is the literal absolute path with `%` doubled; double quotes/control characters are already rejected and delayed expansion stays disabled. No `call` is used on the native payload or user arguments. Native contents:

```bat
@echo off
setlocal DisableDelayedExpansion
for /f "tokens=2 delims=:." %%C in ('chcp') do set "_ENVSWITCH_CP=%%C"
chcp 65001 >nul
"<CMD(payloadPath)>" %*
set "_ENVSWITCH_EXIT=%ERRORLEVEL%"
chcp %_ENVSWITCH_CP% >nul 2>&1
exit /b %_ENVSWITCH_EXIT%
```

Source contents use the same first four lines, followed by:

```bat
where bun >NUL 2>NUL
if %ERRORLEVEL% EQU 0 goto envswitch_bun_path
if exist "%USERPROFILE%\.bun\bin\bun.exe" goto envswitch_bun_home
echo envswitch: bun is not installed or not on PATH 1>&2
set "_ENVSWITCH_EXIT=127"
goto envswitch_finish
:envswitch_bun_path
bun "<CMD(payloadPath)>" %*
set "_ENVSWITCH_EXIT=%ERRORLEVEL%"
goto envswitch_finish
:envswitch_bun_home
"%USERPROFILE%\.bun\bin\bun.exe" "<CMD(payloadPath)>" %*
set "_ENVSWITCH_EXIT=%ERRORLEVEL%"
:envswitch_finish
chcp %_ENVSWITCH_CP% >nul 2>&1
exit /b %_ENVSWITCH_EXIT%
```

These are batch launchers, so callers still apply normal cmd/PowerShell argument quoting. D2b tests spaces, Unicode, `%`, `!`, `&` in configured paths and arguments plus nonzero child statuses. Do not reuse the old parenthesized Bun blocks, whose `%ERRORLEVEL%` expansion can lose the child status.

## 5. Transaction, health, rollback, and cleanup

1. Validate inputs, OS/CPU, destination relationships, preexisting ownership, and required tools. `-WhatIf` stops here with §6.4 without requiring download/hash/Bun/attestation tools. A source checkout (`.git` file/directory in the selected app tree) is never an installation destination. An existing unrelated launcher must cause an error, not be overwritten. Recognize a launcher by valid receipt plus matching content, the historical envswitch launcher shape whose literal source path matches this root, or the exact protocol-1 native launcher shape whose versioned payload path belongs to this root. The last case permits a direct reinstall to repair a missing native receipt. Explicit reinstall is not permission to replace an unrelated executable.
2. Create app root if needed and **exclusively create** `APP_ROOT/.install.lock` (Unix noclobber/exclusive create; Windows `FileMode.CreateNew`). It contains this invocation's random ID and PID, no secrets; private permissions. Do not follow a lock link or overwrite an existing lock. A contending installer fails immediately. No time-based automatic stale-lock deletion. On all controlled exits remove only the lock whose ownership token matches this invocation. After a crash, the user may remove a positively identified stale lock after confirming no installer is running; §6 supplies the guidance.
3. Resolve the selected release exactly once, select kind, preflight kind-specific tools, and download/hash-check into private storage. Use a private app-root stage (`.install-stage-<random>`, Unix directory mode `0700`, files `0600` until setting executable mode; Windows owner-only access plus required system/administrator access). All files to be renamed finally are staged on the destination filesystem. The launcher temporary file is beside the destination launcher and must be exclusively created. Never stage over an existing arbitrary path.
4. Extract safely and perform §3 checks. Source alone may bootstrap Bun as today, **after** successful release/integrity checks, and run locked production dependency installation in staging. Bun/bootstrap side effects are outside the envswitch rollback boundary; never uninstall Bun on failure. Copy the executing installer outside `current` before any commit step (D3/M1 already do this).
5. Under the lock, assess health using actual receipt, canonical `.version`, launcher content, both bundled scripts, and payload. For native, compare the installed executable and scripts to bytes from the verified staged archive and run absolute-path version/help checks. For source, require matching tag, valid source receipt/tree/scripts, Bun and successful absolute launcher `--help`; do not use `--version`. A missing receipt/marker/script/launcher/payload is unhealthy, even when version strings match. `CurrentVersion` is only the caller's old-version label. It never shortcuts these checks.
6. If healthy and already the selected tag/kind/target, return `unchanged` with §6's terminal line. Downloads/verification may have occurred; no payload, marker, launcher, receipt, profile, or persistent PATH writes occur. A missing receipt on a positively identified historical source installation is repair work, not unchanged. A same-tag unhealthy install returns `repaired` only after successfully completing the transaction. A different tag/kind or fresh install returns `installed`. Determine same-tag repair from valid existing receipt/marker, or `CurrentVersion` when neither survives.
7. Native: add the staged file to `current/bin/envswitch_<ver>_<target>[.exe]`; never truncate an existing payload. An existing file with identical verified bytes can be reused; conflicting bytes at that immutable path fail, with recovery guidance. This includes a corrupt same-version existing executable: automatic repair covers missing files, not overwriting a possibly running Windows executable. Keep all previous versioned payloads. Source: back up each positively identified source entry (`src`, `node_modules`, `package.json`, `bun.lock`, `README.md`, `tsconfig.json`) before replacing it from staging; replace scripts/metadata in step 8. Keep `current` itself and `current/bin` in place, including during a native → source pinned install. Never merge dependencies into an old tree or recursively delete an unrecognized `current` tree. Build a temporary source-check launcher pointing to the staged CLI for the precommit `--help` check, then build the final launcher pointing to `current/src/cli.ts`.
8. Back up the old launcher bytes/mode and old scripts/receipt/marker. Prepare complete launcher and metadata replacements before switching. Replace the launcher by a same-directory atomic replace/rename when supported; never remove/truncate the live launcher first. On Windows use a replacement operation that preserves the prior file on a sharing violation; report close/retry guidance. Create a missing launcher by same-directory rename. Execute the absolute final launcher checks. Then replace the bundled scripts, receipt, and marker individually with staged files; verify final metadata agrees with the checked payload. Record the `after-payload`, `after-launcher`, and `after-metadata` injection boundaries at their named steps.
9. Apply PATH policy only after launcher/metadata verification. Preserve a snapshot of the selected profile bytes/existence or Windows persistent User PATH; restore it if this invocation's PATH write fails. Unix selects `.zshrc` for basename `zsh`, `.bashrc` for `bash`, otherwise `.profile`. Preserve the exact marker block and surrounding user bytes; replace an existing complete managed block when its directory changed, append only when absent, fail rather than guess with multiple/incomplete blocks. Do not rewrite unmarked user exports. Windows compares persistent User PATH entries case-insensitively after trimming trailing `\`, appends if missing, and separately ensures this process PATH contains the directory. Do not edit Machine PATH or claim to modify the invoking terminal's environment.
10. Only after all required steps succeed print the terminal success line, location lines, and guidance. Release lock and clean invocation-private files on normal exit. Perform fallible required cleanup before printing success. Optional deletion of an inert, positively identified backup/stage file may be deferred with `info: deferred cleanup: <path>` before the terminal line; never turn a printed success into a later controlled failure. Do not print success before the help/version checks or a required PATH operation.

**Rollback guarantee:** on any controlled failure after mutation, restore prior managed launcher, source tree, scripts, marker, receipt, and this invocation's PATH changes; a previously working install remains usable. A newly placed verified native payload may remain if locked; it is inert unless a committed launcher references it. Remove newly created empty directories where safe; never remove preexisting parents. If rollback itself fails, emit both failure and rollback guidance and keep identifiable backup paths for manual recovery. Ordinary rollback must not delete a running or old payload. Windows sharing violations are failures, not success with incomplete cleanup.

There is no atomic commit spanning launcher, receipt, marker, and profiles. Crash/power loss/SIGKILL can leave recoverable mixed metadata, private staging/backup files, and a stale lock. The executable's compiled version is authoritative; a later explicit reinstall repairs metadata/launcher using this protocol. Do not claim atomic directory replacement on Windows or guaranteed crash durability/fsync. Catchable INT/TERM runs cleanup/rollback and returns `128 + signal`; Windows cancellation returns 130 when handled. A parent waiting on an installer must not prematurely delete its private script while it is running.

**Source → native cleanup (Light):** native commits replace the managed launcher, scripts, receipt, and marker. They add the versioned binary and stop invoking Bun. They do **not** install a compatibility stub. Keep legacy `src`, `node_modules`, package files, and other source content as unused residual files in `current`; no automatic recursive source-tree prune is required in protocol 1. This conservative choice preserves unknown user files and supports rollback. Unknown custom launchers may continue to run old TS and must be repaired by the explicit README reinstall using their actual directory; do not promise migration of undiscoverable launchers. Direct native fresh installs do not create these residual files.

Delete only this invocation's identified staging/download files and rollback copies on successful completion, or restore them on failure. Do not prune older binaries in this version of the protocol; a locked old EXE is harmless. A source reinstall must refuse replacement if the old tree contains unknown/unmanaged files, `.env`/`.env.*`, or links that cannot be safely preserved, and give manual-recovery guidance. **Never delete Bun, user projects, `.env` files, unrelated launchers, a shared bin directory, an arbitrary overridden root, or anything outside positively identified envswitch paths.** Native conversion is not an excuse for a broad `rm -rf current`.

For an immutable-payload conflict, manual recovery is: close envswitch processes, preserve/move aside **that exact conflicting payload file**, and rerun the README installer with the same destinations. For an invalid receipt, preserve/move aside the invalid receipt after checking its paths, then directly reinstall using a recognized launcher and explicit destinations. Installers do not automatically perform these destructive/ambiguous recovery actions.

## 6. Installer output and exit status

### 6.1 Streams and exit codes

Unix `info:`/`success:` go to stdout and `error:` to stderr. PowerShell uses explicit console stdout/stderr for the same contract; do not return strings as pipeline values or use `throw`/`Write-Error` for controlled diagnostics, which add host decoration. Host parameter-binding/engine errors before script entry are outside this message contract but must be nonzero. Tool errors can add stderr diagnostics; suppress token-bearing URLs/headers. Bun bootstrap/dependency output is tool-owned and source-only.

| Exit | Meaning |
| --- | --- |
| `0` | Installed, repaired, unchanged, successful helper, dry run, or declined `ShouldProcess`. Distinguish a mutating outcome by the terminal line below, not the exit code alone. |
| `1` | Controlled validation, release, integrity, lock, download, verification, commit, or rollback failure. No success line. |
| `128+n` | Installer killed by Unix signal n (130 for INT, 143 for TERM); cleanup when possible. Windows handled cancellation: 130. |

Exactly one terminal outcome line is emitted on an ordinary successful non-dry-run install:

```text
success: installed envswitch <tag>
success: repaired envswitch <tag>
success: envswitch is already up to date (<tag>)
```

Use only the matching line; unchanged prints no location/PATH guidance. Installer failure after success is forbidden. D3/M1 parse only these anchored terminal lines for the result, not arbitrary tool output, and validate the reported tag against committed metadata. A source dependency subprocess must not be able to impersonate the final outcome: only the final installer-owned terminal line followed by the defined location/guidance suffix is accepted, with the postcondition checks in §7.

### 6.2 Information and PATH lines

In execution order: Unix platform line, selected-release line, selected target if native, archive and manifest download lines, archive hash verification, optional attestation, kind-specific extraction, bundled-script hash checks, source-only Bun/dependency setup, payload/launcher verification, PATH helper changes, terminal result, location lines, and PATH guidance. Reused payloads may omit installation lines, but not required checks. Normal informational lines use final paths; only deferred-cleanup/error guidance may expose a private staging/backup path.

Preserved lines (the download helper applies to either `SHA256SUMS` or the archive):

```text
info: detected macOS
info: detected WSL/Linux
info: detected Linux
info: downloading <basename> with GitHub CLI
info: GitHub CLI download did not find <basename>; trying direct download
info: downloading <basename>
info: extracting source archive
info: Bun is not installed; installing Bun
info: using Bun at <bun_bin>
info: installing production dependencies
info: installing launcher to <launcherPath>
info: verifying envswitch launcher
info: source: <APP_ROOT>/current
info: launcher: <launcherPath>
```

The first three are alternatives and Unix-only. Bun/source lines occur only for source. `info: source:` is replaced by `info: binary:` for native. All platforms now print the installing/verifying launcher lines. New lines:

```text
info: resolved envswitch <tag> (<source|native>)
info: selected target: <target>
info: verifying SHA-256 for <basename>
info: verifying GitHub attestation for <basename>
info: extracting native archive
info: installing binary to <payloadPath>
info: binary: <payloadPath>
info: deferred cleanup: <path>
```

`verifying SHA-256` is emitted for the archive and the two bundled script basenames; it indicates a check starting, not proof of success. The attestation line is emitted only when requested. Final native version/help output is captured, not echoed.

Keep these PATH guidance bytes verbatim where applicable. Unix INSTALL_DIR already on PATH (verification line occurs before terminal success; guidance occurs after locations):

```text
info: run 'envswitch --help' to verify the installation
```

Unix INSTALL_DIR missing from process PATH: emit one helper-change line **before** success, then the three guidance lines **after** locations:

```text
info: PATH block already exists in <profile_path>
info: added <INSTALL_DIR> to PATH in <profile_path>
info: restart your terminal, or run this command now:
info: export PATH="<INSTALL_DIR>:$PATH"
info: then run 'envswitch --help' to verify the installation
```

The first two are alternatives. For replacing an old managed block use `info: updated envswitch PATH block in <profile_path>` instead. This is the only new profile message. The stored block still uses exactly `# >>> envswitch PATH >>>`, `export PATH="<escaped-INSTALL_DIR>:$PATH"`, and `# <<< envswitch PATH <<<` on separate lines; escape `\`, `"`, `$`, and backtick within the directory's double-quoted shell literal. Guidance renders the same safe command; ordinary paths preserve current text exactly.

Windows, added-User-PATH line only when persistent PATH changes, before success; remaining guidance always after locations:

```text
info: added <InstallDir> to the user PATH
info: restart your terminal, or run this command now:
info: $env:Path = "<InstallDir>;$env:Path"
info: then run 'envswitch --help' to verify the installation
```

Escape backtick, `"`, and `$` inside the directory in the PowerShell guidance command. The existing ordinary-path text and shell variable casing above remain unchanged.

### 6.3 Controlled errors

Each row is the complete installer-owned stderr sequence for that condition. Failure-specific external stderr may precede it; no stack trace is required. `<detail>` is a sanitized platform/tool diagnostic without credentials; the prefix is stable, while the detail is not golden-tested.

| Condition | Exact line(s), in order |
| --- | --- |
| Invalid repo, Unix | `error: invalid repository format: <repo>` then `error: expected owner/repo` |
| Invalid repo, Windows | `error: invalid repository format: <repo>. Expected owner/repo.` |
| Invalid Unix absolute/`..` rule | `error: invalid <INSTALL_DIR\|APP_ROOT>: <dir>` then `error: <INSTALL_DIR\|APP_ROOT> must be an absolute path without '..'` |
| Other unsafe destination | `error: unsafe installation path: <path>` then `error: choose absolute, non-overlapping envswitch paths without symlinks or reparse points` |
| Missing LOCALAPPDATA and no AppRoot | `error: LOCALAPPDATA is not set. Pass -InstallDir and -AppRoot explicitly.` |
| Invalid version/current | `error: invalid <Version\|CurrentVersion>: <value>; expected 0.0.N or v0.0.N` |
| Unknown Unix argument | `error: unexpected installer argument: <argument>` |
| Invalid attestation input/test hook | `error: invalid installer setting: <name>` |
| Unsupported Unix OS | `error: unsupported OS for this installer: <lowercase_uname>` then `error: use scripts/install.ps1 on Windows` |
| Non-Windows PowerShell | `error: unsupported OS for this installer. Use scripts/install.sh on macOS, Linux, or WSL.` |
| Unsupported CPU | `error: unsupported architecture for this installer: <architecture>` |
| Missing general tool | `error: required installer tool not found: <tool>` |
| No latest response | `error: no GitHub Release found for <repo>, or access is denied` |
| Missing pinned release | `error: GitHub Release not found or unavailable: <repo>@<tag>` |
| Invalid response/tag/kind/assets | `error: invalid envswitch release metadata for <repo>` |
| Download unavailable | `error: release asset not found or unavailable: <basename>` |
| No SHA256SUMS asset | `error: release <tag> is missing SHA256SUMS; refusing unverified installation` |
| Bad manifest/entry | `error: invalid SHA256SUMS for release <tag>` |
| Hash mismatch | `error: SHA-256 mismatch for <basename>` |
| Requested verifier missing | `error: gh is required when attestation verification is requested` |
| Attestation failure | `error: GitHub attestation verification failed for <basename>` |
| Unsafe/invalid archive | `error: unsafe or invalid archive: <basename>` |
| Missing source shape | `error: archive did not contain an envswitch source package` |
| Bun bootstrap tool absent | `error: curl is required to install Bun automatically` then `error: install Bun manually from https://bun.com/docs/installation, then rerun this installer` |
| Bun absent after bootstrap | `error: Bun installation finished, but bun was not found on PATH` |
| Bun/version/dependencies fail | `error: bun install failed` (dependency install), or `error: Bun verification failed` (`--version`) |
| Staged native version mismatch/nonzero | `error: native payload verification failed; expected envswitch <ver> <target>` |
| Help/final launcher check fails | `error: envswitch launcher verification failed: <launcherPath>` |
| Immutable payload conflict | `error: existing native payload differs from verified release: <payloadPath>` then `error: close envswitch processes and reinstall using the README instructions; preserve the existing file until recovery is complete` |
| Missing/not executable Unix launcher after write | `error: launcher was not installed or is not executable: <launcherPath>` |
| Missing Windows launcher after write | `error: launcher was not installed: <launcherPath>` |
| Bad receipt | `error: invalid envswitch install receipt: <receiptPath>` then `error: reinstall envswitch with the install script from the README after checking the install paths` |
| Unowned existing path/tree | `error: refusing to replace an unmanaged path: <path>` then `error: preserve user files and reinstall using the README instructions with explicit install paths` |
| Lock exists | `error: another envswitch installer may be running: <lockPath>` then `error: retry after it exits; remove the lock only after confirming no installer is running` |
| Windows sharing violation | `error: could not replace envswitch launcher: <launcherPath>` then `error: close processes using the launcher and retry; the previous installation was preserved` |
| Malformed/multiple profile blocks | `error: invalid envswitch PATH block in <profile_path>; repair the block and rerun the installer` |
| Other controlled IO/transaction failure | `error: envswitch installation failed: <detail>` |
| Injected test failure | `error: injected installer test failure: <boundary>` |
| Rollback failure (append to original error) | `error: rollback incomplete; preserve <backupPath> and reinstall using the README instructions` |

Obsolete source-only wording is removed from **native** execution: Bun setup/lookup/dependency lines, `extracting source archive`, and `info: source:`. Remove the old suggestion `publish a release with an envswitch_<version>_source.tar.gz asset first` from dual-mode resolution errors. Remove the Unix `expected asset URL:` line to avoid leaking signed/token-bearing URLs. PowerShell's old bare exceptions become the controlled `error:` lines above. New dry-run lines replace all old source/Bun-specific dry-run lines.

### 6.4 Exact PowerShell dry run

After local argument/platform/path validation, print these lines and return 0; `<selection>` is `latest` or the normalized pinned tag. Do not resolve kind, bootstrap Bun, invoke `gh`, execute a binary, write a profile/User PATH/mock file, create a temp/root/lock, or download anything:

```text
What if: Resolve envswitch release <selection> from <Repo>
What if: Verify SHA-256 and install the release's source or native payload to <AppRoot>\current
What if: Install envswitch launcher to <InstallDir>\envswitch.cmd
What if: Write envswitch install receipt and version marker
What if: Add <InstallDir> to the user PATH if missing
```

When requested, append `What if: Verify GitHub artifact attestation with gh` as the final line. Dry run validates platform/architecture but does not require download/hash/Bun/attestation tools to be installed. Unix has no dry-run public option in protocol 1.

## 7. Native and bridge `update`

### 7.1 Shared installer-runner handoff

The logical `InstallPlan` consists of `repo`, `appRoot`, `installDir`, optional canonical `version` pin, optional canonical `currentVersion`, and `verifyAttestations` (false for ordinary update). `launcherPath` is derived, not a separate override. `InstallerRunner` returns an exit code and, on validated success, `{ disposition: installed|repaired|unchanged, tag }`. No receipt schema field is added merely to return a transient result. The implementation may capture/tee stdout and recognize §6's terminal line; stderr/stdin are inherited or forwarded unchanged. The public `update` command adds no version/repo/attestation flags; unknown flags fail before receipt/network/process work.

Both updaters write/copy the complete installer to a newly created **private temporary directory outside `current`**, use argument-array process spawning (no shell command-string interpolation), wait for completion, and clean it in a finally/RAII path. Unix permissions are `0700` directory/`0600` script; Windows limits access to the current user and required system/administrator principals. Do not execute the bundled script in place while replacing it. Run with a neutral private temporary CWD, never the user's project. Do not fetch/execute a fresh installer from mutable main.

Unix argv is exactly `bash <tmp>/install.sh`. Pass the inherited environment plus explicit `REPO`, `APP_ROOT`, `INSTALL_DIR`, `ENVSWITCH_CURRENT_VERSION=<current-tag>`, and `ENVSWITCH_VERSION=<pin>` when pinned. If no pin/current is intended, **remove** any inherited corresponding variable, rather than accidentally honoring a caller's stale pin/version. In ordinary updates also remove inherited `ENVSWITCH_VERIFY_ATTESTATIONS` (the command exposes no such option). Preserve `GITHUB_TOKEN`, HOME/XDG/PATH, and enabled test hooks for isolated tests.

Windows argv (omit optional `-Version` or `-CurrentVersion` pairs when absent):

```text
powershell -NoProfile -ExecutionPolicy Bypass -File <tmp>\install.ps1 -AppRoot <appRoot> -InstallDir <installDir> -Repo <repo> -Version <pin> -CurrentVersion <current-tag>
```

No `-WhatIf`/`-Confirm` during update; use `-Confirm:$false` if needed to override inherited confirmation preference. Pass each path/value as one argument; the displayed command is an argv description, not a command string. An internal explicit attestation request adds `-VerifyAttestations`; ordinary updates do not. Receipt settings take precedence over unrelated shell `REPO`/destination variables on both platforms.

After child success, independently reread/validate `current/install.json` and canonical `.version`, require agreement with the returned tag/kind and selected locations, and require the receipt's payload and launcher to exist. A zero exit without an accepted terminal outcome and consistent postconditions is failure, not permission to claim an update. Never print an old → new success based only on a supplied latest tag or zero status. An `unchanged` result still requires a healthy receipt. The installer's own absolute verification is the executable-health authority; do not run the old parent executable as the newly installed version.

### 7.2 D3: Rust

Use `current_exe` plus the receipt as in §4.1. Before install lookup, reject a `.git` file/directory in the injected/default application directory or candidate `current` directory and development executable paths `.../target/{debug,release}/envswitch[.exe]` or `.../target/<triple>/{debug,release}/envswitch[.exe]`. A copied binary outside the managed versioned location or without a valid receipt prints the guidance below and exits 1; do not inspect the invoking project's `.git` as if it were the binary's source location or search arbitrary ancestors for repository markers.

The current-version label is `v<CARGO_PKG_VERSION>`, never `.version` (e1). Write the embedded `include_str!` platform installer (e2); missing `current/scripts/install.*` is **not** a pre-invocation error. Pass persisted repo/root/install directory (e3).

**Resolution ownership:** ordinary Rust update supplies no version pin and lets the embedded installer resolve latest once; this avoids a second HTTP implementation/crate. If an internal/test `InstallPlan` supplies a pin, forward it exactly as above. The installer pins all subsequent downloads to its retained response. D3 must not add a separate latest lookup just to fill `-Version`. M1 already has a resolver and passes its pin (§7.3). This is the same protocol with different resolution owners, not two install algorithms.

D3 always invokes the installer, including equal-version cases, because it owns health/repair checks. It must repair missing bundled scripts/metadata/payload when the valid receipt provides the paths; a `.version` equality check in Rust must not short-circuit it. An invalid/missing receipt still needs direct reinstall. Forward child nonzero status as the CLI status; Unix signal termination maps to `128+n`. A controlled launch/preflight/postcondition failure is 1. This deliberately retains the actual installer signal result rather than converting it to generic success.

### 7.3 M1: final TS bridge

`src/cli.ts` keeps the installed source location as the root anchor (`appDir = current`), guards source checkouts, and obtains current from `.version` with its legacy unknown-version fallback. Validate/use a receipt for this root when present, including a native receipt reached through a residual historical TS launcher. For first bridge adoption with no receipt, use the known app root; Unix uses explicit `REPO`/`INSTALL_DIR` when provided, otherwise the repo default and an exactly matching managed launcher in the default directory or process PATH. Windows uses the default repo and an exactly matching default/PATH launcher because old parameters were not stored. Reject ambiguity rather than selecting an unrelated launcher. No broad filesystem scan, and no claim that arbitrary historical custom settings are recoverable. If no safe launcher can be identified, fail with the explicit-path recovery message below. Once a receipt exists, its settings are authoritative.

Resolve latest once from the effective repo using M1's fetch seam. Validate the canonical tag and pass that exact `ENVSWITCH_VERSION`/`-Version`; failure to resolve latest is a controlled error, **not** an unpinned second lookup. It is acceptable for the installer to fetch `/releases/tags/<pin>` to validate assets: that is not another selection of latest. Copy the bundled D2b installer outside current, then use the shared handoff. Always invoke it on equality for health/repair. Missing bundled script remains a bridge error; Rust's embedded script removes this precondition only for D3.

M1's old → bridge tests use the oracle's untouched installer and prove bridge source installation; bridge → native uses a complete simulated native release. Light-mode skipped-bridge tests assert the actual old asset-not-found failure plus the documented recovery path (§8), never a compatibility bootstrap.

### 7.4 Exact update command output

The inherited installer lines remain visible. In addition, the command prints these **CLI-owned** lines (no `info:`/`success:` prefix). `<old>` is the canonical old tag; unknown bridge version uses the placeholders shown. `<new>` comes from validated installer result and post-install metadata.

Before invoking the child, print the progress event on stderr using the existing formatter/color policy:

```text
envswitch [info] updating envswitch <old-or-(unknown version)> -> <pin-or-latest>
```

Rust ordinary update uses `latest`; bridge uses its canonical pin. This progress line also occurs for unchanged/repair because the installer must check health. On child success, exactly one CLI terminal stdout line:

| Installer result | Exact CLI stdout |
| --- | --- |
| `unchanged` | `envswitch is already up to date (<new>)` |
| `installed` | `updated envswitch <old-or-(unknown)> -> <new>` |
| `repaired` | `repaired envswitch <new>` |

The installer retains its prefixed terminal line as well; the two layers are intentionally distinguishable and must not print two CLI summaries. A racing old process can report `updated envswitch v0.0.41 -> v0.0.42` while the installer reports unchanged at v0.0.42: classify the CLI as updated when `<new>` differs from the running `<old>`, otherwise use the table. No CLI success follows a failed child or failed postcondition.

Controlled command stderr (the `envswitch: ` prefix is emitted once by main):

```text
envswitch: envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)
envswitch: cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README
envswitch: cannot recover this installation's launcher directory; reinstall envswitch with explicit APP_ROOT and INSTALL_DIR (PowerShell: -AppRoot and -InstallDir) using the README instructions
envswitch: installer not found at <installer>; reinstall envswitch with the install script from the README
envswitch: no GitHub Release found for <repo>, or access is denied
envswitch: update failed: installer exited with code <code>
envswitch: update failed: could not run installer: <detail>
envswitch: update failed: installer result does not match the installed receipt and version marker; reinstall envswitch with the install script from the README
```

Each is an alternative condition, not a block printed together. The missing-receipt line covers invalid/unsupported receipts as well; exit 1 before an installer. The missing-installer line is M1-only. The release-resolution line is M1's failed preflight; D3 sees the installer's release diagnostic followed by its nonzero-status line. For a signaled child, `<code>` is `128+n`, and D3 exits with that code. Other controlled update errors exit 1; unexpected implementation errors retain main's exit 2 policy. Removed TS expectations: equal version without invoking the installer, fallback unpinned install after failed latest resolution, and zero-status success without checked committed metadata.

## 8. Release states and Light recovery

`Main installer` means the README one-liner's mutable `main/scripts/install.*`. Native rows in S0/S1 are protocol compatibility checks, not claims that native releases are already published. A deliberately pinned old source install is supported; an ordinary native updater may resolve source if that is what latest names, subject to ownership/transaction rules.

| Caller | S0: historical source latest | S1: final TS bridge source latest | S2: native latest |
| --- | --- | --- | --- |
| Fresh install via main | Existing old installer installs source/Bun. In the dual-mode transition window, source is supported **only after** §1.3's integrity gate. | Dual-mode installs checksum-verified bridge source, Bun if needed, and receipt. | Dual-mode installs verified native; no Bun. |
| Pre-bridge `update` | Old bundled installer reinstalls source (or old CLI skips equal version). | Old bundled installer installs full bridge source without integrity verification, as today; subsequent update uses bridge protocol. | Old installer requests the absent source asset, fails nonzero, and leaves prior install. User reruns README one-liner. No compatibility payload or stub. |
| Bridge `update` | Not ordinarily deployed here; if exercised, dual-mode accepts only checksum-complete source and otherwise fails closed. | Resolves/pins bridge tag, installer verifies health; unchanged or repair, not a marker-only skip. | Resolves/pins native tag; dual-mode migrates the managed launcher and records native receipt. |
| Native `update` | Not ordinarily deployed here; a simulated source latest requires full source contract, otherwise fail closed. | Not ordinarily deployed here; a simulated/pinned source install uses source/Bun branch if safe, without deleting immutable native payloads. | Embedded installer resolves once, verifies native, updates/repairs/returns unchanged, preserving saved destinations. |

S0→S1 (bridge merged, bridge not yet published) and S1→S2 (Rust merged, native not yet published) use dual-mode main installers while latest remains source. The historical-source integrity gate is mandatory in the first window; the bridge already includes checksums in the second. Never publish native first and expect an untouched old install to fetch a nonexistent compatibility asset.

### 8.1 Frozen old errors versus new recovery text

An untouched pre-bridge Unix installer cannot be taught a new message. On the public HTTP fallback its controlled stderr is exactly:

```text
error: release asset not found or unavailable: envswitch_<ver>_source.tar.gz
error: expected asset URL: https://github.com/<repo>/releases/download/<tag>/envswitch_<ver>_source.tar.gz
```

External curl/gh diagnostics may precede it. Old Windows throws `matching asset not found: envswitch_<ver>_source.tar.gz` (host exception formatting varies). The old CLI then adds `envswitch: update failed: installer exited with code <code>` if it receives that status. Do not assert that old bytes print newly invented reinstall guidance. The replacement guidance belongs in README/release notes and the new controlled errors, not a nonexistent forwarding stub.

### 8.2 Exact README/release-note recovery copy

The documentation owner must add this text to README installation/update guidance and the first native release notes (D2a writes only this spec):

> Upgrading from the Bun version: run `envswitch update` during the bridge release to prepare for native updates. If you skipped the bridge and update fails with `release asset not found or unavailable` or `matching asset not found`, rerun the install command below. Updates preserve your install paths when they are recorded in an install receipt. When reinstalling, supply any custom install paths explicitly. Your project `.env` files and Bun installation are not removed.

macOS, Linux, or WSL:

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 | iex
```

Follow with this exact paragraph:

> For an older custom installation, use the same app root, launcher directory, and repository you originally selected. The installer cannot discover every old custom launcher. Restart your terminal after reinstalling, then run `envswitch --version`; a native installation prints `envswitch <version> <target-triple>`. Reinstalling at the default location does not repair a different launcher that appears earlier on PATH.

Custom Unix example (replace these locations/repo with the actual old values; set variables on **bash**, not curl):

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh |
  REPO=rnldsalili/envswitch APP_ROOT="$HOME/.local/share/envswitch" INSTALL_DIR="$HOME/.local/bin" bash
```

Custom Windows example (download the script, then pass parameters; do not imply that variables configure `iex`):

```powershell
$installer = Join-Path $env:TEMP ("envswitch-install-" + [guid]::NewGuid() + ".ps1")
try {
    Invoke-WebRequest https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 -OutFile $installer
    & powershell -NoProfile -ExecutionPolicy Bypass -File $installer -Repo rnldsalili/envswitch -AppRoot "$env:LOCALAPPDATA\Programs\envswitch" -InstallDir "$env:LOCALAPPDATA\Programs\envswitch\bin"
} finally {
    Remove-Item -LiteralPath $installer -ErrorAction SilentlyContinue
}
```

The public one-liners are a mutable bootstrap trust choice; checksums protect subsequent payload downloads, not the script that has already been executed.

## 9. Isolated test environments and required assertions

Installer tests MUST create a fresh suite root and set `HOME`, every `XDG_*` home used by tools (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`), `USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `TMPDIR`, `TEMP`, `TMP`, `BUN_INSTALL`, and `GH_CONFIG_DIR` to suite-owned paths. Set `SHELL` and a controlled PATH; disable inherited GitHub auth except synthetic tokens needed by the case. Never bootstrap Bun or consult authenticated real `gh` during tests. Stub all source-branch bootstrap calls; native tests assert Bun was never invoked. Never call real `op`.

Unix tests create `.profile`, `.bashrc`, and `.zshrc` only under the isolated HOME and test selection, markers, idempotence, changing INSTALL_DIR, escaping, and preservation of surrounding bytes. The current `test/install-sh.test.sh` helper test must be retained/extended with isolation, not removed as obsolete. Windows sets `ENVSWITCH_INSTALL_TEST_MODE=install` and `ENVSWITCH_INSTALL_TEST_USER_PATH_FILE` for **every** mutating test; assert the real persistent User PATH API was not called. Changing HOME/USERPROFILE alone does not sandbox registry-backed PATH. Test `-WhatIf` by snapshotting all suite files, fake-tool call logs, and the mock User PATH file: none may change, and no release/asset server requests may occur.

Serve complete synthetic source/native releases from a loopback server implementing `/repos/<repo>/releases/latest`, `/releases/tags/<tag>`, and saved asset URLs; no live GitHub dependency. Record requests to prove a single latest selection and no auth header forwarding. Fake `gh` in PATH must support/fail `auth status`, `api`, `release download`, and `attestation verify` deterministically; test `gh` absent, unauthenticated, metadata failure with HTTP fallback, download failure with fallback, and requested attestation absent/failing/succeeding. Test URLs work only through the unsupported gated hook. Never write to GitHub or run publication as an installer test.

Required cross-platform assertions:

- Both kinds; all five complete asset names; partial/mixed/duplicate kind failures; missing source checksums in the exposed S0 window; canonical pin/current parsing; no latest lookup for a pin; changed latest between requests cannot change selected bits.
- Manifest absent/malformed/duplicate/selected entry missing, wrong hash, scripts mismatching standalone hashes, unsafe archive names/types/collisions, wrong native version/target/newline, nonzero help/version; each fails before launcher commit.
- Linux x64/arm64, Darwin Intel/ARM/Rosetta, WSL, Windows x64 OS with both 32/64-bit PowerShell, and rejected Windows ARM/32-bit/unknown CPUs. Use fakes for detection and real target executables in packaged smoke tests.
- Fresh install; same-tag healthy unchanged; missing payload/marker/scripts repair; missing source receipt adoption; native copied binary/no receipt guidance; invalid/escaping receipt; hostile preexisting launcher; same-version immutable binary conflict; custom paths/repo persist through updates.
- Failure injection at all three boundaries restores old launcher and metadata; lock contention; no lock deletion by a nonowner; Windows running old EXE and launcher sharing violation; signal status; crash leftovers recover by explicit reinstall rather than unsafe automatic lock stealing.
- Launcher argument/status forwarding, Unicode/metacharacter paths, safe profile guidance, no shadowed-PATH verification, no `op`/project writes, no Bun in native branch, no recursive root/source cleanup, no deletion of `.env` or user sentinel files.
- S0/S1/S2 table including old → bridge, bridge → native, and skipped-bridge actual error/reinstall recovery. Use an untouched oracle old installer for old behavior. Assert CLI terminal text and metadata postconditions, including zero-status fake child with no marker/result and failed latest lookup.

D2b owns Bash/PowerShell installer suites and their CI job; M1 owns bridge and Light migration rows; D3 owns native invocation/receipt/result/signal rows; D1 owns packaged smoke/publication gates. Bash, PowerShell, migration, and packaged-target checks are explicit publication dependencies, not assumed to run under `cargo test`. No test may install into the real home, write real Windows User PATH, publish, or rely on a live release.

## 10. Non-goals and trust boundary

No compatibility tarball, forwarding stub, universal macOS binary, Windows ARM emulation, new package manager, updater crate, native HTTP/archive stack, arbitrary mirror/target override, channel selection, public force/skip-checksum option, automatic privilege escalation, automatic Bun removal, or broad cleanup of legacy source/old binaries. Developer ID/notarization, Authenticode, independent signing keys, anti-rollback guarantees, and protocol-breaking future release layouts are outside protocol 1. Never disable Gatekeeper, strip quarantine, use `Unblock-File`, or bypass enterprise execution policy to make verification pass.

The executing README script or installed embedded/bundled script, configured repository/publisher, GitHub transport, local user account, and required platform tools are trusted. Remote release metadata, archive paths/types, tags, receipt bytes, destinations, generated shell/batch text, and authentication redirects cross validation boundaries. A same-release unsigned SHA manifest detects corruption/mismatched assets; an attacker who controls the publisher can replace both hashes and payloads. Optional repository-bound attestations add provenance checks but do not make mutable bootstrap scripts safe, create Windows publisher reputation, or guarantee local same-user filesystem race resistance. The installer lock serializes cooperating installers, not a malicious process with the user's filesystem permissions.

## 11. D2a decisions to carry into the freeze

1. Mandatory SHA verification creates the historical-source rollout prerequisite in §1.3; no legacy bypass is allowed. The orchestrator must incorporate this into publication sequencing before changing the main one-liner's script.
2. Rust delegates latest selection to its embedded installer; bridge resolves/pins with its existing fetch seam. Both consume one selected release and the same result protocol, without adding HTTP to Rust.
3. Success uses exit 0 for all three outcomes and exact terminal lines plus checked receipt/marker postconditions. CLI progress may precede an unchanged result; no marker-only skip remains.
4. Protocol 1 retains unused legacy source and old versioned binaries rather than implementing risky recursive pruning. Same-version conflicting immutable executables require explicit recovery; missing files/metadata can be repaired.
5. Source receipts use `target: null`; native receipts use the actual triple. All new markers, including Windows, are `v<ver>\n` in UTF-8 without BOM. PowerShell controlled diagnostics adopt the same `error:` stderr style as Bash.
