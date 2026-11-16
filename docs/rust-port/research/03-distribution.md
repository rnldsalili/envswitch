# Rust distribution research

Research date: **November 16, 2026**. Repository inspected at `53de029a25a717c8ac824ca8553035e2b0fec991`. Planning only; no installer, build, release, tag, or push was executed. Part A describes existing behavior, including defects; Part B is a proposed contract.

## Decision

Keep the hand-written GitHub Actions workflow and the two installers. Build five targets on matching CPU runners; use static musl on Linux. Retain the existing launcher paths and `APP_ROOT/current/.version`. Install versioned native executables behind the launchers so Windows updates never overwrite the running executable. Implement Rust `update` by running an embedded copy of the installer, with persisted install locations. Keep `0.0.${GITHUB_RUN_NUMBER}` versioning, stamped into the build checkout's Cargo manifest.

Publish SHA-256 checksums for every downloadable payload and script. Add GitHub provenance attestations when the repository's plan supports them; keep verification optional for users without `gh`. Defer independent signing keys, package managers, `cargo-dist`, and updater crates. Publish a small source compatibility archive on every Rust release so an old Bun installer can still reach the native installer. A single bridge release does **not** cover users who skip it.

## A. Current install, release, and update contract

### A1. Entry points, prerequisites, and configuration

The README executes the installer from mutable `main`, then the installer separately resolves the latest GitHub Release. These are not necessarily the same commit. Unix uses `curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh | bash`; Windows uses `irm https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 | iex`. Alternative examples retrieve the scripts through GitHub's contents API. The Unix API example decodes `.content`; the Windows example retrieves `.download_url` then calls `irm`. Neither example pins a version or verifies the script. [README.md:9][readme-install], [README.md:38][readme-gh]

| Platform/input | Exact current default or behavior |
| --- | --- |
| Unix `REPO` | `${REPO:-rnldsalili/envswitch}`; unset **or empty** uses the default. |
| Unix `INSTALL_DIR` | `${INSTALL_DIR:-$HOME/.local/bin}`. |
| Unix `APP_ROOT` | `${APP_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/envswitch}`. macOS deliberately follows this XDG-style location, not `Library/Application Support`. |
| Unix `APP_DIR` | Always `${APP_ROOT}/current`; no override. |
| Unix `HOME`, `PATH`, `SHELL` | Used for defaults, executable lookup, and profile selection. `HOME` is required by expansion under `set -u` when defaults need it. Empty/missing `SHELL` selects `.profile`. |
| Unix `BUN_INSTALL` | Exported as `${BUN_INSTALL:-$HOME/.bun}` only after automatic Bun installation; its `bin` is prepended to this process's `PATH`. |
| Both `GITHUB_TOKEN` | Optional bearer header on HTTP calls. `gh` uses its own authentication environment/configuration; the scripts do not explicitly pass this token as a CLI argument. |
| Unix `ENVS_SWITCH_INSTALL_TEST_MODE` | `ensure-profile-path` calls the profile helper with `$1` and `$2`; `shell-profile-path` prints the selected profile. Both exit before normal installation and validation. This is an internal test hook. |
| Windows `-Repo` | String parameter, default `rnldsalili/envswitch`; no explicit reading of environment variable `REPO`. |
| Windows `-AppRoot` | If omitted/empty, `$env:LOCALAPPDATA\Programs\envswitch`. Environment variable `APP_ROOT` is not read by this installer. |
| Windows `-InstallDir` | If omitted/empty, `<effective AppRoot>\bin`. Environment variable `INSTALL_DIR` is not read. |
| Windows `LOCALAPPDATA` | Required **even if both path parameters were supplied**. The error suggests explicit parameters, but they do not bypass this unconditional check. |
| Windows `USERPROFILE` | Used for the Bun fallback path and bootstrap PATH update. |
| Windows common parameters | `[CmdletBinding(SupportsShouldProcess = $true)]` exposes `-WhatIf` and `-Confirm`. `-WhatIf` returns through a special early branch; ordinary confirmation coverage is incomplete. |

Evidence: [scripts/install.sh:3][sh-defaults], [scripts/install.sh:62][sh-profile-62], [scripts/install.sh:164][sh-bun], [scripts/install.sh:343][sh-hooks], [scripts/install.ps1:1][ps-defaults], [scripts/install.ps1:56][ps-headers], [scripts/install.ps1:270][ps-main].

Unix validates `REPO` with `^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$`. Both directory overrides must begin with `/` and contain no `..` substring anywhere; this rejects even a legitimate name containing two dots. It does not canonicalize paths, reject shell metacharacters, or prevent selecting `/` or another unsafe root. Windows uses the same repository grammar but has no corresponding absolute-path/root validation. [scripts/install.sh:36][sh-validation], [scripts/install.ps1:48][ps-repo]

Unix accepts `darwin` and `linux` from lowercased `uname`; a readable `/proc/version` containing `microsoft` changes only the WSL log message. It never detects architecture. Other operating systems fail. PowerShell requires `RuntimeInformation.IsOSPlatform(Windows)`; running it inside Linux/WSL PowerShell fails with guidance to use Bash. [scripts/install.sh:127][sh-platform], [scripts/install.ps1:38][ps-platform]

The README's only external application prerequisite is the authenticated 1Password CLI (`op whoami`). Bun is installed automatically by the scripts if absent. Unix also relies on ordinary utilities including `uname`, `tr`, `basename`, `grep`, `sed`, `head`, `tar`, `mktemp`, `install`, `chmod`, and filesystem commands without a complete preflight. Windows checks for `tar` before extraction and uses built-in PowerShell HTTP/JSON/file APIs. [README.md:99][readme-prereq], [scripts/install.sh:258][sh-source], [scripts/install.ps1:162][ps-source]

### A2. Files installed and launcher contents

| OS | Application root | Source/dependencies | Launcher | Version marker |
| --- | --- | --- | --- | --- |
| macOS, Linux, WSL | `$APP_ROOT`, default `${XDG_DATA_HOME:-$HOME/.local/share}/envswitch` | `$APP_ROOT/current`, including `src`, `scripts`, package files, and production dependencies | `$INSTALL_DIR/envswitch`, default `$HOME/.local/bin/envswitch`, mode `0755` | `$APP_ROOT/current/.version` |
| Windows | `-AppRoot`, default `%LOCALAPPDATA%\Programs\envswitch` | `<AppRoot>\current` | `<InstallDir>\envswitch.cmd`, default `<AppRoot>\bin\envswitch.cmd`, ASCII | `<AppRoot>\current\.version` |

The source archive contains a single top-level `envswitch/` directory. The workflow copies `package.json`, `bun.lock`, `README.md`, `tsconfig.json`, all `src/`, and all `scripts/`; dependencies are installed on the user's machine. `current` is a real directory, not a symlink or version selector. The Unix marker is the full release tag plus LF, written with `printf '%s\n'`. Windows writes the full tag with `Set-Content`, using that PowerShell version's text encoding/newline defaults; the CLI trims it. [release.yml:48][release-assets], [scripts/install.sh:258][sh-source], [scripts/install.sh:324][sh-main-324], [scripts/install.ps1:285][ps-main-285], [scripts/install.ps1:307][ps-install-307]

Unix generated launcher, with `<APP_DIR>` expanded to the configured absolute source path during installation:

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

exec "$BUN_BIN" "<APP_DIR>/src/cli.ts" "$@"
```

The installer builds this in its temporary directory, sets `0755`, creates `INSTALL_DIR` if needed, and runs `install -m 0755` to the final path. Bun is resolved anew on every command; it is not bound to the Bun executable used at install time. The fallback always uses `HOME/.bun`, regardless of a previous custom `BUN_INSTALL`. Path interpolation is not escaped for embedding quotes or shell substitutions. [scripts/install.sh:282][sh-launcher]

Windows generated launcher, with `<CLI_PATH>` expanded to `<AppRoot>\current\src\cli.ts`:

```bat
@echo off
setlocal
where bun >NUL 2>NUL
if %ERRORLEVEL% EQU 0 (
  bun "<CLI_PATH>" %*
  exit /b %ERRORLEVEL%
)
if exist "%USERPROFILE%\.bun\bin\bun.exe" (
  "%USERPROFILE%\.bun\bin\bun.exe" "<CLI_PATH>" %*
  exit /b %ERRORLEVEL%
)
echo envswitch: bun is not installed or not on PATH 1>&2
exit /b 127
```

`Write-Launcher` creates the directory, writes ASCII under `ShouldProcess`, updates User PATH, and checks that the file exists. It does not execute `--help`. The `%ERRORLEVEL%` expressions inside parenthesized blocks are expanded before the Bun command runs, so this launcher is not a reliable child exit-code propagator. ASCII also cannot preserve every non-ASCII configured path. These are current limitations, not behavior to reproduce. [scripts/install.ps1:233][ps-launcher]

The README manual procedure creates a smaller `exec bun ... "$@"` launcher, has no Bun fallback, does not write `.version`, does not edit PATH, and is not an idempotent reinstall procedure (`mv` can encounter existing `current`). Such installations have an unknown version to `update`. [README.md:61][readme-manual]

### A3. Bun discovery and bootstrap

Unix: use `command -v bun`, else executable `$HOME/.bun/bin/bun`, else require `curl` and execute `curl -fsSL https://bun.com/install | bash`. Then export defaulted `BUN_INSTALL` and prepend its `bin` to `PATH`; retry discovery. Run the selected Bun with `--version` and discard stdout. Under `set -e`, failure stops installation. No Bun version is pinned or checked against a minimum. [scripts/install.sh:150][sh-bun-lookup], [scripts/install.sh:164][sh-bun]

Windows: use `Get-Command bun`'s `.Source`, else any existing `%USERPROFILE%\.bun\bin\bun.exe`. Otherwise execute `powershell -NoProfile -ExecutionPolicy Bypass -Command "irm bun.sh/install.ps1 | iex"`, prepend the fallback directory if it exists, and retry. It invokes `bun --version` with all streams discarded but does not explicitly reject its nonzero `$LASTEXITCODE`; `$ErrorActionPreference = "Stop"` alone does not generally turn native exit codes into exceptions in Windows PowerShell. No Bun pin, bootstrap checksum, or signature check. [scripts/install.ps1:90][ps-bun]

Both perform Bun setup **before** resolving whether an envswitch release is available. A failed envswitch installation can therefore leave Bun and Bun's profile changes installed. Neither installer removes Bun on failure or on reinstallation.

### A4. Release resolution, download, extraction, and verification

**Unix sequence:** [scripts/install.sh:194][sh-latest], [scripts/install.sh:227][sh-download], [scripts/install.sh:258][sh-source], [scripts/install.sh:315][sh-main]

1. If `gh` exists and `gh auth status` succeeds, call `gh api "repos/${REPO}/releases/latest" --jq '.tag_name'`. Errors are suppressed and converted to an empty result. This branch has **no HTTP API fallback** if that API command fails.
2. Otherwise require `curl`; call `https://api.github.com/repos/${REPO}/releases/latest` with `-fsSL` and an optional `Authorization: Bearer ${GITHUB_TOKEN}` header. Extract `tag_name` with `sed` and `head`, not a JSON parser. API/network failures also become an empty result.
3. Validate tag against `^v?[a-zA-Z0-9][a-zA-Z0-9._-]*$`; fail if invalid/empty. This grammar is not SemVer. It even accepts the single tag `v`, which becomes an empty version after stripping one leading `v`.
4. Strip one leading lowercase `v`; construct `envswitch_${version}_source.tar.gz`. No `VERSION` variable, flag, positional version, prerelease selector, or explicit downgrade protection exists.
5. Create `mktemp -d` and register an `EXIT` cleanup trap.
6. Prefer authenticated `gh release download "$latest" --repo "$REPO" --pattern "$archive" --dir "$tmpdir"`. If it fails, fall back to `curl -fsSL -H 'Accept: application/octet-stream'` and the optional bearer token at `https://github.com/${REPO}/releases/download/${latest}/${archive}`. This is the browser-style asset URL, not the API asset-ID URL. In this fallback, `curl` is not preflighted if metadata was obtained through `gh`; private-asset authentication at this URL is less robust than using the asset API.
7. Extract with `tar -xzf` under `<tmp>/staging`. Require `<staging>/envswitch/package.json` and `<staging>/envswitch/src/cli.ts` to be files.
8. Run `bun install --production --frozen-lockfile` inside staging, then `rm -rf "$APP_DIR"` and `mv` the staged source into `current`. Write `.version`, replace launcher, print success, then handle PATH/verification.

**Windows sequence:** [scripts/install.ps1:70][ps-latest], [scripts/install.ps1:130][ps-download], [scripts/install.ps1:162][ps-source], [scripts/install.ps1:296][ps-install]

1. Try authenticated `gh api "repos/$Repository/releases/latest"`; parse the full JSON response if exit zero and nonempty. On failure, unlike Unix, fall back to `Invoke-RestMethod` against the same GitHub API endpoint. Default HTTP `Accept` is `application/vnd.github+json`, with optional bearer token.
2. Remove one leading `v`; require the remaining version to match `^[a-zA-Z0-9][a-zA-Z0-9._-]*$`. Construct the identical source asset name. No version-pin parameter exists.
3. Create a GUID-named directory under `[System.IO.Path]::GetTempPath()` within `try/finally`.
4. Prefer `gh release download <tag> --repo <repo> --pattern <archive> --dir <temp>`; on failure, select the first exact `.assets[].name` match from the release response and download its `.url` (GitHub's asset API URL) with `Invoke-WebRequest`, octet-stream `Accept`, and optional bearer token.
5. Require `tar`, extract under staging, explicitly check tar exit code, and require package/CLI paths to exist (the check is `Test-Path`, not a file-type check).
6. Push staging as the working directory, run `bun install --production --frozen-lockfile`, explicitly check its exit code, and restore location in `finally`.
7. Under `ShouldProcess`, recursively remove old `current` and move staged source there. Then write `.version` outside that guard, create launcher/PATH, and print success/guidance.
8. Recursively remove the temporary directory in `finally` if it exists.

**Verification today:** HTTPS only. No SHA checksum, detached signature, artifact attestation, expected publisher check, or archive-member/path-traversal validation in either installer. Package/CLI shape checks and a frozen dependency lockfile are not authenticity verification. The release workflow produces no checksum/signature assets. [scripts/install.sh:227][sh-download], [scripts/install.sh:258][sh-source], [scripts/install.ps1:130][ps-download], [scripts/install.ps1:162][ps-source], [release.yml:48][release-assets]

**Cleanup and reinstall:** Normal success/failure after temporary-directory setup cleans the registered directory; abrupt termination, machine failure, or failure before registration is outside that guarantee. Staging and dependency installation happen before deleting old `current`, but deletion followed by move is not transactional and may cross filesystems. A failed move, launcher write, or PATH operation can leave a broken/partially updated installation; success is printed before Unix's final `--help` check. A direct reinstall always downloads/reinstalls the latest release even if `.version` already matches. Nothing tracks historical launcher locations, and an old launcher outside the newly selected directory remains. Concurrent installers are not locked. [scripts/install.sh:30][sh-cleanup], [scripts/install.sh:275][sh-source-275], [scripts/install.sh:328][sh-main-328], [scripts/install.ps1:202][ps-source-202], [scripts/install.ps1:307][ps-install-307]

### A5. PATH behavior and exact message inventory

Unix checks whether `:${PATH}:` contains `:${INSTALL_DIR}:`. If present, it verifies the file is executable, then runs **`envswitch --help` by command name**, not by absolute installed path; an earlier PATH entry can therefore shadow the installation. Otherwise, choose `$HOME/.zshrc` for basename `zsh`, `$HOME/.bashrc` for `bash`, and `$HOME/.profile` for everything else; ignore `ZDOTDIR`, fish syntax, and alternative startup conventions. Append this block unless any existing line contains the opening marker; an existing block is neither validated nor rewritten when `INSTALL_DIR` changes. The parent shell's PATH cannot be changed by a piped child installer. [scripts/install.sh:57][sh-profile], [scripts/install.sh:79][sh-profile-write], [scripts/install.sh:98][sh-guidance]

```bash

# >>> envswitch PATH >>>
export PATH="<INSTALL_DIR>:$PATH"
# <<< envswitch PATH <<<
```

Windows reads persistent **User** PATH, compares entries case-insensitively after trimming trailing `\`, appends the directory to persistent User PATH if absent, and prepends it to the installer's process PATH in that branch. If User PATH already contains it but current process PATH does not, the function returns without fixing process PATH. It does not compare against Machine PATH or normalize quotes, whitespace, slashes, environment expansion, or aliases. It always prints terminal-restart guidance. [scripts/install.ps1:210][ps-path], [scripts/install.ps1:314][ps-success]

The following are all script-owned normal-operation messages. `<...>` denotes a runtime substitution; prefixes, case, and punctuation are literal. Tool-owned `curl`, Bun, `tar`, `install`, PowerShell errors, and confirmation prompts are additional, variable output.

Unix stdout (`info:`/`success:` via `printf`):

```text
info: detected macOS
info: detected WSL/Linux
info: detected Linux
info: Bun is not installed; installing Bun
info: using Bun at <bun_bin>
info: downloading <archive> with GitHub CLI
info: GitHub CLI download did not find <archive>; trying direct download
info: downloading <archive>
info: extracting source archive
info: installing production dependencies
info: installing launcher to <INSTALL_DIR>/envswitch
success: installed envswitch <full_tag>
info: source: <APP_ROOT>/current
info: launcher: <INSTALL_DIR>/envswitch
```

Unix PATH-already-present branch:

```text
info: verifying envswitch launcher
info: run 'envswitch --help' to verify the installation
```

Unix PATH-missing branch (one of the first two lines, then the remaining three):

```text
info: PATH block already exists in <profile_path>
info: added <INSTALL_DIR> to PATH in <profile_path>
info: restart your terminal, or run this command now:
info: export PATH="<INSTALL_DIR>:$PATH"
info: then run 'envswitch --help' to verify the installation
```

Unix controlled errors on stderr, with explicit exit `1`:

```text
error: invalid repository format: <repo>
error: expected owner/repo
error: invalid <INSTALL_DIR|APP_ROOT>: <dir>
error: <INSTALL_DIR|APP_ROOT> must be an absolute path without '..'
error: launcher was not installed or is not executable: <launcher>
error: unsupported OS for this installer: <lowercase_uname>
error: use scripts/install.ps1 on Windows
error: curl is required to install Bun automatically
error: install Bun manually from https://bun.com/docs/installation, then rerun this installer
error: Bun installation finished, but bun was not found on PATH
error: curl or an authenticated GitHub CLI is required to download envswitch
error: no GitHub Release found for <REPO>, or access is denied
error: publish a release with an envswitch_<version>_source.tar.gz asset first
error: release asset not found or unavailable: <archive>
error: expected asset URL: <url>
error: archive did not contain an envswitch source package
```

Evidence: [scripts/install.sh:14][sh-logging], [scripts/install.sh:36][sh-validation], [scripts/install.sh:79][sh-profile-write], [scripts/install.sh:98][sh-guidance], [scripts/install.sh:127][sh-platform], [scripts/install.sh:164][sh-bun], [scripts/install.sh:194][sh-latest], [scripts/install.sh:227][sh-download], [scripts/install.sh:258][sh-source], [scripts/install.sh:282][sh-launcher], [scripts/install.sh:337][sh-main-337].

Windows host output (`Write-Host`, not success-pipeline values):

```text
info: Bun is not installed; installing Bun
info: using Bun at <bun>
info: downloading <ArchiveName> with GitHub CLI
info: GitHub CLI download did not find <ArchiveName>; trying direct download
info: downloading <ArchiveName>
info: extracting source archive
info: installing production dependencies
info: added <Directory> to the user PATH
success: installed envswitch <full_tag>
info: source: <appDir>
info: launcher: <launcher>
info: restart your terminal, or run this command now:
info: $env:Path = "<InstallDir>;$env:Path"
info: then run 'envswitch --help' to verify the installation
```

Windows controlled exception messages (`throw`, **no** script-added `error:` prefix; the host renders exception context):

```text
unsupported OS for this installer. Use scripts/install.sh on macOS, Linux, or WSL.
invalid repository format: <Value>. Expected owner/repo.
no GitHub Release found for <Repository>, or access is denied
Bun installation finished, but bun was not found on PATH
matching asset not found: <ArchiveName>
tar is required to extract the envswitch source archive
failed to extract <ArchivePath>
archive did not contain an envswitch source package
bun install failed
launcher was not installed: <targetLauncher>
LOCALAPPDATA is not set. Pass -InstallDir and -AppRoot explicitly.
latest release tag is invalid: <tag_name>
```

Windows explicit dry-run output, after OS/repo/`LOCALAPPDATA` validation but before Bun/download/filesystem work:

```text
What if: Install Bun if missing
What if: Download the latest envswitch source release asset from <Repo>
What if: Install envswitch source to <appDir>
What if: Install envswitch launcher to <InstallDir>\envswitch.cmd
What if: Add <InstallDir> to the user PATH if missing
```

Evidence: [scripts/install.ps1:11][ps-logging], [scripts/install.ps1:38][ps-platform], [scripts/install.ps1:48][ps-repo], [scripts/install.ps1:70][ps-latest], [scripts/install.ps1:90][ps-bun], [scripts/install.ps1:130][ps-download], [scripts/install.ps1:162][ps-source], [scripts/install.ps1:210][ps-path], [scripts/install.ps1:233][ps-launcher], [scripts/install.ps1:270][ps-main], [scripts/install.ps1:296][ps-install].

### A6. Release workflow and repository metadata

| Area | Exact current contract |
| --- | --- |
| Triggers | Every push to branch `main`; `workflow_dispatch` with no declared inputs. No tag trigger, path filter, release-event trigger, concurrency group, or dispatch branch restriction. |
| Default permission | `contents: read`. |
| Verify job | `ubuntu-latest`; checkout with `persist-credentials: false`; setup Bun; `bun install --frozen-lockfile`; `bun run test`; `bun run typecheck`. |
| Publish job | Requires verify; `ubuntu-latest`; overrides permission to `contents: write`; separate checkout, also without persisted credentials. |
| Version | Shell-local `VERSION="0.0.${GITHUB_RUN_NUMBER}"`; independent of package metadata, commit message, and tag history. `GITHUB_RUN_ATTEMPT` is absent, so reruns reuse the version. |
| Packaging | Remove `release` and `dist/release`; create `dist/release/envswitch`; copy the explicit package files plus `src/` and `scripts/`; tar that directory to `release/envswitch_<ver>_source.tar.gz`. No executable compilation, OS matrix, checksum, signature, or attest step. |
| Tag and release | `TAG="v0.0.${GITHUB_RUN_NUMBER}"`; `GH_TOKEN: ${{ github.token }}`; `gh release create "$TAG" release/* --target "$GITHUB_SHA" --generate-notes --latest`. GitHub creates the tag implicitly if absent, targeting that run's SHA. No explicit signed/annotated tag operation. Release is not marked draft or prerelease. |
| Repeat invocation | No existence check or asset reconciliation; an existing tag/release may make creation fail. Parallel runs can finish out of order and each sets `--latest`. |

Evidence: [release.yml:3][release-trigger], [release.yml:13][release-verify], [release.yml:35][release-publish], [release.yml:48][release-assets], [release.yml:63][release-create].

Both existing third-party action references are pinned to full commit SHAs, with these literal version comments:

```yaml
uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0
uses: oven-sh/setup-bun@0c5077e51419868618aeaa5fe8019c62421857d6 # v2.2.0
```

The Bun setup step supplies no `bun-version`; pinning the action does not pin Bun. Runner labels and the installed `gh` CLI also float. [release.yml:18][release-verify-18]

`package.json` has **no `version` field**, is `private: true`, uses ESM, and maps its `envswitch` bin to `src/cli.ts`. `build` only runs typecheck. `test` is `bun test && bash test/install-sh.test.sh`. Production dependency: `@inquirer/prompts: ^8.5.2`; development versions: `@types/bun: 1.3.14`, `typescript: 5.9.3`. [package.json:1][package]

`.gitignore` ignores `node_modules/`, `dist/`, `.env` and `.env.*` with example-file exceptions, logs, coverage, TypeScript build info, `.version`, and editor/OS files. It does **not** currently ignore Rust `target/`, and it does not ignore workflow output `release/`. Neither `Cargo.toml` nor a Rust toolchain/lockfile exists in this inspected tree. [\.gitignore:1][gitignore]

The README example `envswitch_0.1.0_source.tar.gz` demonstrates the naming shape, not the workflow's actual `0.0.<run>` derivation. Contributor `bun run build` is a TypeScript check, not packaging. [README.md:76][readme-assets], [README.md:494][readme-development]

### A7. `envswitch update`

Current dependency scope: command dispatch, `UPDATE_FLAGS`, hard-coded repository constant, `runUpdate`, version-file helpers, API lookup, installer spawning, dependency injection, help text, CLI update tests, both bundled scripts, release archive contents, and README guidance. There is no independent updater executable or npm update path. [src/cli.ts:97][cli-flags], [src/cli.ts:172][cli-dispatch], [src/cli.ts:334][cli-update], [src/cli.ts:459][cli-deps], [src/cli.ts:722][cli-help]

1. Accept only the `help` flag (handled by the main help path); no `--version`, `--force`, target pin, or update-specific repository argument. Extra positionals are not checked by `runUpdate`.
2. `appDir` defaults to `resolve(import.meta.dir, "..")`, so an installed copy identifies `APP_ROOT/current` from its source location, not the invoking working directory.
3. If `<appDir>/.git` exists, fail, including when it is a worktree-style `.git` file. This does not prove every source checkout is detected if its `.git` lives in an ancestor.
4. Require `<appDir>/scripts/install.ps1` on `process.platform === "win32"`; otherwise require `install.sh`. This happens **before** the equal-version short-circuit.
5. Read `<appDir>/.version` using `Bun.file().text().trim()`; empty or any read failure becomes unknown.
6. Fetch `https://api.github.com/repos/rnldsalili/envswitch/releases/latest` using `fetch`, JSON `Accept`, and optional `GITHUB_TOKEN`. This uses the hard-coded repository, **not** `REPO`, and never tries `gh`. Non-OK response, network failure, invalid JSON, or a missing/empty string tag becomes unknown. It does not validate the tag grammar used by installers.
7. Strip one leading `v` for equality only. If both tags exist and match, print `envswitch is already up to date (<current>)` and stop. There is no SemVer ordering; a different older latest tag triggers a reinstall/downgrade too.
8. Otherwise log `updating envswitch <current-or-(unknown version)> -> <latest-or-latest>` and invoke the **already-installed** script, passing `dirname(appDir)` as app root.
9. Unix child: `bash <scriptPath>` with inherited stdin/stdout/stderr and `{ ...process.env, APP_ROOT: appRoot }`. Existing `INSTALL_DIR`/`REPO` survive only if present in this invocation's environment; their original values were never recorded.
10. Windows child: `powershell -NoProfile -ExecutionPolicy Bypass -File <scriptPath> -AppRoot <appRoot>` with the same environment. It does not forward `-InstallDir` or `-Repo`; environment `INSTALL_DIR`, `APP_ROOT`, and `REPO` do not substitute for these PowerShell parameters. A custom Windows launcher directory can revert to `<appRoot>\bin`.
11. The installer resolves latest **again**; the tag used for the skip/log is not passed as a pin. A newer release can appear between lookups.
12. Nonzero child exit fails. On zero, reread the same `.version` path and print `updated envswitch <old-or-(unknown)> -> <new-file-or-latest-or-latest>`. It does not verify a version advance, launcher health, or agreement between installed bits and marker. A zero exit with an unchanged marker can report `v0.0.4 -> v0.0.4` as success.

Evidence: [src/cli.ts:334][cli-update], [src/cli.ts:368][cli-version], [src/cli.ts:381][cli-fetch], [src/cli.ts:398][cli-install], [src/cli.ts:459][cli-deps], [test/cli.test.ts:429][cli-test-unknown].

Literal controlled failure messages before `main` adds the `envswitch: ` stderr prefix:

```text
envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)
installer not found at <installer>; reinstall envswitch with the install script from the README
update failed: installer exited with code <code>
```

Default progress output, without ANSI color, is `envswitch [info] updating envswitch ...` on stderr; success/up-to-date lines use stdout. `main` returns `1` for `EnvswitchError`, `2` for other errors, and `130` for prompt cancellation. [src/cli.ts:108][cli-progress], [src/cli.ts:178][cli-errors], [src/cli.ts:334][cli-update]

### A8. What the current tests establish

`test/install-sh.test.sh` uses a temporary directory, registers cleanup, and invokes only the `ensure-profile-path` test hook twice. It checks the opening marker, literal PATH export, and one opening marker after the second call. Its success line is `install.sh PATH profile helper ok`. It does **not** run installation, test profile selection, use an isolated `HOME`, test changed directories, validate the closing marker, fetch an asset, bootstrap Bun, cover failures/rollback, or exercise Windows. Its explicit profile/launcher destinations are temporary. [test/install-sh.test.sh:1][sh-test]

`test/cli.test.ts` covers equal-version skip, installer path/app-root arguments and reporting, `.git` rejection, missing script, nonzero installer, missing version, failed latest lookup, and unknown flags. These tests stub `fetchLatestVersion` and `runInstaller`; they do not execute the spawn implementation, prove override persistence, verify the release network protocol, or test replacing files while the source is running. [test/cli.test.ts:292][cli-test]

## B. Proposed Rust distribution contract

### B1. Build targets and build strategy

Use five matrix rows. WSL selects a Linux row; there is no WSL-specific asset.

| Platform | Rust target / asset suffix | Runner | Build details |
| --- | --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | `macos-15` (arm64) | Native Cargo build/test; set and test an explicit deployment target compatible with the compiler/dependencies. |
| macOS Intel | `x86_64-apple-darwin` | `macos-15-intel` | Native Cargo build/test; separate binary, no universal-binary packaging. |
| Linux x86_64 | `x86_64-unknown-linux-musl` | `ubuntu-24.04` | Matching CPU, musl target plus `musl-tools`; static CRT; native execution of the resulting target tests. |
| Linux arm64 | `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | Matching CPU, arm64 musl compiler/linker and Rust target; verify the compiler prefix available on the image instead of assuming an x86 cross-toolchain name. |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows-2025` | Native Cargo build/test with MSVC; set `-C target-feature=+crt-static` for the release target and confirm dependency compatibility. |

GitHub currently lists these explicit runner labels for both public and private repositories; ARM Linux does not inherently require a paid larger runner. Labels pin an OS generation, not a disk-image digest. Rust documents the target triples and platform floors; the final support floor must be checked against the chosen compiler and dependency graph, then exercised on the oldest supported systems. [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), [Rust platform support](https://doc.rust-lang.org/rustc/platform-support.html)

**Why musl:** one Linux build per CPU avoids an accidental glibc floor inherited from a new Ubuntu build host and can run on glibc distributions and Alpine. Static libc does not make a binary independent of the Linux kernel, CA configuration, DNS behavior, CPU baseline, external programs, or filesystem semantics. Use baseline target CPU settings, not `target-cpu=native`. Avoid introducing OpenSSL/native dynamic library requirements through release/update dependencies; this design keeps update HTTP in the platform installers. Confirm static linkage using `readelf` program headers/dependencies and `file`, then smoke-test on glibc Linux, Alpine, and WSL. The Rust program still needs an installed/authenticated `op` for its core operation; musl availability does not imply 1Password officially supports every Linux distribution.

**GNU alternative:** `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` simplify some native dependencies and follow glibc/NSS behavior, but require a declared and tested glibc floor. An Ubuntu 24.04 native build does not guarantee glibc 2.17 compatibility merely because rustc supports it. If musl exposes a concrete dependency or DNS incompatibility, replace the affected rows with GNU builds using an older compatible sysroot or explicit `cargo-zigbuild` glibc target; do not publish both libc families by default.

**Native vs. cross alternatives:**

| Approach | Benefits | Costs / decision |
| --- | --- | --- |
| Matching CPU hosted runners + Cargo | Direct execution of tests and final payload; straightforward macOS/MSVC signing later; no emulation or separate Rust build framework. | Five runner jobs; musl linker setup remains necessary. **Choose this.** |
| `cross` 0.2.5 | Containerized Linux cross toolchains and optional emulated testing; useful where a native ARM runner is unavailable. | Docker/Podman and image lifecycle/digest pinning; macOS and Windows MSVC still need their own strategy; the latest stable tool release is old. No need here. [Official project](https://github.com/cross-rs/cross), [release](https://github.com/cross-rs/cross/releases/tag/v0.2.5). |
| `cargo-zigbuild` 0.24.1 | Zig-based Linux cross linking and explicit glibc version targeting, without Docker; useful for a GNU fallback. | Adds Zig/tool version/checksum pinning and target-test logistics. `+crt-static` glibc linking is not supported by Zig's toolchain; use musl for static libc. [Versioned README](https://github.com/rust-cross/cargo-zigbuild/blob/v0.24.1/README.md), [release](https://github.com/rust-cross/cargo-zigbuild/releases/tag/v0.24.1). |

Neither alternative eliminates the need to run final binaries on their intended OS/architecture. No `cross`, Zig, or QEMU dependency is recommended for the initial implementation.

### B2. Hand-written Actions vs. `cargo-dist`

`cargo-dist` is now branded **dist**; current upstream release is **0.34.0**, not the stale 0.33.0 shown in some search results. It generates/checks release CI, builds platform archives, and can generate shell/PowerShell installers, npm packages, Homebrew formulas, MSI installers, release metadata/manifests, and checksums. Enable only requested installer types. It can optionally install an `envswitch-update` companion built from `axoupdater`; it does not automatically turn that executable into an internal `envswitch update` subcommand. Library integration is also possible. [Official release](https://github.com/axodotdev/cargo-dist/releases/tag/v0.34.0), [dist project](https://github.com/axodotdev/cargo-dist), [updater guide](https://axodotdev.github.io/cargo-dist/book/installers/updater.html)

Dist normally releases on tag push. `dispatch-releases = true` changes this to workflow dispatch, with a dry-run mode; it does not directly mean “publish every push to main.” A main-triggered wrapper, deliberate version/tag input, custom jobs, or a hand-maintained trigger customization would still be needed. Dist supports configured install directories, messages, custom runners/jobs, and `github-action-commits` SHA overrides. Its generated workflow must remain consistent with its generation/configuration checks. [Configuration reference](https://axodotdev.github.io/cargo-dist/book/reference/config.html), [CI customization](https://axodotdev.github.io/cargo-dist/book/ci/customizing.html)

Dist generates per-archive SHA-256 checksums by default and a unified checksum manifest. Generation of checksums and enforcement by every installer are separate questions: its checksum documentation explicitly cautions that fetching-installer support is still evolving. Inspect the exact generated 0.34.0 scripts before making an enforcement claim. [Checksum guide](https://axodotdev.github.io/cargo-dist/book/artifacts/checksums.html)

| Requirement | Hand-written workflow/installers | Dist |
| --- | --- | --- |
| Push main → one release | Extend current trigger and job graph. | Supported with orchestration/customization; not default tag flow. |
| Existing `APP_ROOT/current`, split launcher directory | Explicitly preserve it. | Install-path support helps, but source-to-native cleanup and custom launchers still need migration logic. |
| Preserve Unix variables and PowerShell parameters/messages | Small focused replacement in current scripts. | Generated installer options/receipts are a different public contract; compatibility wrapper or customization needed. |
| Old source asset consumers | Publish bridge tarball in existing workflow. | Additional custom artifact/job regardless. |
| Updater/install receipts | Persist only envswitch's required locations/target. | Standard receipt/updater ecosystem, but existing installations have none. |
| Future many package managers | More work per format. | Strong advantage. |

**Recommendation:** keep hand-written Actions/installers for this port. Dist would still leave the hardest requirement—legacy migration—as custom code while adding generated configuration and a receipt transition. Reconsider when several package-manager formats or substantially more targets are required. Do not fork dist's generated templates for this five-target release.

### B3. Versioning and CI enforcement

Retain `VERSION=0.0.${GITHUB_RUN_NUMBER}` and `TAG=v${VERSION}` for the port. This preserves the requested automatic release on every successful main push and avoids requiring a manifest-version commit for every push. In **each disposable CI checkout**, stamp the Rust package's version to that exact value before compiling. The checked-in manifest may start at `0.0.0`; it is not the release authority under this policy. Do not commit or push the stamped manifest. This is a deliberate distinction from normal Cargo package versioning.

CI contract:

1. One planning job computes `version`, `tag`, and source SHA; matrix jobs consume those outputs, never derive independent values.
2. Pin Rust **1.100.0** in `rust-toolchain.toml` (current stable released November 12). Use runner-provided `rustup` to install/select that exact toolchain and required targets/components; no extra Rust setup action is necessary. Keep `rust-version`/MSRV a separate tested decision. [Official release](https://blog.rust-lang.org/2026/11/12/Rust-1.100.0/), [stable channel manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml)
3. Commit `Cargo.lock`. After stamping the package version in the disposable checkout, deliberately regenerate/reconcile its local package entry in the lockfile without upgrading dependencies, assert that the dependency graph is otherwise unchanged, then build/test with `--locked`. A changed package version can invalidate a lockfile; simply stamping then promising `--locked` without reconciling it is insufficient.
4. Assert `cargo metadata --no-deps --format-version 1` reports the planned version. Assert each built `envswitch --version` prints the same version; assert tag/archive/marker values agree. Record source SHA in release metadata.
5. Keep `push: main` and manual dispatch, but restrict actual publication to `refs/heads/main`. Verify/unit-test all targets; run isolated installer and compatibility migration tests before publish.
6. Serialize release publication without canceling an in-progress release. GitHub concurrency groups alone do not guarantee FIFO ordering; before promotion, refuse to replace latest with a lower published run/version. Build all assets, upload to a draft release, verify the expected set/checksums, and only then publish/promote latest. Incomplete releases must not be visible through `/releases/latest`.
7. A rerun reuses the tag. If an already-published release for the same SHA is complete, report a no-op; if SHA differs or payloads conflict, fail. Never overwrite public assets under an existing tag. Handle incomplete drafts explicitly; do not assume rebuilds are byte-for-byte reproducible.
8. Build jobs have `contents: read`; only publishing gets `contents: write`. Grant OIDC/attestation permissions only to attesting jobs. Retain `persist-credentials: false`.

**Alternative considered:** make `Cargo.toml` the semantic version authority and publish only when that version changes, enforcing `v<manifest-version>` and rejecting reused tags. That is cleaner for curated releases/crates.io, but changes the current every-main-push release contract. Avoid build-metadata-only versions for ordering: SemVer precedence ignores `+metadata`. Do not add a release-bump bot solely for this port.

### B4. Assets, checksums, provenance, and OS signing

Use the full Rust target triple in the basename; version excludes the leading `v`:

```text
envswitch_<ver>_aarch64-apple-darwin.tar.gz
envswitch_<ver>_x86_64-apple-darwin.tar.gz
envswitch_<ver>_x86_64-unknown-linux-musl.tar.gz
envswitch_<ver>_aarch64-unknown-linux-musl.tar.gz
envswitch_<ver>_x86_64-pc-windows-msvc.zip
envswitch_<ver>_source.tar.gz             # legacy compatibility payload
install.sh
install.ps1
SHA256SUMS
```

Native archives contain the known executable (`envswitch` or `envswitch.exe`), both installers under `scripts/`, and explicitly listed documentation/license files if included. The verified archive therefore supplies the `current/scripts/` files as well as the native payload; normal updates do not need a separate unverified script download. Preserve `envswitch/` as the top directory only for the legacy source archive. Publish the installers with the release as well as keeping the stable README URLs. `SHA256SUMS` uses lowercase SHA-256, two spaces, and an exact basename per line; include every native archive, compatibility archive, and installer, but not the manifest itself. Compute after any platform signing/notarization-related packaging changes. `Get-FileHash -Algorithm SHA256`, `sha256sum`, or `shasum -a 256` are enough; no hashing crate is needed by this update design.

Installer verification must fail closed on a missing/malformed/duplicate selected entry, checksum mismatch, unexpected archive members, unsafe extraction paths/symlinks, or a wrong-target/wrong-version executable. Verify before executing the binary. Stage and extract only in private temporary storage. Resolve a release once, retain that tag/release asset list, and download the archive and manifest for **that same release**. Do not use repeated `latest/download` redirects for separate files. GitHub asset digests can supplement but do not replace the published contract.

An unsigned manifest from the same GitHub release catches corruption and mismatched assets. It does **not** protect against a compromised publisher who can replace both bytes and hashes, a malicious mutable bootstrap script, or rollback to another legitimate release. State that trust boundary plainly.

| Mechanism | What it supplies | Recommendation |
| --- | --- | --- |
| GitHub artifact attestations | Sigstore-backed provenance tied to repository/workflow identity and artifact digest; optional user verification with `gh attestation verify <archive> --repo rnldsalili/envswitch`, ideally constrained to the expected workflow/ref. | Add with `actions/attest` if repository plan supports it. Attest in the producing build job; attesting downloaded outputs only in the publish job proves that job handled them, not the full original build. Do not silently call an unsigned hash a signature. |
| Minisign 0.12 | Small detached signature scheme; pin a public verification key independently and sign `SHA256SUMS`. | Defer: requires secret-key custody/rotation and a trusted verifier bootstrap on both OS families. Worth revisiting if verification independent of GitHub is required. [Official release](https://github.com/jedisct1/minisign/releases/tag/0.12). |
| Cosign 3.1.4 | Keyed or keyless Sigstore blob signatures and bundles; keyless checks must constrain issuer and workflow identity. | Defer: more client/bootstrap surface than necessary alongside GitHub attestations. It is not a Windows Authenticode or Apple Developer ID signature. [Official release](https://github.com/sigstore/cosign/releases/tag/v3.1.4). |

GitHub now recommends **`actions/attest`** for new implementations; `actions/attest-build-provenance` v4 is a wrapper. Required provenance permissions are `id-token: write`, `attestations: write`, and `contents: read`. Public repositories on current plans support attestations; private/internal repositories require Enterprise Cloud. Do not infer this repository's visibility/plan from source files. Optional local verification must fail if the user explicitly requested it and verification fails; absence of `gh` should not silently masquerade as verified provenance. [GitHub attestation documentation](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations), [official action](https://github.com/actions/attest), [legacy wrapper notice](https://github.com/actions/attest-build-provenance)

**macOS:** Apple documents that Safari-style downloads acquire quarantine and Finder extraction propagates it; Unix `curl`/`tar` generally do not. Curl installation therefore often avoids the normal quarantined-download Gatekeeper path, but is not a supported guarantee of exemption: macOS can assess execution at other times. Apple Silicon also needs a valid code signature; preserve/verify the Rust linker's ad-hoc signature where applicable. Ad-hoc signing is not Developer ID trust or notarization. [Apple DTS explanation](https://developer.apple.com/forums/thread/706442), [Apple silicon signature requirement](https://developer.apple.com/documentation/Xcode/embedding-a-helper-tool-in-a-sandboxed-app?changes=_3_3)

For a smooth browser-download path, Developer ID sign the final Mach-O, enable the required hardened-runtime settings, submit a suitable zip/pkg/dmg with `xcrun notarytool`, and test actual distribution on a clean Mac. Raw command-line executables and zip files are not generally staplable like an app/pkg/dmg; online ticket lookup or a staplable signed installer package affects offline behavior. Do not promise that a `.tar.gz` alone carries a stapled ticket. Never automatically strip quarantine or disable Gatekeeper. For this initial internal CLI port, retain the shell install path and explicitly document that Developer ID/notarization is deferred until credentials and public-download support are in scope. [Apple notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow), [Apple notarization session: command-line tool stapling limit](https://developer.apple.com/videos/play/wwdc2019/703/?time=2009)

**Windows:** an unsigned/newly signed EXE can trigger SmartScreen, and enterprise policy or Smart App Control may block it outright. SHA-256, minisign, and GitHub attestations do not create Windows publisher reputation. Authenticode signing with a consistent trusted identity and timestamp improves the publisher signal but does not guarantee warnings disappear; EV certificates no longer provide an automatic SmartScreen bypass. Defer commercial code signing for the initial internal binary, disclose the limitation, and require it before claiming frictionless managed/public installation. Do not bypass policy or call `Unblock-File` automatically. [Microsoft's developer guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)

### B5. Replacement installers and persistent layout

Preserve these user-visible paths and overrides:

```text
Unix launcher:       $INSTALL_DIR/envswitch
Windows launcher:    <InstallDir>\envswitch.cmd
Application root:    same APP_ROOT / -AppRoot defaults
Current metadata:    <APP_ROOT>/current/.version
Native payloads:     <APP_ROOT>/current/bin/envswitch_<ver>_<target>[.exe]
Bundled installers:  <APP_ROOT>/current/scripts/install.{sh,ps1}
Install receipt:     <APP_ROOT>/current/install.json
```

The small receipt records schema version, install kind (`native`), repository, effective absolute app root, effective launcher directory/path(s), target triple, installed tag, and the managed native payload path. It contains no 1Password data or GitHub token. It solves the current missing-override problem; `.version` alone cannot. Reject inconsistent/out-of-root receipt paths before cleanup. An unmanaged copied binary may run, but `update` should explain how to install it properly instead of guessing writable paths or escalating privileges.

Keep Unix `REPO`, `INSTALL_DIR`, `APP_ROOT`, `XDG_DATA_HOME`, `GITHUB_TOKEN`, and profile-selection behavior; remove Bun bootstrap/discovery entirely from native installation. Keep Windows `-Repo`, `-InstallDir`, `-AppRoot`, `-WhatIf`, and the current default layout; preserve parameter precedence rather than silently inventing environment equivalents. Fix the unconditional `LOCALAPPDATA` failure when explicit locations suffice. Persist the effective settings and pass all of them explicitly on later updates. Keep the internal shell test hooks for the existing profile tests.

Add one narrow version pin, `ENVSWITCH_VERSION` on Unix and `-Version` on Windows, used by the bridge and update handoff to avoid the double-latest race. Unset means latest; accept a validated full tag or normalize a supported version to `v<version>` consistently. Pinned installs use `/releases/tags/<tag>`; ordinary installs use `/releases/latest`. No separate channel, custom URL, mirror, or arbitrary target override is needed.

Detection:

- Bash `uname -s`: Darwin → Apple; Linux → Linux, including WSL. `uname -m`: `arm64|aarch64` → aarch64; `x86_64|amd64` → x86_64. Reject 32-bit/unknown systems before downloading. On macOS under Rosetta, query the physical Apple Silicon capability and prefer arm64; do not blindly label the translated process as native Intel. Confirm that choice with tests.
- PowerShell: require Windows; inspect **OS** architecture using `RuntimeInformation.OSArchitecture` rather than process bitness/`PROCESSOR_ARCHITECTURE` alone. Select x86_64 MSVC for x64 Windows, including 32-bit PowerShell running on x64. Native Windows ARM is outside this five-target matrix; report unsupported rather than assuming emulation support.
- Linux always selects musl; no heuristic libc detector is needed for this matrix. Build portability and external `op` support remain separately tested constraints.

Download with the existing authenticated-`gh` preference, but make API fallback consistent between OSes. Select exact archive/manifest asset names from a validated release response; HTTP fallback uses asset API URLs with appropriate `Accept`/token handling, which supports private assets more reliably than the current Unix browser URL. Public direct URLs may be a fallback, always tied to the resolved tag. Do not forward authorization headers to arbitrary hosts or expose tokens in logs. Use bounded retries/timeouts for transient HTTP errors; fail promptly on auth/404/checksum failures. Avoid installing a JSON parser solely for Bash: keep the selected-release JSON handling constrained and validated, or use `gh`'s query facility when present; do not pretend a one-line `sed` expression is a general JSON parser.

Install transaction:

1. Validate repository/tag, architecture, effective destinations, permissions, required download/hash/extraction tools, and that cleanup destinations are owned envswitch paths. Reject filesystem roots, symlink escapes, relative roots, and app/launcher layouts that would overwrite one another. Correctly escape generated launchers; current Unix interpolation and Windows ASCII restrictions are not a safe implementation pattern.
2. Lock the app root to serialize installers; all code paths release the lock. Explicit `-WhatIf` validates and prints the plan but creates no directories, changes no PATH, runs no network/download, and launches no binary. Honor `ShouldProcess` consistently for committed PowerShell changes.
3. Stage downloads privately and verify selected hashes before extraction. Stage the final executable **on the destination filesystem** for the final rename; the system temporary directory may be a different mount.
4. Run the staged executable by absolute path with `--version` and `--help`; confirm expected version/target before touching the launcher. Never verify by a potentially shadowed command name. No 1Password sign-in or project `.env` write belongs in installer verification.
5. Place the payload at its immutable version-and-target filename. On Windows do not replace an existing active file at that path; matching verified bytes mean reuse, conflicting bytes mean fail. On Unix use rename from same-filesystem staging, not in-place truncation.
6. Build the replacement launcher beside its destination, then atomically replace the launcher file where the platform/filesystem permits. Keep rollback data through this step. A Windows sharing violation must leave the previous launcher valid and report retry guidance.
7. Atomically replace receipt and `.version` individually after successful payload/launcher verification. Several independent files cannot be committed as one atomic transaction; the launched binary's compiled version is authoritative, and reinstall repairs interrupted metadata updates. Retain the old payload until a later invocation so in-flight processes and rollback survive.
8. Update PATH with the existing platform policy, print success and guidance only after verification, and clean temporary files on all normal exits. Prune only known old payload/source files later; a Windows locked old executable can remain for a future cleanup. Never delete Bun itself, user projects, `.env` files, or an arbitrary overridden root.

Keep `info:` / `success:` prefixes and the exact existing PATH/restart/verification guidance in A5, including profile markers. Keep the same Unix choice of profile. Preserve one marker block and avoid duplicates; if a managed block points to a previous `INSTALL_DIR`, replace that block safely while preserving all surrounding user text. Windows continues a case-insensitive User PATH check; also repair the current installer process PATH independently when needed. Do not claim the parent terminal can be modified.

Native-specific output should say `info: binary: <absolute_payload_path>` instead of the obsolete `info: source: ...`, and `info: verifying envswitch launcher` should correspond to absolute-path execution. Retain `success: installed envswitch <tag>` and existing update success/up-to-date wording. Replace obsolete Bun/source messages and dry-run lines with native download/checksum/install steps; do not continue printing that production dependencies are installed.

Launcher contract:

- Unix keeps a tiny Bash launcher with strict mode and one safely quoted `exec <versioned-native-payload> "$@"`. No Bun lookup. It can carry the known effective launcher/app-root settings for the native process, but the receipt remains the durable configuration.
- Windows keeps `envswitch.cmd` for existing PATH and explicit-path consumers; it invokes the quoted native payload with `%*` and propagates the child status on a separate, non-parenthesized line. Test arguments, spaces, Unicode paths, `%`, `!`, `&`, and exit codes rather than copying the current batch block. Avoid introducing a competing `envswitch.exe` in the launcher directory that changes command resolution.

### B6. Migration that does not strand Bun users

**Invariant:** existing launchers, bundled installers, and users who skip releases must all retain a path to a working native command. Switching only the repository's `main/scripts/install.*` files is insufficient because A7 runs the installed script. Dropping the source asset immediately yields a deterministic “asset not found” failure for every untouched old updater.

**Stage 1 — final full TypeScript bridge release, before Rust becomes latest:**

1. Keep the current source asset and full TS behavior. Upgrade its bundled installers to recognize/install native releases while still installing a source release when that is the release type. Release metadata can explicitly distinguish the two; do not treat arbitrary download failures as permission to bypass checksum verification or fall back to unverified native files.
2. Fix installer invocation to preserve repository and both effective locations. New installs record these values. Copy/re-execute any installer outside `current` before replacing/deleting the tree it came from; an executing script must not depend on files it is removing.
3. Its `envswitch update` resolves one release, forwards that exact tag, and hands off to the native-capable installer when native assets exist. It waits for completion, then reads the unchanged `.version` location and reports old → new. Up-to-date checks must consider installation kind/payload health, not just equality of `.version`.
4. The new installer stages/validates native payloads and switches managed launchers as in B5. It retains a dependency-free compatibility `current/src/cli.ts` forwarding stub while legacy launcher coverage is uncertain. Persist receipts for subsequent updates.

**Stage 2 — first and every subsequent Rust release also publishes `envswitch_<ver>_source.tar.gz`:**

This is a small **compatibility bootstrap**, not a second TS implementation. It contains the `envswitch/` top directory, a valid dependency-free `package.json` and matching Bun lockfile, `src/cli.ts`, native-capable `scripts/install.sh` and `scripts/install.ps1`, and enough version/release metadata to pin bootstrap to its own release. Test `bun install --production --frozen-lockfile` on it. The old installer only requires the package and CLI paths and will accept it.

An untouched older `envswitch update` downloads this asset through its old installer, writes the new tag to `.version`, and rewrites its old-style Bun launcher. On Unix with PATH already present, the old installer's `--help` verification may immediately invoke the stub; otherwise the next command does. On Windows the next command does. The stub must:

- Run with the already-installed Bun and no application dependencies.
- Locate app root from its own file location, not the project CWD. Detect absence/unhealthiness of the native payload even if `.version` already equals latest.
- Copy a bundled native installer to temporary storage outside `current`, invoke it with the bootstrap's pinned release tag, known app root, and any recovered/persisted launcher/repository settings; ensure cleanup cannot remove an active parent script's required files.
- On success, invoke the native executable by absolute path, forwarding the original arguments, stdin/stdout/stderr, and exit status. An original `update` should complete migration before the native equal-version check can short-circuit. On failure, retain a runnable bootstrap and return a clear nonzero error; never delete the recovery stub.

The old update itself may initially report that the compatibility release is installed before native bootstrap occurs. Document this one-time lazy handoff; do not rely on Bun lifecycle hooks or assume an old installer will execute a new installer automatically.

**Custom launcher recovery is a real information gap:** historical installers never recorded arbitrary `INSTALL_DIR`/`-InstallDir`, and `Bun.argv` does not necessarily reveal the shell launcher that exec'd Bun. The current Windows updater also discards custom `-Repo`. No design can recover an unknown path or repository with certainty. The bridge should use explicit overrides and receipts first, inspect default/PATH candidate launchers only when their contents positively match this app root, and replace only positively identified managed launchers. Never scan/rewrite unrelated executables or silently claim all overrides were recovered.

Keep the tiny `current/src/cli.ts` forwarding stub so a non-discoverable old launcher still works through Bun. Print a one-time migration note with the explicit reinstall command/overrides to repair that launcher. Once a specific launcher is repaired, it no longer needs Bun; do not uninstall a shared Bun installation. This stub is the bounded compatibility exception to removing the source tree, and is necessary to avoid breaking otherwise undiscoverable absolute-path launchers. If the owner demands removal of every TS file, explicit confirmation of all launcher paths becomes a migration prerequisite; it cannot be achieved transparently from the current metadata.

**Source cleanup:** after native verification and launcher commit, remove old application TS modules, `node_modules`, package/tooling files, and replaced source content only from a positively identified installed tree. Keep the compatibility stub, current installers, receipt, native payloads, and `.version`. Defer cleanup when a legacy installer/TS ancestor is still using the tree; complete it on the next independent successful installer/update invocation, with pending cleanup recorded. Do not recursively remove `current` after placing native payloads there. Cleanup failures must not roll back a working native launcher or erase recovery paths. An old in-flight Bun process must be allowed to finish reading the marker and reporting success.

Retain the compatibility source asset on every latest native release for the supported lifetime of old installations; do not assume everyone installs the bridge in order. Its small storage/CI cost is preferable to a forced manual reinstall. The legacy first hop still has its original absence of checksum verification and requires working Bun. Later native bootstrap verifies hashes. This plan cannot retroactively secure or run an old installation whose Bun runtime was removed; the documented direct native installer is the recovery path.

**Marker semantics:** `.version` remains the full `v<version>` tag at `current/.version`, with a newline; readers accept the old encoding and trim whitespace. It continues to mean the selected installed release, but during compatibility bootstrap it does not prove native installation. The native compiled version plus receipt/payload checks distinguish `legacy/bridge/native`; do not change the marker to JSON or relocate it while old TS parents still read it.

### B7. Rust self-update alternatives

| Approach | Behavior / tradeoffs | Decision |
| --- | --- | --- |
| Run embedded installer | Rust writes its compile-time `include_str!` installer into private temporary storage and invokes Bash or Windows PowerShell with persisted repo/root/launcher settings. One implementation of detection, authenticated download, hash verification, migration, PATH handling, and commit logic. Keeps existing platform shell/tool prerequisites. | **Choose this.** The native executable needs no HTTP/archive/signature/update crate solely to update itself. The installer resolves/pins the release once and receives the native current version for skip decisions. |
| `self_update` 1.3.3 | Maintained Rust GitHub updater with archive/executable replacement support; checksum verification is an opt-in feature, signature support uses its own zipsign format. Can configure checksum-manifest handling and custom verification. It would still need envswitch receipt, launcher, legacy cleanup, and `.version` integration. | Do not add now. Good if shell-free self-update later becomes a requirement; assess the exact feature/dependency set and Windows replacement behavior then. [Versioned docs](https://docs.rs/self_update/1.3.3/self_update/), [versioned README](https://github.com/jaemk/self_update/blob/v1.3.3/README.md). |
| `axoupdater` 0.10.3 | Standalone updater or library using dist install receipts; reuses the dist installation ecosystem. Existing envswitch installs lack these receipts; dist labels its companion-updater integration experimental. | Do not create synthetic dist receipts solely to adopt this crate. Reconsider together with dist. [Versioned README](https://github.com/axodotdev/axoupdater/blob/v0.10.3/README.md), [API docs](https://docs.rs/axoupdater/0.10.3/axoupdater/). |
| Custom native downloader/replacer | Full control of HTTP/asset selection, verification, staging, rollback, and native Windows APIs. Requires implementing and testing all of those alongside first-install scripts, including trust/metadata policy. | More code and two update implementations; defer. |

The embedded installer is trusted as part of the installed executable. It downloads only release payloads/checksums; do not fetch and execute a fresh script from mutable `main` as the normal native update mechanism. Ship updated scripts in the verified release/compatibility payload for first install and future migration; keep a stable installer protocol so an older embedded installer can consume later releases. An explicit protocol-breaking release would need another compatibility bridge.

**Windows replacement:** Windows normally prevents overwriting/deleting a running executable. A conventional single-file updater can rename the loaded EXE to an old name, move the staged new EXE into the original name, and defer deleting the old file until the process exits (subject to sharing permissions/security tools); another option is a helper that waits for exit before replacement. Neither is equivalent to Unix overwrite. This design avoids the primary problem: immutable versioned payload files plus a small launcher switch, with deferred pruning of locked old payloads. Test concurrent invocations, sharing violations, interrupted metadata commits, and same-version reinstall; do not claim “atomic directory replacement” on Windows. [Reference implementation of rename/deferred deletion](https://github.com/mitsuhiko/self-replace/blob/main/src/windows.rs)

Rust `update` should use the running executable's compiled version as current, pass explicit recorded settings, return nonzero on failure, and print success only after the installer verifies the selected new payload/launcher. An equal `.version` with a missing binary/receipt is repair work, not “already up to date.” Preserve the current public command and output wording while improving those checks.

### B8. Verified versions and immutable action pins

Verified against official upstream releases, GitHub's release/tag APIs (annotated tags peeled to commits), crates.io metadata, and versioned documentation on **2026-11-16**. Search indexes lagged for dist and cargo-zigbuild; the upstream release/API values below take precedence. These are research-time selections; recheck advisories and runner compatibility when implementing. Alternatives are listed for comparison, not as dependencies to install.

| Tool/crate | Verified stable version | Upstream release date (UTC) / source |
| --- | --- | --- |
| Rust | `1.100.0` | 2026-11-12; [official announcement](https://blog.rust-lang.org/2026/11/12/Rust-1.100.0/). |
| rustup | `1.29.3` | Current stable from the [official rustup release manifest](https://static.rust-lang.org/rustup/release-stable.toml); GitHub does not publish rustup Releases through `/releases/latest`. Record the runner-provided version; the compiler pin remains separate. |
| GitHub CLI | `2.104.0` | 2026-11-12. [Official release](https://github.com/cli/cli/releases/tag/v2.104.0); use/verify this version for CI release and attestation commands if pinning the CLI. |
| Bun (bridge verification only) | `1.4.4` | 2026-10-22. [Official stable release](https://github.com/oven-sh/bun/releases/tag/bun-v1.4.4); also test the legacy baseline implied by the existing lockfile. Not a prerequisite for native installs. |
| dist / cargo-dist | `0.34.0` | Published 2026-11-02; release title dated November 1. [Release](https://github.com/axodotdev/cargo-dist/releases/tag/v0.34.0), [API](https://api.github.com/repos/axodotdev/cargo-dist/releases/latest). |
| cargo-zigbuild | `0.24.1` | 2026-11-04. [Release](https://github.com/rust-cross/cargo-zigbuild/releases/tag/v0.24.1), [API](https://api.github.com/repos/rust-cross/cargo-zigbuild/releases/latest). |
| cross | `0.2.5` | 2023-02-04; latest stable upstream release, not a recommendation to use unpinned main. [Release](https://github.com/cross-rs/cross/releases/tag/v0.2.5). |
| self_update | `1.3.3` | 2026-10-27; not yanked. [crates.io API](https://crates.io/api/v1/crates/self_update), [docs](https://docs.rs/self_update/1.3.3/self_update/). |
| axoupdater | `0.10.3` | 2026-10-21; not yanked. [crates.io API](https://crates.io/api/v1/crates/axoupdater), [docs](https://docs.rs/axoupdater/0.10.3/axoupdater/). |
| minisign | `0.12` | 2025-01-15. [Release](https://github.com/jedisct1/minisign/releases/tag/0.12). |
| cosign | `3.1.4` | 2026-10-20. [Release](https://github.com/sigstore/cosign/releases/tag/v3.1.4). |

Use these literal action references if implementing the proposed workflow now; each is a peeled 40-character commit, not a mutable major tag:

```yaml
uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
uses: actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6 # v4.2.2, optional
```

Sources: [checkout release](https://github.com/actions/checkout/releases/tag/v7.0.1), [upload release](https://github.com/actions/upload-artifact/releases/tag/v7.0.2), [download release](https://github.com/actions/download-artifact/releases/tag/v8.0.2), [attest release](https://github.com/actions/attest/releases/tag/v4.2.2). Tag-ref API pattern: `https://api.github.com/repos/<owner>/<repo>/git/ref/tags/<tag>`; peel tag objects before using a SHA. For example, [checkout tag ref](https://api.github.com/repos/actions/checkout/git/ref/tags/v7.0.1).

The selected actions use Node 24, so use current supported hosted runner agents; verify a self-hosted agent separately if one is introduced. Retain `persist-credentials: false`. Upload one unique artifact name per target and download only this workflow run's expected set; the Actions artifact-container digest is not the SHA-256 of the enclosed release archive. No Bun setup action is needed for native builds; retain the currently pinned Bun action only in legacy bridge verification while that compatibility path exists, and pin Bun 1.4.4 for the current bridge test lane and add a tested historical baseline rather than assuming every old Bun can read a newly generated lockfile.

### B9. Implementation scope and acceptance checks (not executed)

Future files/consumers to reach:

- Existing [scripts/install.sh][sh-defaults], [scripts/install.ps1][ps-defaults], [release.yml][release-trigger], [src/cli.ts][cli-update] for the final TS bridge, [test/install-sh.test.sh][sh-test], [test/cli.test.ts][cli-test], [README.md][readme-install], and [\.gitignore][gitignore]. Preserve package/lockfile compatibility for old source installers; do not delete it from the compatibility artifact just because the main implementation becomes Rust.
- New `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, target-specific build configuration if necessary, native version/update implementation, install receipt handling, and compatibility archive generator/stub. Pick exact Rust module/test filenames after the broader port layout is settled; no extra release framework or dependency wrapper is needed.

Required automated checks before publication, all with temporary app roots/launcher directories and isolated `HOME`, `XDG_DATA_HOME`, `USERPROFILE`, `LOCALAPPDATA`, PATH/profile state, and mocked HTTP/`gh`/Bun where applicable:

| Area | Acceptance evidence |
| --- | --- |
| Matrix | Execute the packaged binary's `--help`/`--version` on each CPU/OS; verify target/version and Linux static linkage; smoke-test Alpine, supported glibc floor/WSL, and declared macOS floor. |
| Detection | Darwin Intel/ARM/Rosetta, Linux ARM/x86/WSL, x64 Windows from 32-bit and 64-bit PowerShell; clear rejection of unsupported systems. |
| Integrity | Missing/duplicate/malformed checksum, changed payload, wrong tag/target, hostile archive entries, missing tool, private-asset API download, API denial/rate limit, interrupted download. Every failure leaves the previous command usable. |
| Install | Fresh install, same-version reinstall/repair, custom root and launcher directory, spaces/Unicode/metacharacters, readonly destination, cross-filesystem staging, interrupted commit, concurrent install, rollback, explicit `-WhatIf` no changes/network. |
| PATH | Existing/missing PATH, shadowing, each shell-profile selection, changed managed block, repeat install, Windows User/Machine/process PATH differences; exact preserved guidance. |
| Migration | Last full TS bridge → Rust and **unmodified pre-bridge** source installer → compatibility archive → Rust on both platforms; latest marker already equal but native absent; original arguments/exit status; custom launcher absent from PATH; legacy ancestor still running; cleanup does not remove project `.env` or Bun. |
| Windows update | Running native EXE, second concurrent native invocation, locked old payload, launcher sharing violation, failure between launcher and marker, deferred cleanup. |
| Release | All expected assets and hashes before latest promotion; consistent source SHA/version; failed target prevents public release; rerun no overwrites; older run cannot displace newer latest. |

Candidate commands for the future CI are `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked --target <target>`, `cargo build --release --locked --target <target>`, and isolated Bash/PowerShell installer/bridge suites. These commands were **not** run for this research.

### B10. Main risks and explicit limits

1. **Skipping the bridge:** removing the legacy source asset strands old updaters. Keep the tiny compatibility archive in every latest release.
2. **Lost custom settings:** old installers cannot reveal every historical launcher path/repository. Preserve the forwarding stub and require explicit overrides for unidentifiable custom installs; avoid claims of perfect automatic recovery.
3. **Windows locks / partial commits:** versioned payloads avoid overwriting a running EXE, but launcher and metadata writes still need rollback/repair and real Windows tests.
4. **Integrity vs. trust:** same-origin SHA-256 is mandatory integrity checking, not independent publisher authentication. The old migration hop and mutable first-install script retain their historical trust limits; optional attestations help only when verified.
5. **Platform trust/portability:** static musl must be tested with actual dependencies and external `op`; Gatekeeper, SmartScreen, Smart App Control, and enterprise policies remain separate from checksums. Commercial OS signing/notarization is deferred, not solved by using Rust.
6. **Automatic releases:** run-number versions require explicit Cargo/lockfile stamping and latest-order protection. Pinning actions/toolchain does not freeze runner images or imply reproducible bytes.

No installation, build, test suite, signing/notarization, account-plan check, or live Windows/macOS warning test was performed. Only this research file was authored; the proposed contracts need implementation and the acceptance evidence above.

## Repository source references

Links identify the inspected local files/starting lines. The commit at the top fixes the factual baseline; later edits can move line numbers.

[sh-defaults]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:3
[sh-logging]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:14
[sh-cleanup]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:30
[sh-validation]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:36
[sh-profile]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:57
[sh-profile-write]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:79
[sh-guidance]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:98
[sh-platform]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:127
[sh-bun-lookup]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:150
[sh-bun]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:164
[sh-latest]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:194
[sh-download]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:227
[sh-source]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:258
[sh-launcher]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:282
[sh-main]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:315
[sh-hooks]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:343
[ps-defaults]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:1
[ps-logging]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:11
[ps-platform]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:38
[ps-repo]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:48
[ps-headers]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:56
[ps-latest]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:70
[ps-bun]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:90
[ps-download]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:130
[ps-source]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:162
[ps-path]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:210
[ps-launcher]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:233
[ps-main]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:270
[ps-install]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:296
[ps-success]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:314
[release-trigger]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:3
[release-verify]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:13
[release-publish]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:35
[release-assets]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:48
[release-create]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:63
[package]: /Users/rnldsalili/Documents/career/tools/envswitch/package.json:1
[gitignore]: /Users/rnldsalili/Documents/career/tools/envswitch/.gitignore:1
[readme-install]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:9
[readme-gh]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:38
[readme-manual]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:61
[readme-assets]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:76
[readme-prereq]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:99
[readme-development]: /Users/rnldsalili/Documents/career/tools/envswitch/README.md:494
[cli-flags]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:97
[cli-progress]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:108
[cli-dispatch]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:172
[cli-errors]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:178
[cli-update]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:334
[cli-version]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:368
[cli-fetch]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:381
[cli-install]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:398
[cli-deps]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:459
[cli-help]: /Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:722
[sh-test]: /Users/rnldsalili/Documents/career/tools/envswitch/test/install-sh.test.sh:1
[cli-test]: /Users/rnldsalili/Documents/career/tools/envswitch/test/cli.test.ts:292
[cli-test-unknown]: /Users/rnldsalili/Documents/career/tools/envswitch/test/cli.test.ts:429

[sh-profile-62]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:62
[sh-main-324]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:324
[ps-main-285]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:285
[ps-install-307]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:307
[sh-source-275]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:275
[sh-main-328]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:328
[ps-source-202]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.ps1:202
[sh-main-337]: /Users/rnldsalili/Documents/career/tools/envswitch/scripts/install.sh:337
[release-verify-18]: /Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:18
