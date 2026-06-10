# envswitch

`envswitch` is a reusable Bun-powered CLI for switching local project `.env` files from 1Password. Run it from the root of the project you want to update.

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
envswitch --help   # show all commands
```

If `envswitch` is not found right after installing, restart your terminal (or run the `export PATH` command the installer prints).

## Installation Details

The installer installs Bun if it is missing, downloads the source release, installs it under your data directory, and creates an `envswitch` launcher on your `PATH`.

- macOS, Linux, WSL: source in `${XDG_DATA_HOME:-$HOME/.local/share}/envswitch/current`, launcher at `$HOME/.local/bin/envswitch`.
- Windows: source in `$env:LOCALAPPDATA\Programs\envswitch\current`, launcher at `$env:LOCALAPPDATA\Programs\envswitch\bin\envswitch.cmd`.

If you have the GitHub CLI and prefer it over `curl`:

```bash
gh api repos/rnldsalili/envswitch/contents/scripts/install.sh \
  --jq '.content' | base64 -d | bash
```

```powershell
$rawUrl = gh api repos/rnldsalili/envswitch/contents/scripts/install.ps1 --jq ".download_url"
irm $rawUrl | iex
```

Override the install locations with `INSTALL_DIR` (launcher) and `APP_ROOT` (source) on macOS/Linux, or `-InstallDir` and `-AppRoot` on Windows:

```bash
curl -fsSL https://raw.githubusercontent.com/rnldsalili/envswitch/main/scripts/install.sh |
  INSTALL_DIR="$HOME/.local/bin" APP_ROOT="$HOME/.local/share/envswitch" bash
```

```powershell
.\scripts\install.ps1 -InstallDir "$env:USERPROFILE\bin" -AppRoot "$env:LOCALAPPDATA\envswitch"
```

### Manual Installation

Download the source release archive, extract it, install dependencies with Bun, and create a small launcher on your `PATH`.

```bash
gh release download --repo rnldsalili/envswitch --pattern 'envswitch_*_source.tar.gz'
mkdir -p "$HOME/.local/share/envswitch"
tar -xzf envswitch_*_source.tar.gz -C "$HOME/.local/share/envswitch"
mv "$HOME/.local/share/envswitch/envswitch" "$HOME/.local/share/envswitch/current"
(cd "$HOME/.local/share/envswitch/current" && bun install --production --frozen-lockfile)
mkdir -p "$HOME/.local/bin"
printf '#!/usr/bin/env bash\nexec bun "%s/src/cli.ts" "$@"\n' "$HOME/.local/share/envswitch/current" > "$HOME/.local/bin/envswitch"
chmod +x "$HOME/.local/bin/envswitch"
```

Release assets use this naming format:

```text
envswitch_0.1.0_source.tar.gz
```

## Commands

```bash
envswitch
envswitch doctor --vault <vault> --config-item <item>
envswitch list --vault <vault> --config-item <item> targets
envswitch switch --vault <vault> --config-item <item> --client <client> --env local --dry-run
envswitch switch --vault <vault> --config-item <item> --client <client> --env local
envswitch update
```

Run `envswitch` with no arguments for guided switch prompts. Guided mode uses arrow-key selection menus for 1Password vaults, config items, clients, environments, targets, and dry-run/write mode. It only shows vaults and config items containing the `envswitch-config` tag, auto-selects single available choices, and defaults to writing `.env` files unless you choose dry-run.

Use `envswitch switch ...` for scripts and non-interactive terminals. `--env` defaults to `local`, and `--target` defaults to `all`.

Use `envswitch update` to update an installed copy to the latest GitHub release. It re-runs the bundled install script in place, skips the reinstall when you are already on the latest version, and reports the old and new versions. It does not work from a git checkout; use `git pull` there instead.

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

These commands are for contributors and release maintainers. Normal installation uses the source release from GitHub Releases.

```bash
bun test
bun run build
```

The build command runs the TypeScript check. Release packaging is handled by the GitHub Actions workflow.
