# envswitch

`envswitch` is a native Rust CLI for switching local project `.env` files from 1Password. Run it from the root of the project you want to update.

The CLI contains generic switching logic only. Project paths, clients, environments, target names, and tag rules are loaded from a 1Password config item.

## Quick Start

Install (macOS, Linux, WSL):

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh | bash
```

On Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 | iex
```

Then use it:

```bash
envswitch          # guided switch
envswitch update   # update to the latest release
envswitch --version # show version and target triple
envswitch --help   # show all commands
```

If `envswitch` is not found right after installing, restart your terminal (or run the `export PATH` command the installer prints).

## Installation Details

The installers download a native binary from GitHub Releases and require a matching `SHA256SUMS` manifest before installation. Bun is not required.

Supported release targets:

| Platform | Target | Archive |
| --- | --- | --- |
| macOS Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Linux / WSL x86_64 | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Linux / WSL ARM64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

Linux binaries are statically linked with musl. Windows ARM and 32-bit Windows are not supported.

On macOS/Linux/WSL, the installer needs Bash, `curl` or authenticated `gh`, `tar`, standard Unix tools, and `sha256sum` or `shasum`. The Unix launcher also uses Bash. Windows requires PowerShell 5.1 or later; SHA-256 verification uses `Get-FileHash`.

Default locations:

- macOS/Linux/WSL: `APP_ROOT` is `${XDG_DATA_HOME:-$HOME/.local/share}/envswitch`; `INSTALL_DIR` is `$HOME/.local/bin`.
- Windows: app root is `$env:LOCALAPPDATA\Programs\envswitch`; launcher directory is `<AppRoot>\bin`.

The native layout is:

```text
APP_ROOT/current/
  bin/envswitch_<ver>_<target>[.exe]  # versioned native payload
  scripts/install.sh
  scripts/install.ps1
  .version                          # v<ver> followed by a newline
  install.json                      # install receipt
INSTALL_DIR/envswitch               # Unix launcher
INSTALL_DIR/envswitch.cmd           # Windows launcher
```

The receipt records the repository, version, target, and managed paths for future updates. Older versioned binaries are retained. An upgrade from source also retains unused legacy source files; it does not remove Bun.

### Installer Overrides

On Unix, set `INSTALL_DIR` (launcher directory), `APP_ROOT` (application root), `REPO` (`owner/repo`, default `rnldsalili/envswitch`), or `ENVSWITCH_VERSION` (pin, `0.0.N` or `v0.0.N`). Set variables on `bash`, not `curl`:

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh |
  ENVSWITCH_VERSION=v0.0.42 INSTALL_DIR="$HOME/.local/bin" APP_ROOT="$HOME/.local/share/envswitch" bash
```

Use an existing release version in place of `v0.0.42`. Without a pin, the installer selects the latest release. Both installers support `GITHUB_TOKEN` for GitHub API authentication and use authenticated `gh` when available.

On Windows, download the script to pass parameters: `-InstallDir`, `-AppRoot`, `-Repo`, `-Version` (same pin format), and `-WhatIf` (preview without downloads or writes):

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1 -InstallDir "$env:USERPROFILE\bin" -AppRoot "$env:LOCALAPPDATA\envswitch" -Repo rnldsalili/envswitch -WhatIf
```

Remove `-WhatIf` to install. Use absolute installation paths.

### Verification and Manual Installation

Download the archive matching your platform and `SHA256SUMS` from the same GitHub release. Release assets use these names (`<ver>` has no leading `v`):

```text
envswitch_<ver>_aarch64-apple-darwin.tar.gz
envswitch_<ver>_x86_64-apple-darwin.tar.gz
envswitch_<ver>_x86_64-unknown-linux-musl.tar.gz
envswitch_<ver>_aarch64-unknown-linux-musl.tar.gz
envswitch_<ver>_x86_64-pc-windows-msvc.zip
SHA256SUMS
install.sh
install.ps1
```

For example, on Linux x86_64 (replace the version with an existing release):

```bash
gh release download v0.0.42 --repo rnldsalili/envswitch --pattern 'envswitch_0.0.42_x86_64-unknown-linux-musl.tar.gz' --pattern SHA256SUMS
sha256sum envswitch_0.0.42_x86_64-unknown-linux-musl.tar.gz
# On macOS: shasum -a 256 <archive>
```

Compare the complete hash with the entry for that exact archive filename in `SHA256SUMS`; stop if it is missing or differs. On Windows, use `Get-FileHash -Algorithm SHA256 <archive>` and compare the hash with the matching manifest entry (hexadecimal case does not matter).

After verification, extract with `tar -xzf <archive>` on Unix or `Expand-Archive <archive>` on Windows. Place the extracted `envswitch` (or `envswitch.exe`) binary in a directory on your `PATH`; on Unix, ensure it is executable with `chmod +x <path-to-binary>`. Check it with `envswitch --version`.

Manual copies do not have an installer receipt. `envswitch update` requires an installer-managed installation; manual copies receive reinstall guidance instead.

Both installers can additionally require GitHub attestation verification using `gh`: set `ENVSWITCH_VERIFY_ATTESTATIONS=1` on the Unix `bash` invocation, or pass `-VerifyAttestations` to the downloaded PowerShell script. A missing verifier or failed attestation aborts installation. To verify a downloaded archive yourself:

```bash
gh attestation verify <archive> --repo rnldsalili/envswitch
```

SHA-256 verification is mandatory even with attestations. The public install one-liners execute a mutable bootstrap script; payload checksums do not verify that script before execution.

## Upgrading from the Bun Version

Upgrading from the Bun version: run `envswitch update` during the bridge release to prepare for native updates. If you skipped the bridge and update fails with `release asset not found or unavailable` or `matching asset not found`, rerun the install command below. Updates preserve your install paths when they are recorded in an install receipt. When reinstalling, supply any custom install paths explicitly. Your project `.env` files and Bun installation are not removed.

macOS, Linux, or WSL:

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 | iex
```

For an older custom installation, use the same app root, launcher directory, and repository you originally selected. The installer cannot discover every old custom launcher. Restart your terminal after reinstalling, then run `envswitch --version`; a native installation prints `envswitch <version> <target-triple>`. Reinstalling at the default location does not repair a different launcher that appears earlier on PATH.

Custom Unix example (replace these locations/repo with the actual old values; set variables on **bash**, not curl):

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh |
  REPO=rnldsalili/envswitch APP_ROOT="$HOME/.local/share/envswitch" INSTALL_DIR="$HOME/.local/bin" bash
```

Custom Windows example (download the script, then pass parameters):

```powershell
$installer = Join-Path $env:TEMP ("envswitch-install-" + [guid]::NewGuid() + ".ps1")
try {
    Invoke-WebRequest https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.ps1 -OutFile $installer
    & powershell -NoProfile -ExecutionPolicy Bypass -File $installer -Repo rnldsalili/envswitch -AppRoot "$env:LOCALAPPDATA\Programs\envswitch" -InstallDir "$env:LOCALAPPDATA\Programs\envswitch\bin"
} finally {
    Remove-Item -LiteralPath $installer -ErrorAction SilentlyContinue
}
```

Running `envswitch update` from the final TypeScript bridge release migrates an installer-managed installation to the native release automatically.

## Commands

```bash
envswitch
envswitch doctor --vault <vault> --config-item <item>
envswitch list --vault <vault> --config-item <item> targets
envswitch switch --vault <vault> --config-item <item> --client <client> --env local --dry-run
envswitch switch --vault <vault> --config-item <item> --client <client> --env local
envswitch update
envswitch --version
```

Run `envswitch` with no arguments for guided switch prompts. Guided mode uses arrow-key selection menus for 1Password vaults, config items, clients, environments, targets, and dry-run/write mode. It only shows vaults and config items containing the `envswitch-config` tag, auto-selects single available choices, and defaults to writing `.env` files unless you choose dry-run. Press Esc to cancel a prompt (exit status 130).

Use `envswitch switch ...` for scripts and non-interactive terminals. `--env` defaults to `local`, and `--target` defaults to `all`.

Use `envswitch --version` to print `envswitch <version> <target-triple>`.

`envswitch update` always runs the embedded installer, even at the same version, so it can check health and repair missing payloads, scripts, or metadata. It uses the repository and destinations saved in the receipt. Conflicting existing versioned binaries require manual recovery rather than automatic replacement. Source checkouts should use `git pull`; copies without a valid receipt receive reinstall guidance.

Update prints progress on stderr (`envswitch [info] updating envswitch <old> -> latest`) and leaves installer output visible. After verified success, it prints one summary:

```text
envswitch is already up to date (<new>)
updated envswitch <old> -> <new>
repaired envswitch <new>
```

Successful updates, repairs, and unchanged installs exit 0. Installer failures forward the installer's exit status (Unix signals use `128 + signal`); controlled update validation failures exit 1. No success summary follows failure.

## Prerequisites

Install and sign in to the 1Password CLI:

```bash
op whoami
```

If that fails, sign in with your normal 1Password flow before testing `envswitch`.

Choose the vault where the config item and env items will live:

```bash
op vault list
```

## 1Password Item Model

`envswitch` expects two kinds of 1Password items:

1. A config item with a field named `config` and the tag `envswitch-config`.
2. Env layer items whose fields are environment variables and whose tags describe where those variables apply.

Do not store env values or secrets in the config item. Put them in tagged env layer items instead.

## 1Password Config Item

Create a 1Password item with a field named `config`. The field value must be JSON. Add the `envswitch-config` tag so guided mode can find it.

Generic example:

```json
{
  "project": {
    "id": "project-id",
    "rootMarkers": [
      { "packageJsonName": "project-package-name" },
      { "path": "apps" }
    ]
  },
  "clients": ["client-a", "client-b"],
  "environments": ["local"],
  "targets": [
    {
      "name": "api",
      "output": "services/api/.env",
      "groups": ["apis"]
    },
    {
      "name": "web",
      "output": "services/web/.env",
      "groups": ["apps"]
    }
  ],
  "tags": {
    "base": ["envswitch"],
    "project": "project:{project}",
    "env": "env:{env}",
    "client": "client:{client}",
    "target": "target:{target}",
    "group": "group:{group}",
    "layer": "layer:{layer}"
  }
}
```

Important rules:

- `project.id` is used to build the `project:<project-id>` tag.
- `rootMarkers` validate that the user is running from the correct project root.
- `targets[].output` must be relative to the project root. Absolute paths and paths that escape the project root are rejected.
- `targets[].groups` controls shared group layers such as `apis` and `apps`.

You can create the item from the 1Password app, or use the CLI. The app is usually easier for pasting a large JSON field.

### Larger Config Example

```json
{
  "project": {
    "id": "demo-suite",
    "rootMarkers": [
      { "packageJsonName": "demo-suite" },
      { "path": "services" }
    ]
  },
  "clients": ["alpha", "beta"],
  "environments": ["local"],
  "targets": [
    {
      "name": "api",
      "output": "services/api/.env",
      "groups": ["apis"]
    },
    {
      "name": "web",
      "output": "apps/web/.env",
      "groups": ["apps"]
    },
    {
      "name": "worker",
      "output": "workers/sync/.env",
      "groups": ["workers"]
    }
  ],
  "tags": {
    "base": ["envswitch"],
    "project": "project:{project}",
    "env": "env:{env}",
    "client": "client:{client}",
    "target": "target:{target}",
    "group": "group:{group}",
    "layer": "layer:{layer}"
  }
}
```

## 1Password Env Items

Create separate 1Password items for each layer. Store env vars as item fields.

Recommended item naming convention:

```text
envswitch/<project-id>/<env>/<layer-description>
```

Example item names:

```text
envswitch/demo-suite/local/shared
envswitch/demo-suite/local/apis
envswitch/demo-suite/local/apps
envswitch/demo-suite/local/api
envswitch/demo-suite/local/alpha
envswitch/demo-suite/local/alpha-api
```

The names are for humans. `envswitch` selects env items by tags.

## Tag Model

Every env item should include these base tags:

```text
envswitch
project:<project-id>
env:local
layer:<layer-name>
```

Supported layer names:

```text
shared
group
target
client
client-target
```

Add these tags only when the layer needs them:

```text
client:<client-name>
target:<target-name>
group:<group-name>
```

The merge order for each target is:

1. `layer:shared` with no `client:*`, `target:*`, or `group:*` tag
2. `layer:group` with `group:<group-name>` and no `client:*` or `target:*` tag
3. `layer:target` with `target:<target-name>` and no `client:*` or `group:*` tag
4. `layer:client` with `client:<client-name>` and no `target:*` tag
5. `layer:client-target` with both `client:<client-name>` and `group:<group-name>`

Variables inside a target-tagged item are written only to that configured target. Variables inside a group-tagged item are written only to targets in that configured group. Variables inside a client-target item are written only to the selected client and target group.

Items tagged for another target or group are ignored, so each service only receives its own service-specific and group-specific variables plus shared variables. Use `layer:target` for all-client service values and `layer:client-target` for client plus API/app group values.

### Shared Layer

Use this for values that every target and every client should receive.

Tags:

```text
envswitch
project:demo-suite
env:local
layer:shared
```

Example fields:

```text
NODE_ENV=development
AWS_REGION=us-east-1
LOG_LEVEL=debug
```

### Group Shared Layer

Use this for values that all targets in one configured group should receive, such as every API target or every app target.

Tags:

```text
envswitch
project:demo-suite
env:local
layer:group
group:apis
```

Example fields:

```text
API_SHARED_TIMEOUT_MS=30000
API_RATE_LIMIT=100
```

Use `group:apps` on a separate `layer:group` item for variables shared by app targets.

### Target Shared Layer

Use this for values that all clients share for one service target.

Tags:

```text
envswitch
project:demo-suite
env:local
layer:target
target:api
```

Example fields:

```text
PUBLIC_API_URL=http://localhost:3001
API_SERVICE_BASE_URL=http://localhost:3001
```

### Client Shared Layer

Use this for client values that every service target needs.

Tags:

```text
envswitch
project:demo-suite
env:local
layer:client
client:alpha
```

Example fields:

```text
CLIENT_ID=alpha
DATABASE_NAME=...
STORAGE_BUCKET=...
```

### Client Target Layer

Use this for one client's values for one target group, such as all API targets or all app targets.

Tags:

```text
envswitch
project:demo-suite
env:local
layer:client-target
client:alpha
group:apis
```

Example fields:

```text
SESSION_SECRET=...
API_TOKEN_SECRET=...
API_ONLY_VALUE=...
```

Use `group:apps` on a separate `layer:client-target` item for app-specific values for the same client.

## CLI Creation Examples

The exact fields depend on your 1Password item category and team conventions. A simple pattern is to create a secure note in the 1Password app, then add fields and tags there.

To verify tags from the CLI:

```bash
op item list --vault "<vault>" --tags envswitch,project:demo-suite,env:local --format json
```

To inspect one item:

```bash
op item get "<item-name-or-id>" --vault "<vault>" --format json --reveal
```

## Validate with envswitch

From the target project root:

```bash
cd /path/to/project

envswitch
```

Guided mode defaults to writing `.env` files. For scriptable checks or previews, use explicit commands:

```bash
envswitch doctor \
  --vault "<vault>" \
  --config-item "<config-item-name>"
```

List configured targets:

```bash
envswitch list \
  --vault "<vault>" \
  --config-item "<config-item-name>" \
  targets
```

Dry run before writing files:

```bash
envswitch switch \
  --vault "<vault>" \
  --config-item "<config-item-name>" \
  --client alpha \
  --env local \
  --dry-run
```

Run for one target only:

```bash
envswitch switch \
  --vault "<vault>" \
  --config-item "<config-item-name>" \
  --client alpha \
  --env local \
  --target api \
  --dry-run
```

After the dry run looks correct, remove `--dry-run`.

## Checklist

- Config item has a non-empty field named `config`.
- Config item has the `envswitch-config` tag for guided discovery.
- Config JSON includes all target outputs relative to the repo root.
- Targets that should share API or app values include `groups`, such as `apis` or `apps`.
- Every env item has `envswitch`, `project:<project-id>`, `env:local`, and exactly one `layer:*` tag.
- Group-specific items have exactly one `group:<group-name>` tag and no `client:*` or `target:*` tag.
- Target-specific shared items use `layer:target` with exactly one `target:<target-name>` tag and no `client:*` or `group:*` tag.
- Client group-specific items use `layer:client-target` with exactly one `client:<client-name>` tag and exactly one `group:<group-name>` tag.
- Client-specific items have exactly one `client:<client-name>` tag.
- Shared items do not have `client:*`, `target:*`, or `group:*` tags.
- `doctor` passes from the project root.
- `switch --dry-run` shows the expected layers for each target.

## Troubleshooting

If `doctor` fails authentication, run:

```bash
op whoami
```

If no layers are found for a target, check that the item has all required selector tags:

```text
envswitch
project:<project-id>
env:local
layer:<layer-name>
```

If a target receives the wrong variables, check for incorrect `client:*` or `group:*` tags. Native 1Password tags apply to the whole item, not individual fields, so split variables into separate items when only some keys apply to a target group.

## Development

The native crate is `crates/envswitch`. From the repository root:

```bash
cargo build
cargo test --locked
cargo test --locked --test parity -- --ignored
bash test/install-sh.test.sh
```

See [parity/README.md](parity/README.md) for parity setup, the frozen Bun oracle used only for development, and platform requirements. Linux update parity requires a packaged musl target as described there. The Bash installer suite uses isolated synthetic releases.
