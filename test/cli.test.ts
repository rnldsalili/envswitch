import { afterAll, describe, expect, test } from "bun:test";
import type { Subprocess } from "bun";
import { createHash } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import {
  buildInstallerInvocation,
  fetchLatestReleaseTag,
  formatProgressEvent,
  isManagedLauncher,
  main,
  parseArgs,
  parseInstallerOutcome,
  parseInstallReceipt,
  runInstallScript,
  type InstallerOutcome,
  type InstallerRun,
  type InstallPlan,
  type InstallReceipt,
  type UpdateEnv,
} from "../src/cli.ts";
import { EnvswitchError } from "../src/errors.ts";

describe("cli progress formatter", () => {
  test("formats plain progress output without ANSI colors", () => {
    expect(formatProgressEvent(
      { status: "info", message: "reading config item demo-envswitch" },
      { color: false },
    )).toBe("envswitch [info] reading config item demo-envswitch");
  });

  test("formats colored status labels when color is enabled", () => {
    expect(formatProgressEvent(
      { status: "done", message: "found 5 candidate item(s)" },
      { color: true },
    )).toBe("\x1b[2menvswitch\x1b[0m \x1b[32m[done]\x1b[0m found 5 candidate item(s)");

    expect(formatProgressEvent(
      { status: "dry-run", message: "would write apps/api/.env" },
      { color: true },
    )).toBe("\x1b[2menvswitch\x1b[0m \x1b[33m[dry-run]\x1b[0m would write apps/api/.env");
  });

  test("rejects unknown flags before command execution", async () => {
    const result = await runCli(["switch", "--unknown"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --unknown"]);
  });

  test("rejects boolean flags passed with values", async () => {
    const result = await runCli(["switch", "--dry-run=false"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: --dry-run does not take a value"]);
  });

  test("uses command-specific flag allowlists", async () => {
    const result = await runCli(["list", "--client", "alpha"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --client"]);
  });

  test("parses omitted optional switch flags so defaults can apply", () => {
    expect(parseArgs(["switch", "--vault", "vault", "--config-item", "cfg", "--client", "alpha"])).toEqual({
      command: "switch",
      positional: [],
      flags: {
        vault: "vault",
        "config-item": "cfg",
        client: "alpha",
      },
    });
  });

  test("runs a guided switch when no command is provided", async () => {
    const prompt = new FakePrompt(["beta", "dev", "api", true]);
    const output: string[] = [];
    const logs: string[] = [];
    const clientFactory = new FakeClientFactory();

    const result = await runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [
        { id: "vault-a", name: "Vault A" },
        { id: "vault-b", name: "Vault B" },
      ],
      createOnePasswordClient: (vault) => clientFactory.create(vault),
      log: (event) => logs.push(`${event.status}:${event.message}`),
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(clientFactory.vaults).toEqual(["vault-a", "vault-b"]);
    expect(clientFactory.allTagCalls()).toContainEqual(["envswitch-config"]);
    expect(clientFactory.allTagCalls()).toContainEqual(["envswitch", "project:demo", "env:dev"]);
    expect(prompt.selects.map((select) => select.message)).toEqual([
      "Client",
      "Environment",
      "Target",
      "Mode",
    ]);
    expect(prompt.selects[0]?.choices).toEqual([
      { label: "alpha", value: "alpha" },
      { label: "beta", value: "beta" },
    ]);
    expect(prompt.selects[1]?.defaultValue).toBe("local");
    expect(prompt.selects[2]?.defaultValue).toBe("all");
    expect(prompt.selects[3]?.choices).toEqual([
      { label: "Dry run", value: true },
      { label: "Write .env files", value: false },
    ]);
    expect(output).toContain("envswitch guided switch");
    expect(output.some((line) => line.startsWith("Dry run 1 target(s) from envswitch-project"))).toBe(true);
    expect(output).toContain("  layers: client:beta-client");
    expect(logs).toContain("info:reading config item demo config");
    expect(clientFactory.clientFor("vault-a")?.readConfigItemIds).toEqual(["cfg"]);
    expect(logs).toContain("info:checking vaults for config items tagged envswitch-config");
  });

  test("skips inaccessible vaults during guided discovery", async () => {
    const prompt = new FakePrompt(["alpha", "local", "all"]);
    const logs: string[] = [];
    const result = await runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [
        { id: "vault-bad", name: "Bad Vault" },
        { id: "vault-a", name: "Vault A" },
      ],
      createOnePasswordClient: (vault) => new FakeClient(vault, undefined, undefined, {
        failConfigList: vault === "vault-bad",
      }),
      log: (event) => logs.push(`${event.status}:${event.message}`),
      writeLine: () => {},
    });

    expect(result.code).toBe(0);
    expect(logs).toContain("info:skipping vault Bad Vault (could not list items)");
  });

  test("scans config-tagged vault items in parallel", async () => {
    const prompt = new FakePrompt(["alpha", "local", "all"]);
    const scan = new DeferredVaultScan();
    const resultPromise = runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [
        { id: "vault-a", name: "Vault A" },
        { id: "vault-b", name: "Vault B" },
      ],
      createOnePasswordClient: (vault) => new FakeClient(vault, undefined, undefined, {
        deferredScan: scan,
      }),
      log: () => {},
      writeLine: () => {},
    });

    await scan.waitForStarted(2);
    expect(scan.started).toEqual(["vault-a", "vault-b"]);
    scan.resolve("vault-a", [{ id: "cfg", title: "demo config", tags: ["envswitch-config"] }]);
    scan.resolve("vault-b", []);

    const result = await resultPromise;
    expect(result.code).toBe(0);
  });

  test("uses write mode as the guided default", async () => {
    const prompt = new FakePrompt(["alpha", "local", "all"]);
    const output: string[] = [];
    const result = await runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [{ id: "vault-a", name: "Vault A" }],
      createOnePasswordClient: (vault) => new FakeClient(vault),
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(prompt.selects.at(-1)?.message).toBe("Mode");
    expect(prompt.selects.at(-1)?.defaultValue).toBe(false);
    expect(output.some((line) => line.startsWith("Updated 2 target(s)"))).toBe(true);
  });

  test("falls back to input when config omits clients or environments", async () => {
    const prompt = new FakePrompt([true], ["alpha", "local"]);
    const output: string[] = [];
    const result = await runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [{ id: "vault-a", name: "Vault A" }],
      createOnePasswordClient: (vault) => new FakeClient(vault, {
        project: { id: "demo" },
        targets: [{ name: "api", output: "services/api/.env", groups: ["apis"] }],
        tags: { base: ["envswitch"] },
      }),
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(prompt.selects.map((select) => select.message)).toEqual(["Mode"]);
    expect(prompt.inputs).toEqual([
      { message: "Client", defaultValue: undefined, required: true },
      { message: "Environment", defaultValue: "local", required: true },
    ]);
    expect(output.some((line) => line.startsWith("Dry run 1 target(s)"))).toBe(true);
  });

  test("shows select prompts when guided choices have multiple options", async () => {
    const prompt = new FakePrompt(["vault-b", "cfg-b", "alpha", "local", "all"]);
    const output: string[] = [];
    const result = await runCli([], {
      prompt,
      isInteractive: true,
      root: "/tmp/envswitch-project",
      listVaults: async () => [
        { id: "vault-a", name: "Vault A" },
        { id: "vault-b", name: "Vault B" },
      ],
      createOnePasswordClient: (vault) => new FakeClient(vault, {
        project: { id: "demo" },
        clients: ["alpha", "beta"],
        environments: ["local", "dev"],
        targets: [
          { name: "api", output: "services/api/.env", groups: ["apis"] },
          { name: "web", output: "apps/web/.env", groups: ["apps"] },
        ],
        tags: { base: ["envswitch"] },
      }, [
        { id: `cfg-${vault.slice(-1)}`, title: `${vault} config`, tags: ["envswitch-config"] },
        { id: `cfg-${vault.slice(-1)}-other`, title: `${vault} other config`, tags: ["envswitch-config"] },
      ]),
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(prompt.selects.map((select) => select.message)).toEqual([
      "Vault",
      "Config item",
      "Client",
      "Environment",
      "Target",
      "Mode",
    ]);
    expect(prompt.selects[0]?.choices).toEqual([
      { label: "Vault A (vault-a)", value: "vault-a" },
      { label: "Vault B (vault-b)", value: "vault-b" },
    ]);
    expect(prompt.selects[1]?.choices).toEqual([
      { label: "vault-b config (cfg-b)", value: "cfg-b" },
      { label: "vault-b other config (cfg-b-other)", value: "cfg-b-other" },
    ]);
    expect(output.some((line) => line.startsWith("Updated 2 target(s)"))).toBe(true);
  });

  test("rejects guided mode when no vault has a config-tagged item", async () => {
    const result = await runCli([], {
      prompt: new FakePrompt([]),
      isInteractive: true,
      listVaults: async () => [{ id: "vault-a", name: "Vault A" }],
      createOnePasswordClient: (vault) => new FakeClient(vault, undefined, []),
      log: () => {},
    });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual([
      "envswitch: no 1Password vaults contain items tagged envswitch-config",
    ]);
  });

  test("returns 130 without an error line when the user cancels a prompt", async () => {
    const result = await runCli([], {
      prompt: new CancelPrompt(),
      isInteractive: true,
      listVaults: async () => [
        { id: "vault-a", name: "Vault A" },
        { id: "vault-b", name: "Vault B" },
      ],
      createOnePasswordClient: (vault) => new FakeClient(vault, undefined, [
        { id: `cfg-${vault.slice(-1)}`, title: `${vault} config`, tags: ["envswitch-config"] },
      ]),
      log: () => {},
    });

    expect(result.code).toBe(130);
    expect(result.stderr).toEqual([]);
  });

  test("rejects guided mode without an interactive terminal", async () => {
    const result = await runCli([], { isInteractive: false });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual([
      "envswitch: plain envswitch requires an interactive terminal; use envswitch switch --vault <vault> --config-item <item> --client <client> for scripts",
    ]);
  });
});

// ---------------------------------------------------------------------------
// update: final TypeScript bridge (INSTALLER-PROTOCOL.md §7.1, §7.3, §7.4)
//
// Sandbox rule for every process test below: a fresh realpath'd temp root
// with its own HOME/XDG/USERPROFILE/TMPDIR, a fresh PATH directory holding
// only symlinks to the needed host tools plus suite-owned fakes created with
// exclusive-create (never written through a link), and link-safe cleanup.
// ---------------------------------------------------------------------------

const WINDOWS = process.platform === "win32";
const REPO_ROOT = resolve(import.meta.dir, "..");
const DEFAULT_REPO = "rnldsalili/envswitch";
const UNIX_TARGET = "x86_64-unknown-linux-musl";
const RECEIPT_ERROR =
  "envswitch: cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README";
const LAUNCHER_DIR_ERROR =
  "envswitch: cannot recover this installation's launcher directory; reinstall envswitch with explicit APP_ROOT and INSTALL_DIR (PowerShell: -AppRoot and -InstallDir) using the README instructions";
const POSTCONDITION_ERROR =
  "envswitch: update failed: installer result does not match the installed receipt and version marker; reinstall envswitch with the install script from the README";

const tempRoots: string[] = [];
afterAll(() => {
  for (const root of tempRoots.splice(0)) {
    try {
      removeTree(root);
    } catch (error) {
      console.error(`warning: could not remove test directory ${root}: ${String(error)}`);
    }
  }
});

function realTempDir(prefix: string): string {
  // The native form expands Windows 8.3 short names (RUNNER~1) into the long path.
  const dir = realpathSync.native(mkdtempSync(join(tmpdir(), prefix)));
  tempRoots.push(dir);
  return dir;
}

/** Remove a suite-owned tree without ever following a symbolic link or junction. */
function removeTree(path: string): void {
  let stat;
  try {
    stat = lstatSync(path);
  } catch {
    return;
  }
  if (stat.isSymbolicLink()) {
    try {
      unlinkSync(path);
    } catch {
      rmdirSync(path); // Windows directory links and junctions
    }
  } else if (stat.isDirectory()) {
    for (const name of readdirSync(path)) {
      removeTree(join(path, name));
    }
    rmdirSync(path);
  } else {
    unlinkSync(path);
  }
}

/** Exactly what the pre-bridge installer's write_launcher heredoc produces. */
function legacyLauncher(appDir: string): string {
  return `#!/usr/bin/env bash
set -euo pipefail

if command -v bun >/dev/null 2>&1; then
  BUN_BIN="$(command -v bun)"
elif [[ -x "\${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="\${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\\n' >&2
  exit 127
fi

exec "$BUN_BIN" "${appDir}/src/cli.ts" "$@"
`;
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

/** Protocol §4.2 Unix launchers. */
function sourceLauncher(cli: string): string {
  return `#!/usr/bin/env bash
set -euo pipefail
if command -v bun >/dev/null 2>&1; then
  BUN_BIN="$(command -v bun)"
elif [[ -x "\${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="\${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\\n' >&2
  exit 127
fi
exec "$BUN_BIN" ${shellQuote(cli)} "$@"
`;
}

function nativeLauncher(payload: string): string {
  return `#!/usr/bin/env bash\nset -euo pipefail\nexec ${shellQuote(payload)} "$@"\n`;
}

function writeExecutable(path: string, content: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content, { flag: "wx", mode: 0o755 });
}

interface InstalledCopy {
  root: string;
  home: string;
  appRoot: string;
  appDir: string;
  binDir: string;
  installer: string;
  env: UpdateEnv;
}

function makeInstalledCopy(
  options: { version?: string; installer?: boolean; gitCheckout?: boolean; launcher?: "legacy" | "none" } = {},
): InstalledCopy {
  const root = realTempDir("envswitch-update-");
  const home = join(root, "home");
  const appRoot = join(home, ".local", "share", "envswitch");
  const appDir = join(appRoot, "current");
  const binDir = join(home, ".local", "bin");
  const installer = join(appDir, "scripts", "install.sh");
  mkdirSync(appDir, { recursive: true });
  mkdirSync(join(root, "empty-path"));
  if (options.installer !== false) {
    writeExecutable(installer, "#!/usr/bin/env bash\nexit 99\n");
  }
  if (options.version !== undefined) {
    writeFileSync(join(appDir, ".version"), options.version);
  }
  if (options.gitCheckout) {
    mkdirSync(join(appDir, ".git"));
  }
  if ((options.launcher ?? "legacy") === "legacy") {
    writeExecutable(join(binDir, "envswitch"), legacyLauncher(appDir));
  }
  return { root, home, appRoot, appDir, binDir, installer, env: { HOME: home, PATH: join(root, "empty-path") } };
}

/** Simulate the installer's committed state for a plan. */
function commitInstall(
  plan: Pick<InstallPlan, "appRoot" | "installDir" | "repo" | "version">,
  options: {
    kind?: "native" | "source";
    tag?: string;
    repo?: string;
    installDir?: string;
    marker?: string;
    payload?: boolean;
    launcher?: boolean;
  } = {},
): InstallReceipt {
  const kind = options.kind ?? "native";
  const tag = options.tag ?? plan.version;
  const installDir = options.installDir ?? plan.installDir;
  const target = kind === "native" ? UNIX_TARGET : null;
  const payloadPath =
    kind === "native"
      ? join(plan.appRoot, "current", "bin", `envswitch_${tag.slice(1)}_${UNIX_TARGET}`)
      : join(plan.appRoot, "current", "src", "cli.ts");
  const receipt: InstallReceipt = {
    kind,
    repo: options.repo ?? plan.repo,
    appRoot: plan.appRoot,
    installDir,
    launcherPath: join(installDir, "envswitch"),
    target,
    tag,
    payloadPath,
  };
  mkdirSync(dirname(payloadPath), { recursive: true });
  if (options.payload !== false) {
    writeFileSync(payloadPath, "payload\n");
  }
  mkdirSync(installDir, { recursive: true });
  if (options.launcher !== false) {
    writeFileSync(receipt.launcherPath, kind === "native" ? nativeLauncher(payloadPath) : sourceLauncher(payloadPath));
  }
  writeFileSync(join(plan.appRoot, "current", "install.json"), `${JSON.stringify({ schemaVersion: 1, ...receipt })}\n`);
  writeFileSync(join(plan.appRoot, "current", ".version"), options.marker ?? `${tag}\n`);
  return receipt;
}

/** The dual-mode installer's stdout for a result (§6.1, §6.2). */
function installerStdout(receipt: InstallReceipt, outcome: InstallerOutcome, onPath = true): string {
  const lines = ["info: detected Linux", `info: resolved envswitch ${receipt.tag} (${receipt.kind})`];
  if (outcome === "unchanged") {
    return `${[...lines, `success: envswitch is already up to date (${receipt.tag})`].join("\n")}\n`;
  }
  lines.push(
    `success: ${outcome} envswitch ${receipt.tag}`,
    receipt.kind === "native" ? `info: binary: ${receipt.payloadPath}` : `info: source: ${join(receipt.appRoot, "current")}`,
    `info: launcher: ${receipt.launcherPath}`,
  );
  if (onPath) {
    lines.push("info: run 'envswitch --help' to verify the installation");
  } else {
    lines.push(
      "info: restart your terminal, or run this command now:",
      `info: export PATH="${receipt.installDir}:$PATH"`,
      "info: then run 'envswitch --help' to verify the installation",
    );
  }
  return `${lines.join("\n")}\n`;
}

async function runUpdate(
  copy: InstalledCopy,
  options: {
    argv?: string[];
    latest?: string | undefined;
    install?: (plan: InstallPlan) => InstallerRun | Promise<InstallerRun>;
    env?: UpdateEnv;
    appDir?: string;
  } = {},
) {
  const plans: InstallPlan[] = [];
  const fetches: string[] = [];
  const output: string[] = [];
  const logs: string[] = [];
  const result = await runCli(options.argv ?? ["update"], {
    appDir: options.appDir ?? copy.appDir,
    env: options.env ?? copy.env,
    fetchLatestVersion: async (repo) => {
      fetches.push(repo);
      return "latest" in options ? options.latest : "v0.0.5";
    },
    runInstaller: async (plan) => {
      plans.push(plan);
      return options.install
        ? options.install(plan)
        : { code: 0, stdout: installerStdout(commitInstall(plan), "installed") };
    },
    log: (event) => logs.push(`${event.status}:${event.message}`),
    writeLine: (line) => output.push(line),
  });
  return { ...result, plans, fetches, output, logs };
}

describe.skipIf(WINDOWS)("cli update command", () => {
  test("rejects unknown update flags before receipt, network, or installer work", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n" });
    writeFileSync(join(copy.appDir, "install.json"), "{not json");

    const result = await runUpdate(copy, { argv: ["update", "--vault", "x"] });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --vault"]);
    expect(result.fetches).toEqual([]);
    expect(result.plans).toEqual([]);
  });

  test("rejects updates from a source checkout", async () => {
    for (const kind of ["directory", "file"] as const) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });
      if (kind === "directory") {
        mkdirSync(join(copy.appDir, ".git"));
      } else {
        writeFileSync(join(copy.appDir, ".git"), "gitdir: /elsewhere\n");
      }

      const result = await runUpdate(copy);

      expect(result.code).toBe(1);
      expect(result.fetches).toEqual([]);
      expect(result.plans).toEqual([]);
      expect(result.stderr).toEqual([
        "envswitch: envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)",
      ]);
    }
  });

  test("fails when the bundled installer is missing", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n", installer: false });

    const result = await runUpdate(copy);

    expect(result.code).toBe(1);
    expect(result.fetches).toEqual([]);
    expect(result.stderr).toEqual([
      `envswitch: installer not found at ${copy.installer}; reinstall envswitch with the install script from the README`,
    ]);
  });

  test("pins the latest tag once and hands the identified locations to the installer", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n" });

    const result = await runUpdate(copy);

    expect(result.code).toBe(0);
    expect(result.stderr).toEqual([]);
    expect(result.fetches).toEqual([DEFAULT_REPO]);
    expect(result.plans).toEqual([
      {
        installer: copy.installer,
        repo: DEFAULT_REPO,
        appRoot: copy.appRoot,
        installDir: copy.binDir,
        version: "v0.0.5",
        currentVersion: "v0.0.4",
      },
    ]);
    expect(result.logs).toEqual(["info:updating envswitch v0.0.4 -> v0.0.5"]);
    expect(result.output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
  });

  test("always invokes the installer on an equal version and reports its outcome", async () => {
    for (const [outcome, expected] of [
      ["unchanged", "envswitch is already up to date (v0.0.5)"],
      ["repaired", "repaired envswitch v0.0.5"],
      ["installed", "updated envswitch v0.0.5 -> v0.0.5"],
    ] as const) {
      const copy = makeInstalledCopy({ version: "v0.0.5\n" });

      const result = await runUpdate(copy, {
        install: (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan), outcome) }),
      });

      expect(result.code).toBe(0);
      expect(result.plans).toHaveLength(1);
      expect(result.plans[0]?.currentVersion).toBe("v0.0.5");
      expect(result.logs).toEqual(["info:updating envswitch v0.0.5 -> v0.0.5"]);
      expect(result.output).toEqual([expected]);
    }
  });

  test("reports an update when a racing installer already committed the new tag", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n" });

    const result = await runUpdate(copy, {
      install: (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan), "unchanged") }),
    });

    expect(result.code).toBe(0);
    expect(result.output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
  });

  test("treats a missing or non-canonical marker as an unknown version", async () => {
    for (const marker of [undefined, "", "latest\n", "v0.0.04\n", " v0.0.4\n", "v1.2.3\n", "v0.0.4\n\n"]) {
      const copy = makeInstalledCopy(marker === undefined ? {} : { version: marker });

      const result = await runUpdate(copy);

      expect(result.code).toBe(0);
      expect(result.plans[0]?.currentVersion).toBeUndefined();
      expect(result.logs).toEqual(["info:updating envswitch (unknown version) -> v0.0.5"]);
      expect(result.output).toEqual(["updated envswitch (unknown) -> v0.0.5"]);
    }

    const copy = makeInstalledCopy();
    const repaired = await runUpdate(copy, {
      install: (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan), "repaired") }),
    });
    expect(repaired.output).toEqual(["repaired envswitch v0.0.5"]);
  });

  test("accepts legacy BOM, CRLF, and unprefixed version markers", async () => {
    for (const marker of ["\uFEFFv0.0.4\r\n", "0.0.4\n", "v0.0.4"]) {
      const copy = makeInstalledCopy({ version: marker });

      const result = await runUpdate(copy);

      expect(result.plans[0]?.currentVersion).toBe("v0.0.4");
      expect(result.output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
    }
  });

  test("fails without an unpinned fallback when latest cannot be resolved", async () => {
    for (const latest of [undefined, "", "0.0.5", "v0.0.05", "v1.2.3", "v0.0.5-rc.1", " v0.0.5"]) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });

      const result = await runUpdate(copy, { latest });

      expect(result.code).toBe(1);
      expect(result.stderr).toEqual([`envswitch: no GitHub Release found for ${DEFAULT_REPO}, or access is denied`]);
      expect(result.plans).toEqual([]);
      expect(result.logs).toEqual([]);
      expect(result.output).toEqual([]);
    }
  });

  test("reports installer failures with the installer status and exit 1", async () => {
    for (const code of [1, 4, 130, 143]) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });

      const result = await runUpdate(copy, { install: () => ({ code, stdout: "" }) });

      expect(result.code).toBe(1);
      expect(result.stderr).toEqual([`envswitch: update failed: installer exited with code ${code}`]);
      expect(result.output).toEqual([]);
    }
  });

  test("never reports success for a zero exit without a matching committed result", async () => {
    const cases: Array<[string, (plan: InstallPlan, copy: InstalledCopy) => InstallerRun]> = [
      ["no result or receipt", () => ({ code: 0, stdout: "" })],
      ["no terminal line", (plan) => (commitInstall(plan), { code: 0, stdout: "info: detected Linux\n" })],
      ["missing final newline", (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan), "installed").slice(0, -1) })],
      [
        "output after unchanged",
        (plan) => ({ code: 0, stdout: `${installerStdout(commitInstall(plan), "unchanged")}info: extra\n` }),
      ],
      [
        "output after guidance",
        (plan) => ({ code: 0, stdout: `${installerStdout(commitInstall(plan), "installed")}info: extra\n` }),
      ],
      [
        "reported tag differs from receipt",
        (plan) => {
          const receipt = commitInstall(plan);
          return { code: 0, stdout: installerStdout({ ...receipt, tag: "v0.0.6" }, "installed") };
        },
      ],
      [
        "committed tag differs from pin",
        (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { tag: "v0.0.6" }), "installed") }),
      ],
      [
        "marker mismatch",
        (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { marker: "v0.0.4\n" }), "installed") }),
      ],
      [
        "marker with CRLF",
        (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { marker: "v0.0.5\r\n" }), "installed") }),
      ],
      ["missing payload", (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { payload: false }), "installed") })],
      [
        "missing launcher",
        (plan) => {
          const receipt = commitInstall(plan, { launcher: false });
          unlinkSync(receipt.launcherPath);
          return { code: 0, stdout: installerStdout(receipt, "installed") };
        },
      ],
      [
        "receipt repo changed",
        (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { repo: "other/repo" }), "installed") }),
      ],
      [
        "receipt launcher directory changed",
        (plan, copy) => ({
          code: 0,
          stdout: installerStdout(commitInstall(plan, { installDir: join(copy.root, "elsewhere") }), "installed"),
        }),
      ],
      [
        "location line mismatch",
        (plan) => {
          const receipt = commitInstall(plan);
          return { code: 0, stdout: installerStdout(receipt, "installed").replace(receipt.payloadPath, "/elsewhere") };
        },
      ],
      [
        "guidance mismatch",
        (plan) => ({
          code: 0,
          stdout: installerStdout(commitInstall(plan), "installed", false).replace(':$PATH"', ':$HOME/bin:$PATH"'),
        }),
      ],
      [
        "receipt invalid after install",
        (plan) => {
          const stdout = installerStdout(commitInstall(plan), "installed");
          writeFileSync(join(plan.appRoot, "current", "install.json"), "{}\n");
          return { code: 0, stdout };
        },
      ],
    ];
    for (const [name, install] of cases) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });

      const result = await runUpdate(copy, { install: (plan) => install(plan, copy) });

      expect({ name, code: result.code, stderr: result.stderr, output: result.output }).toEqual({
        name,
        code: 1,
        stderr: [POSTCONDITION_ERROR],
        output: [],
      });
    }
  });

  test("accepts only the final installer-owned terminal line and its guidance", async () => {
    for (const onPath of [true, false]) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });

      const result = await runUpdate(copy, {
        install: (plan) => ({
          code: 0,
          // A dependency step printing a success-looking line cannot be the result.
          stdout: `success: installed envswitch v0.0.9\n${installerStdout(commitInstall(plan), "installed", onPath)}`,
        }),
      });

      expect(result.code).toBe(0);
      expect(result.output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
    }
  });

  test("uses a valid receipt as authoritative over shell settings", async () => {
    for (const kind of ["source", "native"] as const) {
      const copy = makeInstalledCopy({ launcher: "none" });
      const installDir = join(copy.root, "custom bin");
      commitInstall({ appRoot: copy.appRoot, installDir, repo: "acme/tools", version: "v0.0.4" }, { kind });

      const result = await runUpdate(copy, {
        env: { ...copy.env, REPO: "other/repo", INSTALL_DIR: "/elsewhere", APP_ROOT: "/elsewhere" },
        install: (plan) => ({ code: 0, stdout: installerStdout(commitInstall(plan, { kind }), "installed") }),
      });

      expect(result.code).toBe(0);
      expect(result.fetches).toEqual(["acme/tools"]);
      expect(result.plans[0]).toMatchObject({
        repo: "acme/tools",
        appRoot: copy.appRoot,
        installDir,
        currentVersion: "v0.0.4",
      });
      expect(result.output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
    }
  });

  test("rejects invalid receipts before resolving latest or running an installer", async () => {
    type Mutation = (receipt: Record<string, unknown>, copy: InstalledCopy) => string | Uint8Array | Record<string, unknown>;
    const mutations: Array<[string, Mutation]> = [
      ["byte order mark", (r) => `\uFEFF${JSON.stringify(r)}`],
      ["duplicate key", (r) => `${JSON.stringify(r).slice(0, -1)},"tag":"v0.0.4"}`],
      ["duplicate key in an unknown object", (r) => `${JSON.stringify(r).slice(0, -1)},"extra":{"a":1,"a":2}}`],
      ["invalid JSON", () => "{"],
      ["array", (r) => JSON.stringify([r])],
      ["invalid UTF-8", (r) => Buffer.concat([Buffer.from(JSON.stringify(r).slice(0, -1)), Buffer.from(',"x":"\xff"}', "latin1")])],
      ["schemaVersion 1.0", (r) => JSON.stringify(r).replace('"schemaVersion":1', '"schemaVersion":1.0')],
      ["schemaVersion string", (r) => ({ ...r, schemaVersion: "1" })],
      ["schemaVersion 2", (r) => ({ ...r, schemaVersion: 2 })],
      ["unknown kind", (r) => ({ ...r, kind: "binary" })],
      ["non-canonical tag", (r) => ({ ...r, tag: "v0.0.04" })],
      ["unprefixed tag", (r) => ({ ...r, tag: "0.0.4" })],
      ["invalid repo", (r) => ({ ...r, repo: "../x" })],
      ["another app root", (r, c) => ({ ...r, appRoot: join(c.root, "other") })],
      ["install dir inside current", (r, c) => ({ ...r, installDir: join(c.appDir, "bin"), launcherPath: join(c.appDir, "bin", "envswitch") })],
      ["install dir equals app root", (r, c) => ({ ...r, installDir: c.appRoot, launcherPath: join(c.appRoot, "envswitch") })],
      ["install dir in installer stage", (r, c) => ({ ...r, installDir: join(c.appRoot, ".install-stage-x"), launcherPath: join(c.appRoot, ".install-stage-x", "envswitch") })],
      ["launcher outside install dir", (r, c) => ({ ...r, launcherPath: join(c.root, "envswitch") })],
      ["launcher with another name", (r, c) => ({ ...r, launcherPath: join(c.binDir, "envswitch.cmd") })],
      ["payload outside app root", (r, c) => ({ ...r, payloadPath: join(c.root, "envswitch_0.0.4_x86_64-unknown-linux-musl") })],
      ["payload for another tag", (r, c) => ({ ...r, payloadPath: join(c.appDir, "bin", "envswitch_0.0.3_x86_64-unknown-linux-musl") })],
      ["native without target", (r) => ({ ...r, target: null })],
      ["Windows target", (r, c) => ({ ...r, target: "x86_64-pc-windows-msvc", payloadPath: join(c.appDir, "bin", "envswitch_0.0.4_x86_64-pc-windows-msvc") })],
      ["unknown target", (r, c) => ({ ...r, target: "riscv64gc-unknown-linux-gnu", payloadPath: join(c.appDir, "bin", "envswitch_0.0.4_riscv64gc-unknown-linux-gnu") })],
      ["source with target", (r, c) => ({ ...r, kind: "source", payloadPath: join(c.appDir, "src", "cli.ts") })],
      ["relative install dir", (r) => ({ ...r, installDir: "bin", launcherPath: "bin/envswitch" })],
      ["trailing slash", (r, c) => ({ ...r, installDir: `${c.binDir}/`, launcherPath: `${c.binDir}//envswitch` })],
      ["dot component", (r, c) => ({ ...r, installDir: `${c.home}/./bin`, launcherPath: `${c.home}/./bin/envswitch` })],
      ["parent component", (r, c) => ({ ...r, installDir: `${c.binDir}/../bin`, launcherPath: `${c.binDir}/../bin/envswitch` })],
      ["control character", (r, c) => ({ ...r, installDir: `${c.home}/b\nin`, launcherPath: `${c.home}/b\nin/envswitch` })],
      ["colon", (r, c) => ({ ...r, installDir: `${c.home}/b:in`, launcherPath: `${c.home}/b:in/envswitch` })],
      ["missing field", (r) => {
        const { payloadPath: _payloadPath, ...rest } = r;
        return rest;
      }],
      ["symlinked install dir", (r, c) => {
        mkdirSync(join(c.root, "real-bin"));
        symlinkSync(join(c.root, "real-bin"), join(c.root, "alias-bin"));
        return { ...r, installDir: join(c.root, "alias-bin"), launcherPath: join(c.root, "alias-bin", "envswitch") };
      }],
      ["app root equals HOME", (r, c) => {
        c.env.HOME = c.appRoot;
        return r;
      }],
    ];
    for (const [name, mutate] of mutations) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });
      const valid = commitInstall({ appRoot: copy.appRoot, installDir: copy.binDir, repo: DEFAULT_REPO, version: "v0.0.4" });
      const mutated = mutate({ schemaVersion: 1, ...valid }, copy);
      writeFileSync(
        join(copy.appDir, "install.json"),
        typeof mutated === "string" || mutated instanceof Uint8Array ? mutated : JSON.stringify(mutated),
      );

      const result = await runUpdate(copy);

      expect({ name, code: result.code, stderr: result.stderr, fetches: result.fetches, plans: result.plans }).toEqual({
        name,
        code: 1,
        stderr: [RECEIPT_ERROR],
        fetches: [],
        plans: [],
      });
    }
  });

  test("adopts a receipt-less install from explicit Unix REPO and INSTALL_DIR", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n", launcher: "none" });

    const result = await runUpdate(copy, {
      env: { ...copy.env, REPO: "acme/tools", INSTALL_DIR: `${copy.root}//custom/./bin/`, APP_ROOT: "/ignored" },
    });

    expect(result.code).toBe(0);
    expect(result.fetches).toEqual(["acme/tools"]);
    expect(result.plans[0]).toMatchObject({
      repo: "acme/tools",
      appRoot: copy.appRoot,
      installDir: join(copy.root, "custom", "bin"),
    });
  });

  test("adopts the single managed launcher found in the default directory or on PATH", async () => {
    const shapes: Array<[string, (copy: InstalledCopy) => string]> = [
      ["historical launcher", (c) => legacyLauncher(c.appDir)],
      ["historical launcher without blank lines or final newline", (c) =>
        legacyLauncher(c.appDir).replace(/\n\n/g, "\n").replace(/\n$/, "")],
      ["protocol source launcher", (c) => sourceLauncher(join(c.appDir, "src", "cli.ts"))],
      ["protocol native launcher", (c) =>
        nativeLauncher(join(c.appDir, "bin", "envswitch_0.0.3_aarch64-apple-darwin"))],
    ];
    for (const [name, shape] of shapes) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n", launcher: "none" });
      const pathBin = join(copy.root, "path bin");
      writeExecutable(join(pathBin, "envswitch"), shape(copy));
      writeExecutable(join(copy.root, "other-bin", "envswitch"), "#!/bin/sh\necho unrelated\n");

      const result = await runUpdate(copy, {
        env: { ...copy.env, PATH: ["relative", join(copy.root, "other-bin"), pathBin, join(copy.root, "missing")].join(":") },
      });

      expect({ name, code: result.code, installDir: result.plans[0]?.installDir }).toEqual({
        name,
        code: 0,
        installDir: pathBin,
      });
    }
  });

  test("resolves PATH aliases and launcher links to one real launcher directory", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n" });
    symlinkSync(copy.binDir, join(copy.root, "alias"));
    mkdirSync(join(copy.root, "linked"));
    symlinkSync(join(copy.binDir, "envswitch"), join(copy.root, "linked", "envswitch"));

    const result = await runUpdate(copy, {
      env: { ...copy.env, PATH: [join(copy.root, "alias"), join(copy.root, "linked"), copy.binDir].join(":") },
    });

    expect(result.code).toBe(0);
    expect(result.plans[0]?.installDir).toBe(copy.binDir);
  });

  test("refuses to guess a launcher directory", async () => {
    const cases: Array<[string, (copy: InstalledCopy) => Partial<{ env: UpdateEnv; appDir: string }>]> = [
      ["no launcher", (c) => (unlinkSync(join(c.binDir, "envswitch")), {})],
      ["unrelated launcher", (c) => (writeFileSync(join(c.binDir, "envswitch"), "#!/bin/sh\nexec mytool \"$@\"\n"), {})],
      ["launcher for another root", (c) =>
        (writeFileSync(join(c.binDir, "envswitch"), legacyLauncher(join(c.root, "other", "current"))), {})],
      ["two matching launchers", (c) => {
        writeExecutable(join(c.root, "second-bin", "envswitch"), legacyLauncher(c.appDir));
        return { env: { ...c.env, PATH: join(c.root, "second-bin") } };
      }],
      ["no HOME and nothing on PATH", (c) => ({ env: { PATH: c.env.PATH } })],
      ["app directory is not current", (c) => {
        const appDir = join(c.appRoot, "envswitch-src");
        writeExecutable(join(appDir, "scripts", "install.sh"), "exit 99\n");
        return { appDir };
      }],
    ];
    for (const [name, arrange] of cases) {
      const copy = makeInstalledCopy({ version: "v0.0.4\n" });
      const overrides = arrange(copy);

      const result = await runUpdate(copy, overrides);

      expect({ name, code: result.code, stderr: result.stderr, fetches: result.fetches }).toEqual({
        name,
        code: 1,
        stderr: [LAUNCHER_DIR_ERROR],
        fetches: [],
      });
    }
  });

  test("rejects an invalid explicit repository before network work", async () => {
    const copy = makeInstalledCopy({ version: "v0.0.4\n" });

    const result = await runUpdate(copy, { env: { ...copy.env, REPO: "not-a-repo" } });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: invalid repository format: not-a-repo; expected owner/repo"]);
    expect(result.fetches).toEqual([]);
  });
});

describe("update helpers", () => {
  const winRoot = "C:\\Users\\Demo User\\AppData\\Local\\Programs\\envswitch";
  const winReceipt = {
    schemaVersion: 1,
    kind: "native",
    repo: DEFAULT_REPO,
    appRoot: winRoot,
    installDir: `${winRoot}\\bin`,
    launcherPath: `${winRoot}\\bin\\envswitch.cmd`,
    target: "x86_64-pc-windows-msvc",
    tag: "v0.0.42",
    payloadPath: `${winRoot}\\current\\bin\\envswitch_0.0.42_x86_64-pc-windows-msvc.exe`,
  };
  const encode = (value: unknown) => Buffer.from(JSON.stringify(value));

  test("validates Windows receipts with case-insensitive paths and Windows rules", () => {
    expect(parseInstallReceipt(encode(winReceipt), winRoot.toLowerCase(), {}, true)).toMatchObject({
      kind: "native",
      installDir: `${winRoot}\\bin`,
    });
    // PowerShell ConvertTo-Json output: pretty-printed, CRLF, escaped characters.
    const pretty = JSON.stringify(winReceipt, null, 4).replace(/\n/g, "\r\n").replace(/Demo User/g, "Demo\\u0020User");
    expect(parseInstallReceipt(Buffer.from(`${pretty}\n`), winRoot, {}, true)?.appRoot).toBe(winRoot);
    expect(
      parseInstallReceipt(
        encode({ ...winReceipt, kind: "source", target: null, payloadPath: `${winRoot}\\current\\src\\cli.ts` }),
        winRoot,
        {},
        true,
      )?.kind,
    ).toBe("source");

    for (const invalid of [
      { payloadPath: winReceipt.payloadPath.replace(".exe", "") },
      { target: "aarch64-apple-darwin", payloadPath: `${winRoot}\\current\\bin\\envswitch_0.0.42_aarch64-apple-darwin.exe` },
      { launcherPath: `${winRoot}\\bin\\envswitch` },
      { installDir: "C:/tools/bin", launcherPath: "C:/tools/bin/envswitch.cmd" },
      { installDir: "C:\\tools\\bin.", launcherPath: "C:\\tools\\bin.\\envswitch.cmd" },
      { installDir: "\\\\server\\share\\bin", launcherPath: "\\\\server\\share\\bin\\envswitch.cmd" },
      { installDir: "C:\\tools\\bin:stream", launcherPath: "C:\\tools\\bin:stream\\envswitch.cmd" },
    ]) {
      expect(parseInstallReceipt(encode({ ...winReceipt, ...invalid }), winRoot, {}, true)).toBeUndefined();
    }
    expect(parseInstallReceipt(encode(winReceipt), winRoot, { USERPROFILE: winRoot.toUpperCase() }, true)).toBeUndefined();
  });

  test("ignores unknown receipt fields", () => {
    const root = "/home/demo/.local/share/envswitch";
    const receipt = {
      schemaVersion: 1,
      kind: "source",
      repo: DEFAULT_REPO,
      appRoot: root,
      installDir: "/home/demo/.local/bin",
      launcherPath: "/home/demo/.local/bin/envswitch",
      target: null,
      tag: "v0.0.41",
      payloadPath: `${root}/current/src/cli.ts`,
      future: { anything: [1, 2.5e3, true, null] },
    };
    expect(parseInstallReceipt(encode(receipt), root, {}, false)?.tag).toBe("v0.0.41");
  });

  test("recognizes Windows launchers exactly as the PowerShell installer does", () => {
    const root = "C:\\Apps\\env%switch";
    const cli = "C:\\Apps\\env%%switch\\current\\src\\cli.ts";
    const exe = "C:\\Apps\\env%%switch\\current\\bin\\envswitch_0.0.42_x86_64-pc-windows-msvc.exe";
    const head = '@echo off\r\nsetlocal DisableDelayedExpansion\r\nfor /f "tokens=2 delims=:." %%C in (\'chcp\') do set "_ENVSWITCH_CP=%%C"\r\nchcp 65001 >nul\r\n';
    const tail = "chcp %_ENVSWITCH_CP% >nul 2>&1\r\nexit /b %_ENVSWITCH_EXIT%\r\n";
    const native = `${head}"${exe}" %*\r\nset "_ENVSWITCH_EXIT=%ERRORLEVEL%"\r\n${tail}`;
    const source =
      `${head}where bun >NUL 2>NUL\r\nif %ERRORLEVEL% EQU 0 goto envswitch_bun_path\r\n` +
      'if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" goto envswitch_bun_home\r\n' +
      'echo envswitch: bun is not installed or not on PATH 1>&2\r\nset "_ENVSWITCH_EXIT=127"\r\ngoto envswitch_finish\r\n' +
      `:envswitch_bun_path\r\nbun "${cli}" %*\r\nset "_ENVSWITCH_EXIT=%ERRORLEVEL%"\r\ngoto envswitch_finish\r\n` +
      `:envswitch_bun_home\r\n"%USERPROFILE%\\.bun\\bin\\bun.exe" "${cli}" %*\r\nset "_ENVSWITCH_EXIT=%ERRORLEVEL%"\r\n` +
      `:envswitch_finish\r\n${tail}`;
    const protocolText = (text: string) =>
      text.replace("tokens=2 delims=:.", "tokens=2 delims=:").replace(">nul 2>&1", ">nul");
    // Set-Content -Encoding ASCII output of the pre-bridge PowerShell installer.
    const legacyCli = `${root}\\current\\src\\cli.ts`;
    const legacy =
      `@echo off\r\nsetlocal\r\nwhere bun >NUL 2>NUL\r\nif %ERRORLEVEL% EQU 0 (\r\n  bun "${legacyCli}" %*\r\n  exit /b %ERRORLEVEL%\r\n)\r\n` +
      `if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" (\r\n  "%USERPROFILE%\\.bun\\bin\\bun.exe" "${legacyCli}" %*\r\n  exit /b %ERRORLEVEL%\r\n)\r\n` +
      "echo envswitch: bun is not installed or not on PATH 1>&2\r\nexit /b 127\r\n";

    for (const text of [native, source, protocolText(native), protocolText(source), legacy, `\uFEFF${native}`]) {
      expect(isManagedLauncher(Buffer.from(text), root, true)).toBe(true);
    }
    for (const text of [
      native.replace(/\r\n/g, "\n"),
      native.replace("0.0.42", "0.0.042"),
      legacy.replace("exit /b 127", "exit /b 1"),
      "@echo off\r\nmytool %*\r\n",
    ]) {
      expect(isManagedLauncher(Buffer.from(text), root, true)).toBe(false);
    }
    expect(isManagedLauncher(Buffer.from(native), "C:\\Apps\\other", true)).toBe(false);
  });

  test("quotes Unix launcher paths the way the Bash installer does", () => {
    const root = "/home/o'neil/envswitch data";
    const payload = `${root}/current/bin/envswitch_0.0.42_x86_64-unknown-linux-musl`;
    expect(isManagedLauncher(Buffer.from(nativeLauncher(payload)), root, false)).toBe(true);
    expect(isManagedLauncher(Buffer.from(sourceLauncher(`${root}/current/src/cli.ts`)), root, false)).toBe(true);
    expect(isManagedLauncher(Buffer.from(`${nativeLauncher(payload)}\n\n`), root, false)).toBe(true);
    expect(isManagedLauncher(Buffer.from(nativeLauncher(payload).replace("set -euo", "set -eu")), root, false)).toBe(false);
    expect(isManagedLauncher(Buffer.from(nativeLauncher(payload)), "/home/other", false)).toBe(false);
  });

  test("parses installer outcomes on Unix and Windows", () => {
    const unix: InstallReceipt = {
      kind: "native",
      repo: DEFAULT_REPO,
      appRoot: "/data/envswitch",
      installDir: '/data/new $`"bin',
      launcherPath: '/data/new $`"bin/envswitch',
      target: UNIX_TARGET,
      tag: "v0.0.42",
      payloadPath: `/data/envswitch/current/bin/envswitch_0.0.42_${UNIX_TARGET}`,
    };
    const offPath = [
      "success: repaired envswitch v0.0.42",
      `info: binary: ${unix.payloadPath}`,
      `info: launcher: ${unix.launcherPath}`,
      "info: restart your terminal, or run this command now:",
      'info: export PATH="/data/new \\$\\`\\"bin:$PATH"',
      "info: then run 'envswitch --help' to verify the installation",
      "",
    ].join("\n");
    expect(parseInstallerOutcome(offPath, unix, false)).toEqual({ outcome: "repaired", exactLocations: true });
    expect(parseInstallerOutcome(offPath.replace(/\n/g, "\r\n"), unix, false)).toEqual({
      outcome: "repaired",
      exactLocations: true,
    });
    expect(parseInstallerOutcome(offPath.replace('new \\$\\`\\"bin', 'new $`"bin'), unix, false)).toBeUndefined();
    expect(parseInstallerOutcome("success: envswitch is already up to date (v0.0.42)\n", unix, false)).toEqual({
      outcome: "unchanged",
      exactLocations: true,
    });
    expect(parseInstallerOutcome("success: envswitch is already up to date (v0.0.41)\n", unix, false)).toBeUndefined();

    const windows: InstallReceipt = {
      ...unix,
      appRoot: "C:\\Programs\\envswitch",
      installDir: "C:\\Programs\\envswitch\\bin",
      launcherPath: "C:\\Programs\\envswitch\\bin\\envswitch.cmd",
      target: "x86_64-pc-windows-msvc",
      payloadPath: "C:\\Programs\\envswitch\\current\\bin\\envswitch_0.0.42_x86_64-pc-windows-msvc.exe",
    };
    const windowsOut = (binary: string) =>
      [
        "success: installed envswitch v0.0.42",
        `info: binary: ${binary}`,
        `info: launcher: ${windows.launcherPath}`,
        "info: restart your terminal, or run this command now:",
        'info: $env:Path = "C:\\Programs\\envswitch\\bin;$env:Path"',
        "info: then run 'envswitch --help' to verify the installation",
        "",
      ].join("\r\n");
    expect(parseInstallerOutcome(windowsOut(windows.payloadPath), windows, true)).toEqual({
      outcome: "installed",
      exactLocations: true,
    });
    // A console code page may replace characters; the receipt stays authoritative.
    expect(parseInstallerOutcome(windowsOut("C:\\Programs\\envswitch\\current\\bin\\?.exe"), windows, true)).toEqual({
      outcome: "installed",
      exactLocations: false,
    });
    expect(parseInstallerOutcome(windowsOut(windows.payloadPath).replace("info: then", "info: now"), windows, true)).toBeUndefined();
  });

  test("builds the Unix handoff environment and removes stale caller settings", () => {
    const plan: InstallPlan = {
      installer: "/data/envswitch/current/scripts/install.sh",
      repo: "acme/tools",
      appRoot: "/data/envswitch",
      installDir: "/data/bin",
      version: "v0.0.5",
      currentVersion: "v0.0.4",
    };
    const inherited: UpdateEnv = {
      PATH: "/usr/bin",
      HOME: "/home/demo",
      GITHUB_TOKEN: "synthetic",
      REPO: "stale/repo",
      APP_ROOT: "/stale",
      INSTALL_DIR: "/stale",
      ENVSWITCH_VERSION: "v0.0.1",
      ENVSWITCH_CURRENT_VERSION: "v0.0.2",
      ENVSWITCH_VERIFY_ATTESTATIONS: "1",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
      ENVSWITCH_INSTALL_TEST_API_BASE_URL: "http://127.0.0.1:1",
      UNSET: undefined,
    };

    expect(buildInstallerInvocation(plan, "/tmp/private/install.sh", inherited, false)).toEqual({
      argv: ["bash", "/tmp/private/install.sh"],
      env: {
        PATH: "/usr/bin",
        HOME: "/home/demo",
        GITHUB_TOKEN: "synthetic",
        REPO: "acme/tools",
        APP_ROOT: "/data/envswitch",
        INSTALL_DIR: "/data/bin",
        ENVSWITCH_VERSION: "v0.0.5",
        ENVSWITCH_CURRENT_VERSION: "v0.0.4",
        ENVSWITCH_INSTALL_TEST_MODE: "install",
        ENVSWITCH_INSTALL_TEST_API_BASE_URL: "http://127.0.0.1:1",
      },
    });
    const unknown = buildInstallerInvocation({ ...plan, currentVersion: undefined }, "/tmp/p/install.sh", inherited, false);
    expect(unknown.env.ENVSWITCH_CURRENT_VERSION).toBeUndefined();
    expect(unknown.env.ENVSWITCH_VERIFY_ATTESTATIONS).toBeUndefined();
  });

  test("builds the exact Windows PowerShell argv", () => {
    const plan: InstallPlan = {
      installer: "C:\\envswitch\\current\\scripts\\install.ps1",
      repo: DEFAULT_REPO,
      appRoot: "C:\\Users\\Demo User\\envswitch",
      installDir: "C:\\Users\\Demo User\\envswitch\\bin",
      version: "v0.0.42",
      currentVersion: "v0.0.41",
    };
    const inherited = { Path: "C:\\Windows", envswitch_verify_attestations: "1", Envswitch_Current_Version: "v0.0.1" };

    const pinned = buildInstallerInvocation(plan, "C:\\Temp\\envswitch-update-x\\install.ps1", inherited, true);
    expect(pinned.argv).toEqual([
      "powershell",
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      "C:\\Temp\\envswitch-update-x\\install.ps1",
      "-AppRoot",
      "C:\\Users\\Demo User\\envswitch",
      "-InstallDir",
      "C:\\Users\\Demo User\\envswitch\\bin",
      "-Repo",
      DEFAULT_REPO,
      "-Version",
      "v0.0.42",
      "-CurrentVersion",
      "v0.0.41",
    ]);
    expect(pinned.env).toEqual({ Path: "C:\\Windows", Envswitch_Current_Version: "v0.0.1" });

    const unknown = buildInstallerInvocation({ ...plan, currentVersion: undefined }, "C:\\T\\install.ps1", inherited, true);
    expect(unknown.argv).not.toContain("-CurrentVersion");
    expect(unknown.env).toEqual({ Path: "C:\\Windows" });
  });

  describe("latest release resolution", () => {
    function fakeFetch(responses: Array<Response | Error>) {
      const calls: Array<{ url: string; init: RequestInit }> = [];
      const sleeps: number[] = [];
      return {
        calls,
        sleeps,
        options: {
          fetch: async (url: string, init: RequestInit) => {
            calls.push({ url, init });
            const next = responses.shift();
            if (next === undefined) {
              throw new Error("unexpected fetch");
            }
            if (next instanceof Error) {
              throw next;
            }
            return next;
          },
          sleep: async (ms: number) => {
            sleeps.push(ms);
          },
        },
      };
    }
    const release = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
      new Response(typeof body === "string" ? body : JSON.stringify(body), { status, headers });

    test("returns only a canonical published tag from the configured repository", async () => {
      const ok = fakeFetch([release({ tag_name: "v0.0.42", draft: false, prerelease: false })]);
      expect(await fetchLatestReleaseTag("acme/tools", { env: { GITHUB_TOKEN: "synthetic" }, ...ok.options })).toBe("v0.0.42");
      expect(ok.calls.map((call) => call.url)).toEqual(["https://api.github.com/repos/acme/tools/releases/latest"]);
      expect(ok.calls[0]?.init.headers).toEqual({ Accept: "application/vnd.github+json", Authorization: "Bearer synthetic" });
      expect(ok.calls[0]?.init.redirect).toBe("manual");

      const anonymous = fakeFetch([release({ tag_name: "v0.0.42" })]);
      await fetchLatestReleaseTag("acme/tools", { env: { GITHUB_TOKEN: "" }, ...anonymous.options });
      expect(anonymous.calls[0]?.init.headers).toEqual({ Accept: "application/vnd.github+json" });

      for (const body of [
        { tag_name: "0.0.42" },
        { tag_name: "v0.0.042" },
        { tag_name: "v1.0.0" },
        { tag_name: "v0.0.42", draft: true },
        { tag_name: "v0.0.42", prerelease: true },
        ["v0.0.42"],
        "{not json",
      ]) {
        const once = fakeFetch([release(body)]);
        expect(await fetchLatestReleaseTag("acme/tools", { env: {}, ...once.options })).toBeUndefined();
        expect(once.calls).toHaveLength(1);
      }
    });

    test("retries only transport, 408, 429, and 5xx failures, at most three attempts", async () => {
      const notFound = fakeFetch([release("", 404)]);
      expect(await fetchLatestReleaseTag("acme/tools", { env: {}, ...notFound.options })).toBeUndefined();
      expect(notFound.calls).toHaveLength(1);

      const flaky = fakeFetch([release("", 500), release("", 429, { "retry-after": "5" }), release({ tag_name: "v0.0.7" })]);
      expect(await fetchLatestReleaseTag("acme/tools", { env: {}, ...flaky.options })).toBe("v0.0.7");
      expect(flaky.sleeps).toEqual([1000, 5000]);

      const offline = fakeFetch([new Error("down"), new Error("down"), new Error("down")]);
      expect(await fetchLatestReleaseTag("acme/tools", { env: {}, ...offline.options })).toBeUndefined();
      expect(offline.calls).toHaveLength(3);
      expect(offline.sleeps).toEqual([1000, 2000]);
    });

    test("follows only same-origin redirects, never with the token", async () => {
      const renamed = fakeFetch([
        release("", 301, { location: "/repositories/1/releases/latest" }),
        release({ tag_name: "v0.0.8" }),
      ]);
      expect(await fetchLatestReleaseTag("acme/tools", { env: { GITHUB_TOKEN: "t" }, ...renamed.options })).toBe("v0.0.8");
      expect(renamed.calls.map((call) => [call.url, call.init.headers])).toEqual([
        ["https://api.github.com/repos/acme/tools/releases/latest", { Accept: "application/vnd.github+json", Authorization: "Bearer t" }],
        ["https://api.github.com/repositories/1/releases/latest", { Accept: "application/vnd.github+json" }],
      ]);

      for (const location of ["https://example.com/latest", "http://api.github.com/latest", "https://api.github.com:8443/latest"]) {
        const elsewhere = fakeFetch([release("", 302, { location }), release({ tag_name: "v0.0.9" })]);
        expect(await fetchLatestReleaseTag("acme/tools", { env: { GITHUB_TOKEN: "t" }, ...elsewhere.options })).toBeUndefined();
        expect(elsewhere.calls.map((call) => call.url)).toEqual(["https://api.github.com/repos/acme/tools/releases/latest"]);
      }
    });

    test("honors only the gated loopback test hook, never sending the token there", async () => {
      const hooked = fakeFetch([release("", 307, { location: "/repositories/2/releases/latest" }), release({ tag_name: "v0.0.1" })]);
      const env = {
        ENVSWITCH_INSTALL_TEST_MODE: "install",
        ENVSWITCH_INSTALL_TEST_API_BASE_URL: "http://127.0.0.1:8123",
        GITHUB_TOKEN: "synthetic",
      };
      expect(await fetchLatestReleaseTag("demo/suite", { env, ...hooked.options })).toBe("v0.0.1");
      expect(hooked.calls.map((call) => [call.url, call.init.headers])).toEqual([
        ["http://127.0.0.1:8123/repos/demo/suite/releases/latest", { Accept: "application/vnd.github+json" }],
        ["http://127.0.0.1:8123/repositories/2/releases/latest", { Accept: "application/vnd.github+json" }],
      ]);

      const ignored = fakeFetch([release({ tag_name: "v0.0.1" })]);
      await fetchLatestReleaseTag("demo/suite", { env: { ENVSWITCH_INSTALL_TEST_API_BASE_URL: "http://127.0.0.1:1" }, ...ignored.options });
      expect(ignored.calls[0]?.url).toBe("https://api.github.com/repos/demo/suite/releases/latest");

      for (const [bad, setting] of [
        [{ ENVSWITCH_INSTALL_TEST_MODE: "install", ENVSWITCH_INSTALL_TEST_API_BASE_URL: "http://example.com" }, "ENVSWITCH_INSTALL_TEST_API_BASE_URL"],
        [{ ENVSWITCH_INSTALL_TEST_MODE: "install" }, "ENVSWITCH_INSTALL_TEST_API_BASE_URL"],
        [{ ENVSWITCH_INSTALL_TEST_MODE: "shell-profile-path" }, "ENVSWITCH_INSTALL_TEST_MODE"],
      ] as const) {
        const never = fakeFetch([]);
        const error = await fetchLatestReleaseTag("demo/suite", { env: bad, ...never.options }).catch((e: unknown) => e);
        expect(error).toBeInstanceOf(EnvswitchError);
        expect((error as Error).message).toBe(`invalid installer setting: ${setting}`);
        expect(never.calls).toEqual([]);
      }
    });
  });
});

/** A sandbox whose PATH holds only symlinks to the named host tools. */
function makeToolSandbox(tools: string[]): { root: string; bin: string } {
  const root = realTempDir("envswitch-tools-");
  const bin = join(root, "bin");
  mkdirSync(bin);
  for (const name of tools) {
    const path = Bun.which(name, { PATH: process.env.PATH ?? "" });
    if (path) {
      symlinkSync(path, join(bin, name));
    }
  }
  return { root, bin };
}

describe.skipIf(WINDOWS)("update installer runner", () => {
  const FAKE_INSTALLER = `#!/usr/bin/env bash
{
  printf 'script=%s\\n' "$0"
  printf 'cwd=%s\\n' "$PWD"
  printf 'args=%s\\n' "$#"
  mode=$(ls -ld "\${0%/*}"); printf 'dirmode=%s\\n' "\${mode:0:10}"
  mode=$(ls -l "$0"); printf 'filemode=%s\\n' "\${mode:0:10}"
  for name in REPO APP_ROOT INSTALL_DIR ENVSWITCH_VERSION ENVSWITCH_CURRENT_VERSION ENVSWITCH_VERIFY_ATTESTATIONS GITHUB_TOKEN ENVSWITCH_INSTALL_TEST_MODE; do
    if [[ -n \${!name+x} ]]; then printf '%s=%s\\n' "$name" "\${!name}"; else printf '%s unset\\n' "$name"; fi
  done
} > "$FAKE_LOG"
printf 'info: from fake installer\\n'
case "\${FAKE_MODE:-}" in
  status) exit 7;;
  term) kill -TERM $$;;
esac
exit 0
`;

  function arrange() {
    const sandbox = makeToolSandbox(["bash", "ls"]);
    const appRoot = join(sandbox.root, "app");
    const installer = join(appRoot, "current", "scripts", "install.sh");
    writeExecutable(installer, FAKE_INSTALLER);
    const tempRoot = join(sandbox.root, "tmp");
    mkdirSync(tempRoot);
    const log = join(sandbox.root, "installer.log");
    const plan: InstallPlan = {
      installer,
      repo: "acme/tools",
      appRoot,
      installDir: join(sandbox.root, "bin dir"),
      version: "v0.0.5",
      currentVersion: "v0.0.4",
    };
    const env: UpdateEnv = {
      PATH: sandbox.bin,
      HOME: sandbox.root,
      FAKE_LOG: log,
      GITHUB_TOKEN: "synthetic",
      ENVSWITCH_VERIFY_ATTESTATIONS: "1",
      ENVSWITCH_CURRENT_VERSION: "v0.0.1",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
    };
    const readLog = () =>
      Object.fromEntries(
        readFileSync(log, "utf8")
          .trimEnd()
          .split("\n")
          .map((line) => (line.endsWith(" unset") ? [line.slice(0, -6), undefined] : [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)])),
      ) as Record<string, string | undefined>;
    return { sandbox, plan, env, tempRoot, readLog };
  }

  test("copies the installer outside current, runs it privately, tees stdout, and cleans up", async () => {
    const { plan, env, tempRoot, readLog } = arrange();
    const chunks: Uint8Array[] = [];
    const listeners = [process.listenerCount("SIGINT"), process.listenerCount("SIGTERM")];

    const run = await runInstallScript(plan, { env, windows: false, tempRoot, stdout: { write: (c) => chunks.push(c) } });

    expect(run).toEqual({ code: 0, stdout: "info: from fake installer\n" });
    expect(Buffer.concat(chunks).toString()).toBe("info: from fake installer\n");
    const log = readLog();
    expect(log.script?.startsWith(`${tempRoot}/envswitch-update-`)).toBe(true);
    expect(log.script?.endsWith("/install.sh")).toBe(true);
    expect(log.cwd).toBe(dirname(log.script!));
    expect(log.args).toBe("0");
    expect(log.dirmode).toBe("drwx------");
    expect(log.filemode).toBe("-rw-------");
    expect(log).toMatchObject({
      REPO: "acme/tools",
      APP_ROOT: plan.appRoot,
      INSTALL_DIR: plan.installDir,
      ENVSWITCH_VERSION: "v0.0.5",
      ENVSWITCH_CURRENT_VERSION: "v0.0.4",
      ENVSWITCH_VERIFY_ATTESTATIONS: undefined,
      GITHUB_TOKEN: "synthetic",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
    });
    expect(readdirSync(tempRoot)).toEqual([]);
    expect(readFileSync(plan.installer, "utf8")).toBe(FAKE_INSTALLER);
    expect([process.listenerCount("SIGINT"), process.listenerCount("SIGTERM")]).toEqual(listeners);

    await runInstallScript({ ...plan, currentVersion: undefined }, { env, windows: false, tempRoot, stdout: { write: () => {} } });
    expect(readLog().ENVSWITCH_CURRENT_VERSION).toBeUndefined();
  });

  test("forwards nonzero and signal statuses", async () => {
    const { plan, env, tempRoot } = arrange();
    const quiet = { write: () => {} };

    expect((await runInstallScript(plan, { env: { ...env, FAKE_MODE: "status" }, windows: false, tempRoot, stdout: quiet })).code).toBe(7);
    expect((await runInstallScript(plan, { env: { ...env, FAKE_MODE: "term" }, windows: false, tempRoot, stdout: quiet })).code).toBe(143);
    expect(readdirSync(tempRoot)).toEqual([]);
  });

  test("launch failures are controlled; a spawn failure stays internal and still cleans up", async () => {
    const { sandbox, plan, env, tempRoot } = arrange();

    const missing = await runInstallScript({ ...plan, installer: join(sandbox.root, "missing.sh") }, { env, windows: false, tempRoot }).catch((e: unknown) => e);
    expect(missing).toBeInstanceOf(EnvswitchError);
    expect((missing as Error).message.startsWith("update failed: could not run installer: ")).toBe(true);

    const inside = await runInstallScript(plan, { env, windows: false, tempRoot: join(plan.appRoot, "current") }).catch((e: unknown) => e);
    expect(inside).toBeInstanceOf(EnvswitchError);
    expect((inside as Error).message).toBe("update failed: could not run installer: temporary directory is inside current");

    mkdirSync(join(sandbox.root, "no-tools"));
    const spawn = await runInstallScript(plan, { env: { ...env, PATH: join(sandbox.root, "no-tools") }, windows: false, tempRoot }).catch((e: unknown) => e);
    expect(spawn).toBeInstanceOf(Error);
    expect(spawn).not.toBeInstanceOf(EnvswitchError);
    expect((spawn as Error).message.startsWith("update failed: could not run installer: ")).toBe(true);
    expect(readdirSync(tempRoot)).toEqual([]);
  });
});

const SYSTEM_ROOT = processEnv("SYSTEMROOT") ?? "C:\\Windows";
const WINDOWS_POWERSHELL = join(SYSTEM_ROOT, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");

/** Case-insensitive lookup of this test process's environment (Windows names vary in case). */
function processEnv(name: string): string | undefined {
  return Object.entries(process.env).find(([key]) => key.toUpperCase() === name)?.[1];
}

/**
 * Windows child environment: inherited system settings only. Credentials,
 * envswitch settings, profile/data/temp locations, and PATH are dropped so
 * each test supplies isolated ones. Path holds suite-owned tools, Bun's own
 * directory, and the read-only system directories (never written).
 */
function windowsBaseEnv(): Record<string, string> {
  const dropped = new Set([
    "PATH", "GITHUB_TOKEN", "GH_TOKEN", "REPO", "APP_ROOT", "INSTALL_DIR", "HOME", "USERPROFILE", "LOCALAPPDATA",
    "APPDATA", "TEMP", "TMP", "TMPDIR", "BUN_INSTALL", "GH_CONFIG_DIR", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME", "XDG_STATE_HOME",
  ]);
  const env: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    const upper = name.toUpperCase();
    if (value !== undefined && !dropped.has(upper) && !upper.startsWith("ENVSWITCH_")) {
      env[name] = value;
    }
  }
  return env;
}

function windowsSystemPath(): string[] {
  return [join(SYSTEM_ROOT, "System32"), SYSTEM_ROOT, join(SYSTEM_ROOT, "System32", "WindowsPowerShell", "v1.0")];
}

describe.skipIf(!WINDOWS)("update installer runner (Windows)", () => {
  const FAKE_INSTALLER = [
    "param([string]$AppRoot,[string]$InstallDir,[string]$Repo,[string]$Version,[string]$CurrentVersion)",
    "$dir = Split-Path -Parent $PSCommandPath",
    "$acl = Get-Acl -LiteralPath $dir",
    "$sids = @($acl.Access | ForEach-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } | Sort-Object -Unique)",
    "$current = if ($PSBoundParameters.ContainsKey('CurrentVersion')) { $CurrentVersion } else { '<unset>' }",
    "$lines = @(\"script=$PSCommandPath\", \"cwd=$((Get-Location).ProviderPath)\", \"AppRoot=$AppRoot\", \"InstallDir=$InstallDir\", \"Repo=$Repo\", \"Version=$Version\", \"CurrentVersion=$current\", \"protected=$($acl.AreAccessRulesProtected)\", \"sids=$($sids -join ',')\")",
    "foreach ($name in @('ENVSWITCH_CURRENT_VERSION','ENVSWITCH_VERIFY_ATTESTATIONS','GITHUB_TOKEN','ENVSWITCH_INSTALL_TEST_MODE')) {",
    "  $value = [Environment]::GetEnvironmentVariable($name)",
    "  if ($null -eq $value) { $lines += \"$name unset\" } else { $lines += \"$name=$value\" }",
    "}",
    "[IO.File]::WriteAllLines($env:FAKE_LOG, [string[]]$lines)",
    "if ($env:FAKE_MODE -eq 'echo') { [Console]::Out.WriteLine(\"info: binary: $AppRoot\\current\\bin\\envswitch.exe\") }",
    "[Console]::Out.WriteLine('info: from fake installer')",
    "if ($env:FAKE_MODE -eq 'status') { exit 7 }",
    "exit 0",
    "",
  ].join("\r\n");

  function arrange(label = "") {
    const base = realTempDir("envswitch-winrunner-");
    const root = label === "" ? base : join(base, label);
    mkdirSync(root, { recursive: true });
    const appRoot = join(root, "app");
    const installer = join(appRoot, "current", "scripts", "install.ps1");
    mkdirSync(dirname(installer), { recursive: true });
    writeFileSync(installer, FAKE_INSTALLER, { flag: "wx" });
    const tempRoot = join(root, "tmp");
    mkdirSync(tempRoot);
    mkdirSync(join(root, "home"));
    const log = join(root, "installer.log");
    const plan: InstallPlan = {
      installer,
      repo: "acme/tools",
      appRoot,
      installDir: join(root, "bin"),
      version: "v0.0.5",
      currentVersion: "v0.0.4",
    };
    const env: UpdateEnv = {
      ...windowsBaseEnv(),
      PATH: windowsSystemPath().join(";"),
      USERPROFILE: join(root, "home"),
      LOCALAPPDATA: join(root, "home"),
      APPDATA: join(root, "home"),
      TEMP: tempRoot,
      TMP: tempRoot,
      FAKE_LOG: log,
      GITHUB_TOKEN: "synthetic",
      ENVSWITCH_VERIFY_ATTESTATIONS: "1",
      ENVSWITCH_CURRENT_VERSION: "v0.0.1",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
    };
    const readLog = () =>
      Object.fromEntries(
        readFileSync(log, "utf8")
          .trimEnd()
          .split(/\r?\n/)
          .map((line) => (line.endsWith(" unset") ? [line.slice(0, -6), undefined] : [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)])),
      ) as Record<string, string | undefined>;
    return { base, plan, env, tempRoot, readLog };
  }

  test("non-ASCII profile, app root, install dir, and temp: the real Bun -> Windows PowerShell 5.1 handoff decodes output", () => {
    const { base, plan, env, tempRoot, readLog } = arrange("José-雪");
    // A fresh Bun process runs the handoff exactly as `envswitch update` would.
    const driver = join(base, "run-handoff.ts");
    const input = join(base, "handoff-input.json");
    const output = join(base, "handoff-output.json");
    writeFileSync(
      driver,
      [
        `import { readFileSync, writeFileSync } from "node:fs";`,
        `import { runInstallScript } from ${JSON.stringify(join(REPO_ROOT, "src", "cli.ts"))};`,
        `const input = JSON.parse(readFileSync(process.argv[2], "utf8"));`,
        `const run = await runInstallScript(input.plan, { windows: true, tempRoot: input.tempRoot, stdout: { write: () => {} } });`,
        `writeFileSync(process.argv[3], JSON.stringify(run));`,
        "",
      ].join("\n"),
      { flag: "wx" },
    );
    writeFileSync(input, JSON.stringify({ plan, tempRoot }), { flag: "wx" });
    const result = Bun.spawnSync([process.execPath, driver, input, output], {
      env: { ...env, FAKE_MODE: "echo" } as Record<string, string>,
      stdout: "pipe",
      stderr: "pipe",
      timeout: 120_000,
    });
    expect({ code: result.exitCode, stderr: result.stderr.toString() }).toEqual({ code: 0, stderr: "" });

    const run = JSON.parse(readFileSync(output, "utf8"));
    expect(plan.appRoot).toContain("José-雪");
    expect(run).toEqual({
      code: 0,
      stdout: `info: binary: ${plan.appRoot}\\current\\bin\\envswitch.exe\r\ninfo: from fake installer\r\n`,
    });
    expect(readLog()).toMatchObject({ AppRoot: plan.appRoot, InstallDir: plan.installDir, Version: "v0.0.5" });
    expect(readdirSync(tempRoot)).toEqual([]);
  }, 120_000);

  test("copies the installer outside current, restricts its directory, passes exact parameters, and cleans up", async () => {
    const { plan, env, tempRoot, readLog } = arrange();
    const chunks: Uint8Array[] = [];

    const run = await runInstallScript(plan, { env, windows: true, tempRoot, stdout: { write: (c) => chunks.push(c) } });

    expect(run).toEqual({ code: 0, stdout: "info: from fake installer\r\n" });
    expect(Buffer.concat(chunks).toString()).toBe("info: from fake installer\r\n");
    const log = readLog();
    expect(log.script?.toLowerCase().startsWith(`${tempRoot}\\envswitch-update-`.toLowerCase())).toBe(true);
    expect(log.script?.toLowerCase().endsWith("\\install.ps1")).toBe(true);
    expect(log.cwd?.toLowerCase()).toBe(dirname(log.script!).toLowerCase());
    // Owner-only ACL plus SYSTEM and Administrators, with inheritance removed.
    expect(log.protected).toBe("True");
    const sids = log.sids?.split(",") ?? [];
    expect(sids).toHaveLength(3);
    expect(sids).toContain("S-1-5-18");
    expect(sids).toContain("S-1-5-32-544");
    expect(log).toMatchObject({
      AppRoot: plan.appRoot,
      InstallDir: plan.installDir,
      Repo: "acme/tools",
      Version: "v0.0.5",
      CurrentVersion: "v0.0.4",
      ENVSWITCH_VERIFY_ATTESTATIONS: undefined,
      GITHUB_TOKEN: "synthetic",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
    });
    expect(readdirSync(tempRoot)).toEqual([]);
    expect(readFileSync(plan.installer, "utf8")).toBe(FAKE_INSTALLER);

    await runInstallScript({ ...plan, currentVersion: undefined }, { env, windows: true, tempRoot, stdout: { write: () => {} } });
    expect(readLog()).toMatchObject({ CurrentVersion: "<unset>", ENVSWITCH_CURRENT_VERSION: undefined });
  }, 60_000);

  test("forwards a nonzero status and reports launch failures as controlled errors", async () => {
    const { plan, env, tempRoot } = arrange();

    expect((await runInstallScript(plan, { env: { ...env, FAKE_MODE: "status" }, windows: true, tempRoot, stdout: { write: () => {} } })).code).toBe(7);

    const missing = await runInstallScript({ ...plan, installer: `${plan.installer}.missing` }, { env, windows: true, tempRoot }).catch((e: unknown) => e);
    expect(missing).toBeInstanceOf(EnvswitchError);
    const inside = await runInstallScript(plan, { env, windows: true, tempRoot: join(plan.appRoot, "current") }).catch((e: unknown) => e);
    expect((inside as Error).message).toBe("update failed: could not run installer: temporary directory is inside current");
    expect(readdirSync(tempRoot)).toEqual([]);
  }, 60_000);
});

// ---------------------------------------------------------------------------
// End-to-end migration rows (protocol §7.3, §8, §9): an untouched pre-bridge
// installer (oracle), the TS bridge, and D2b's dual-mode installer served a
// synthetic NATIVE release by D2b's loopback fixture server.
// ---------------------------------------------------------------------------

/**
 * The untouched pre-bridge Unix installer: `git show 53de029:scripts/install.sh`
 * (main before the bridge; unchanged on bridge/m1a), stored as gzip -9n | base64
 * so the oracle survives shallow CI checkouts. Verified by SHA-256 before use.
 */
const ORACLE_INSTALLER_SHA256 = "27244ac150dbfd93ee28ee033684fe983e8dd331d0db397de4b4b4d2ad11a5d4";
const ORACLE_INSTALLER_GZIP_BASE64 =
  "H4sIAAAAAAACA60ZaVPbSPa7f8VD8YZjIitka2pqHcwOBE+ghiMVyGZqgVEJqY01kSWl1QKC8f72fa8PqSXLYXZqv4Dcene/Wy/W" +
  "vLLg3k2ceiy9g5ugmPZ6BRPgsjKDPM7ZJIiTXu/j+MPZyOnP6f/Q5WkSFUESJzFhFfexCKcLp3d0en6xd3zsHxx9JFjr59DtH56d" +
  "jL1BkoVBQuwQfO/DB//j2dkFwZpnBJz/dvDeP9i72PMJpYVZTAPOFg2uhKo5GioLLyw5Z6lweu/OTk72Tg/8072T8cip8Jzeh72L" +
  "Q3//+Ozdr/7++P3R6ch5Abu7u1CBAEHQUQN2fHpAkDs7O21IPKpNcHHyQcrk9HpJduvH6STb2IR5DyDncSomsE5HQ/hbcZWug9Pf" +
  "dnoLBVqUYciKogWtT7sQGOcZb4HLMxsYdl++IYQwm82CNPLZQ1wIw0QfgnunYb2I3XlpmSTwZvfltsRLWJCWuS9meRQbbvEELi/B" +
  "TcG6bKX40F048PIluBG+a75y4Pr6LYgpS5ECAJ+ByyfLUPhyEhPnO/SzKBDM5yw3NpS+AHQwkpaoZFlDQnTswOg/8Ptl4D7uuf9+" +
  "7f7DH7jXP3it3/2mJJUtwYlTyVVyKGKR8W8wyfgsEENQ5NsI7CFnoWARZPcp414Ng3YWsL2sTHBTZEmJD7U1lVJpMGNKKXOCEHjw" +
  "xtLS6eOZA2sj8LZQB3h6sk5HI9gaDLae1a0/J1YL1IjQ2lD6LczKQsANgyAFIzLkgZgCOv40KwWsDwbrnaoSlB9mqQjitFhSUykl" +
  "tSTRh/05xdBiqOSn3wiBP0kPolZMWZL4Oc8mccJ8ot0gp16TyGSl+hfy2MCcxuiZlDo/HB8fk3NuEucQ3+BpDe5ArOz1WEw35UMd" +
  "UiaW5pSOFt4AQXjoaKC3b+UDpc/n8AhmCXHrOSyteQONFUFItmFpUXK22jhofxGghpbN1Qsbo+ViLoWk/Z7CliL6Fn0b3F++4ut2" +
  "BnXaKC0HpIwHjsyVNyjAFwgSzoLoG6hkhHKiU9oEFkpdzkTJU3itXAv/zr6gKuDmyG8Dn/TlNljr+xWYv9t67OyMz37p9ZTH2fIv" +
  "ehjGGRcynVM5sQy3GF5JfZ0mHlaDRY/I2RoGUcQovmx0EJmqEl1KUmJgPJ5885OgTMMpa8aKOVzKde4Dqm3eOisj3kDAfVBAmgnj" +
  "EJSuOMTqjD2wsBTBTcIwI8wNyqI7tLNC+EY7Gee3Jaa1NGSr5LZ7gYXXn9tVedHtkKRoyyq2ttoOS1nGqiRURjpdUJGN01toSQI1" +
  "ecJw2oKC62KuyK0C2aLMyxTW20QV0jq5gOJMMplLCEScpSv8vBWfG8tJULp5R/y3rNCOjoa7coaCoNd/y0oOgvFZnAbJK3IN0kZM" +
  "0UFMg5Bm98Mmsh0xV6171iFz5TRR6ELgLxsK3S9iAoutn+MRVeWGz2UFGS4ryFylTAxPIDisXw7LPGd8eL1Oz0l2L5/tKpAVVfaP" +
  "An4fpyYl15Irxhg2syA8O2+l8CROyweDo5MoBw/tHnqoTYHSN1Lo1xhmccizIpuIJpjlsyv4fz4/9o6Jn5GBJQX7HkIDGJ2ru/hY" +
  "OaNEn8rpahH57Jy6H+UJJnVgc0kW60BEYxYhj3NReBp4kBfbgNp/jtMou6+QqqSyVNBuytSuYmjMZtsKCGAZyWpf8UV3KOkLebAL" +
  "cZnKuQf/t3LnytJtYXSz0QeyZTYJEqFrRdbaquCgknSnbXpD2Zmzr2XM8R4wLDRN2MfwCUqRYUsao9sn35zlNq+GRH4lAcGEZzOY" +
  "CpEXQ4+0GKAwXpSF1VXJIHsFKkRZFf/VrS+Xg0ZoE7O4VWPemkfKt/vKclI1d1KcHy9JY+R+0rMocZMpZv/Tqa+zCxUU62c1IiIN" +
  "WUxaZdwCXdD9mWbTsZqn+pJUHiEXRNCeuTX9W/aT2js37eJiXbbqBLvgqVGfL4eMtJplfzRrGmOuj14hGUGkqso9yUp0dARRvchS" +
  "IC06LqUstOUB26H+XEsmDeX09S+qbCZJNYob2W6fhuMKkox2ywQWZYGFA6cynAyLduGnVxDkMb4u8iwtWK8zjm+nMh9OyZengBYQ" +
  "ZdGePu3gkHTJpoSSx1S7sBfx1Fpi4WlZCk8BklJ/fMUJRQS3sr1fR4I18SeqDCVTJdTkzz8RostBig+BjEcEoXikpPk+FoflDbw7" +
  "PmrHMObANMmCCKxlxNJFqks0BiUufoYpdbQho2azZxUZOX6/P7o4/LTvX5z9Oj6Vw3cjoUFN4QckcQjOHgqb8fhROtwQ9lmA4Q5N" +
  "OjglNWSx75MuQeqOvCvalz9fI2fHhDTCD25xTCxvZGQ/c1errsa+9joxU1q25cHYggLti9ZYL7zBlmPu3Lm8HBZ5ELLh9fXW0P7h" +
  "XG1c/u5cb11tOoMt72rbw87jCaY4kRCV7U2nVTzWZPuphKXtwt0/633Cdf048N3rrfZqQWvQJuk+WjRXte9pZpzpo7KZzgJUk7U1" +
  "ZbcWyA0ReVvE0phFSyUhL28SzCsQgDa+HOLJdStP9Hd0Ftj1C2wHQ4axwwe3jxAUtBScxBwl7aoB7ZKplZLtmvZ3P+DhNL7rShXN" +
  "sVTDqYnUHKrFE579vT5DvyPe+G9UOZ3lcEuuZiTxaMAhvjSKaG4L5/+ToureSzNTY4bhokxeJwfHRDJyMLdSJYjaN1yXogcPSCP6" +
  "iQUFW3UKfU2ZDtX4oyzlfEfIRuNi5xoju5W8KmGiOFIlCLs4SyEky+UohTyx16zgG1n1Oas4VtnqyncgUxb6dy6GmIbyhJIseqmX" +
  "YXcr3EJwFsxktvrzOfGvZ8QqgtdgVQrs42+8kay6Dq+6p+4gN3ev4swq9RxwjLkL4sSM5rbNuneQisanj8cEjoJ0D/GmY1Fx3ghK" +
  "zcLMndsdUWhFJpK5petUhGgGlzs4dUr9l0JaePrIaW5wzCk+mvW902vNmIIHoah5GAGJEuYnbOsfJ3Uk6DWP+84iLiWq5COh9Bvr" +
  "U4Jjb1aI3rzGWHhYNb4Et2zwR5HJcUEvXbtACx56YRIPRLE6qWtZq6jSK4xGMjbaatbP9N5Wl43DZFSGspWMWM5SrAdhzOTgtRFS" +
  "WqmllVt6p6/bPKcaMDDD1ERcFweHR5a6tLWjLcKmnHXM5l5/gJH3eteibr1Fn7vnsWDdS67KsyxvM4D01YEuWrWd6kwqb723Ha1j" +
  "v9TCfn4lZdaH9cqJuJjt4YuOz2YdX83qcqKH05VJueqzr7CvaqBgH8KSeny9enZ+tUh1Dq46K1efioy7DaWAcceKkA70zEHVnT4l" +
  "GUd881OPHJG2hyRb5Uf6Y5y0rR0PCPOzIzemZODpLIvg9U8//ggtMze2nFF7m9UIKiuV2EDfj5Bqo4j9OPpN0zmkw1SBMOsUkBJ7" +
  "E0s6+AxjuGsUMrOVCXvlqXLFaX/eMgXePre/FIHT2uq1NF6BVOXVRo4FaC3S6lUiTbG9xry1NPDJ9lirRdGk3r64k8arOri6sezP" +
  "NfCi2VnKq66Cf2P2RTAMdjeS9NtfU01vQyg8yKH5VRLGvx1dELl2x2k1Ula/ZFNrFsOOqt2AXtHsUrKovX6g9SWEZt6zaGn/1F93" +
  "Kxdl1nAIVbPaXKMqUanEG5bN94YbQTy3fV+90Jedgv7oOB+f/uvcP/98dPHu0K+uZnx+4Z+cHYxlhzUaYbmWPuTqhbNrPh5VIdu9" +
  "rt4mE79xTGJ5LdPK/8hZ7sdXM15enze5UfAiM8xP/wX0pXCEFiEAAA==";

function oracleInstaller(): Uint8Array {
  const bytes = Bun.gunzipSync(Buffer.from(ORACLE_INSTALLER_GZIP_BASE64, "base64"));
  expect(createHash("sha256").update(bytes).digest("hex")).toBe(ORACLE_INSTALLER_SHA256);
  return bytes;
}

const E2E_TOKEN = "synthetic-bridge-token";

const E2E_TOOLS = [
  "bash", "curl", "tar", "gzip", "awk", "sed", "mktemp", "find", "uname", "sysctl", "shasum", "sha256sum", "od",
  "tr", "cat", "cp", "mv", "rm", "rmdir", "mkdir", "chmod", "cmp", "sort", "grep", "cut", "tail", "head", "date",
  "sleep", "readlink", "wc", "ln", "ls", "install", "basename", "dirname", "touch",
];

/** D2b's fixtures.Server/release(), driven over stdin/stdout. */
const MOCK_SERVER_DRIVER = `
import json, sys
sys.path.insert(0, sys.argv[1])
from fixtures import Server, release
config = json.loads(sys.argv[2])
binary = None
if config.get('windowsBinary'):
    with open(config['windowsBinary'], 'rb') as handle:
        binary = handle.read()
server = Server(release(kind='native', ver=config['ver'], fault=config.get('fault'), windows_binary=binary), tag='v' + config['ver'], fault=config.get('fault'))
server.repo = config['repo']
server.statuses = list(config.get('statuses', []))
server.change_latest = bool(config.get('changeLatest'))
print(server.base, flush=True)
sys.stdin.read()
print(json.dumps(server.requests), flush=True)
server.close()
`;

interface ChunkReader {
  read(): Promise<{ done: boolean; value?: Uint8Array | undefined }>;
}

class MockReleaseServer {
  private constructor(
    readonly base: string,
    private readonly proc: Subprocess<"pipe", "pipe", "pipe">,
    private readonly reader: ChunkReader,
    private buffer: string,
  ) {}

  static async start(
    sandbox: { serverEnv(): Record<string, string> },
    config: { ver?: string; repo: string; fault?: string; statuses?: number[]; changeLatest?: boolean; windowsBinary?: string },
  ): Promise<MockReleaseServer> {
    const python =
      processEnv("INSTALL_TEST_PYTHON") || Bun.which(WINDOWS ? "python" : "python3", { PATH: processEnv("PATH") ?? "" });
    if (!python) {
      throw new Error("Python 3 is required for the installer fixture server (set INSTALL_TEST_PYTHON)");
    }
    const proc = Bun.spawn(
      [python, "-I", "-B", "-c", MOCK_SERVER_DRIVER, join(REPO_ROOT, "test", "install"), JSON.stringify({ ver: "0.0.42", ...config })],
      { env: sandbox.serverEnv(), stdin: "pipe", stdout: "pipe", stderr: "pipe" },
    );
    const reader: ChunkReader = proc.stdout.getReader();
    let buffer = "";
    while (!buffer.includes("\n")) {
      const { value, done } = await reader.read();
      if (done || value === undefined) {
        throw new Error(`fixture server failed: ${await new Response(proc.stderr).text()}`);
      }
      buffer += Buffer.from(value).toString();
    }
    const newline = buffer.indexOf("\n");
    return new MockReleaseServer(buffer.slice(0, newline).trim(), proc, reader, buffer.slice(newline + 1));
  }

  hooks(): Record<string, string> {
    return { ENVSWITCH_INSTALL_TEST_MODE: "install", ENVSWITCH_INSTALL_TEST_API_BASE_URL: this.base };
  }

  /** Stop serving and return the recorded [path, Authorization] requests. */
  async close(): Promise<Array<[string, string | null]>> {
    this.proc.stdin.end();
    for (;;) {
      const { value, done } = await this.reader.read();
      if (done || value === undefined) {
        break;
      }
      this.buffer += Buffer.from(value).toString();
    }
    await this.proc.exited;
    return JSON.parse(this.buffer) as Array<[string, string | null]>;
  }
}

interface ProcessResult {
  code: number;
  stdout: string;
  stderr: string;
}

function expectSuccess(result: ProcessResult): void {
  if (result.code !== 0) {
    throw new Error(`process failed (${result.code})\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`);
  }
}

class BridgeSandbox {
  readonly root = realTempDir("envswitch-bridge-");
  readonly home = join(this.root, "home");
  readonly tools = join(this.root, "tools");
  readonly tmp = join(this.root, "tmp");
  readonly project = join(this.root, "project");
  readonly ghFixture = join(this.root, "gh-fixture");
  readonly appRoot = join(this.home, ".local", "share", "envswitch");
  readonly appDir = join(this.appRoot, "current");
  readonly binDir = join(this.home, ".local", "bin");
  readonly launcher = join(this.binDir, "envswitch");
  readonly env: Record<string, string>;

  constructor() {
    for (const dir of [this.home, this.tools, this.tmp, join(this.project, ".git"), join(this.ghFixture, "assets"), this.binDir]) {
      mkdirSync(dir, { recursive: true });
    }
    writeFileSync(join(this.project, ".env"), "SECRET=keep\n");
    for (const name of E2E_TOOLS) {
      const path = Bun.which(name, { PATH: process.env.PATH ?? "" });
      if (path && name !== "curl") {
        symlinkSync(path, join(this.tools, name));
      }
    }
    const realCurl = Bun.which("curl", { PATH: process.env.PATH ?? "" });
    if (!realCurl) {
      throw new Error("curl is required for the installer end-to-end tests");
    }
    // Public GitHub is never contacted: the old installer's direct-download
    // fallback gets a deterministic 404; loopback fixture URLs pass through.
    this.fake(
      "curl",
      `case " $* " in *" https://github.com/"*|*" https://api.github.com/"*) printf 'curl: (22) The requested URL returned error: 404\\n' >&2; exit 22;; esac\nexec ${shellQuote(realCurl)} "$@"`,
    );
    // Real Bun runs the bridge; dependency installation links the checkout's
    // node_modules (read-only) instead of contacting a registry.
    this.fake(
      "bun",
      `if [[ \${1:-} == install ]]; then printf '%s\\n' "$*" >> "$HOME/bun-install.log"; ln -s ${shellQuote(join(REPO_ROOT, "node_modules"))} node_modules; exit 0; fi\nexec ${shellQuote(process.execPath)} "$@"`,
    );
    // Authenticated fake gh for the old installer, reading suite-owned files only.
    this.fake(
      "gh",
      [
        `fixture=${shellQuote(this.ghFixture)}`,
        `printf '%s\\n' "$*" >> "$fixture/gh.log"`,
        `case "\${1:-} \${2:-}" in`,
        `  "auth status") exit 0;;`,
        `  "release download")`,
        `    pattern=''; dir=''; shift 3`,
        `    while [[ $# -gt 0 ]]; do case "$1" in --pattern) pattern=$2; shift 2;; --dir) dir=$2; shift 2;; *) shift;; esac; done`,
        `    [[ -f "$fixture/assets/$pattern" ]] || { echo "no asset matching $pattern" >&2; exit 1; }`,
        `    cp "$fixture/assets/$pattern" "$dir/$pattern"; exit 0;;`,
        `esac`,
        `[[ \${1:-} == api ]] && { cat "$fixture/tag"; exit 0; }`,
        `exit 1`,
      ].join("\n"),
    );
    this.fake("op", 'printf forbidden >> "$HOME/op.log"; exit 99');
    this.env = {
      HOME: this.home,
      USERPROFILE: this.home,
      LOCALAPPDATA: join(this.root, "localappdata"),
      APPDATA: join(this.root, "appdata"),
      XDG_DATA_HOME: join(this.home, ".local", "share"),
      XDG_CONFIG_HOME: join(this.root, "config"),
      XDG_CACHE_HOME: join(this.root, "cache"),
      XDG_STATE_HOME: join(this.root, "state"),
      TMPDIR: this.tmp,
      TEMP: this.tmp,
      TMP: this.tmp,
      BUN_INSTALL: join(this.root, "bun-install"),
      GH_CONFIG_DIR: join(this.root, "gh-config"),
      SHELL: "/bin/bash",
      PATH: `${this.tools}:${this.binDir}`,
      HTTPS_PROXY: "http://127.0.0.1:9",
      https_proxy: "http://127.0.0.1:9",
      NO_PROXY: "127.0.0.1,localhost,::1",
      no_proxy: "127.0.0.1,localhost,::1",
      NO_COLOR: "1",
      COPYFILE_DISABLE: "1",
    };
  }

  /** Environment for D2b's loopback fixture server process. */
  serverEnv(): Record<string, string> {
    return { HOME: this.root, PATH: this.tools };
  }

  /** Create a suite-owned fake tool; exclusive create never writes through a link. */
  fake(name: string, body: string): void {
    writeFileSync(join(this.tools, name), `#!/bin/bash\n${body}\n`, { flag: "wx", mode: 0o755 });
  }

  run(argv: string[], extra: Record<string, string> = {}, stdin?: Uint8Array): ProcessResult {
    const result = Bun.spawnSync(argv, {
      cwd: this.project,
      env: { ...this.env, ...extra },
      stdin: stdin ?? "ignore",
      stdout: "pipe",
      stderr: "pipe",
      timeout: 120_000,
    });
    return { code: result.exitCode ?? -1, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
  }

  /** Publish a release for the fake gh (old installer); assets map basename -> bytes. */
  publishForGh(tag: string, assets: Record<string, Uint8Array>): void {
    for (const name of readdirSync(join(this.ghFixture, "assets"))) {
      unlinkSync(join(this.ghFixture, "assets", name));
    }
    writeFileSync(join(this.ghFixture, "tag"), `${tag}\n`);
    for (const [name, bytes] of Object.entries(assets)) {
      writeFileSync(join(this.ghFixture, "assets", name), bytes);
    }
  }

  sourceArchive(ver: string, files: Record<string, string | Uint8Array>): Uint8Array {
    const stage = join(this.root, "build", ver);
    for (const [name, bytes] of Object.entries(files)) {
      mkdirSync(dirname(join(stage, "envswitch", name)), { recursive: true });
      writeFileSync(join(stage, "envswitch", name), bytes);
    }
    const out = join(this.root, "build", `envswitch_${ver}_source.tar.gz`);
    expectSuccess(this.run([join(this.tools, "tar"), "-czf", out, "-C", stage, "envswitch"]));
    return readFileSync(out);
  }

  /** S0: a fresh pre-bridge install made by the untouched old installer. */
  installPreBridge(extra: Record<string, string> = {}): void {
    const files = {
      "package.json": '{"name":"envswitch"}\n',
      "bun.lock": "",
      "src/cli.ts": "// pre-bridge envswitch fixture\n",
      "scripts/install.sh": oracleInstaller(),
      "scripts/install.ps1": "# pre-bridge fixture\n",
    };
    this.publishForGh("v0.0.40", { "envswitch_0.0.40_source.tar.gz": this.sourceArchive("0.0.40", files) });
    writeFileSync(join(this.root, "readme-oracle-install.sh"), oracleInstaller());
    expectSuccess(this.run(["bash", join(this.root, "readme-oracle-install.sh")], extra));
    expect(readFileSync(this.launcher, "utf8")).toBe(legacyLauncher(this.appDir));
    expect(readFileSync(join(this.appDir, ".version"), "utf8")).toBe("v0.0.40\n");
  }

  /** S0 -> S1: the old CLI's update re-runs its bundled old installer in place with APP_ROOT. */
  oldUpdate(extra: Record<string, string> = {}): ProcessResult {
    return this.run(["bash", join(this.appDir, "scripts", "install.sh")], { APP_ROOT: this.appRoot, ...extra });
  }

  bridgeFiles(): Record<string, Uint8Array> {
    const files: Record<string, Uint8Array> = {};
    for (const name of ["package.json", "bun.lock", "README.md", "tsconfig.json", "scripts/install.sh", "scripts/install.ps1"]) {
      files[name] = readFileSync(join(REPO_ROOT, name));
    }
    for (const name of readdirSync(join(REPO_ROOT, "src"))) {
      files[`src/${name}`] = readFileSync(join(REPO_ROOT, "src", name));
    }
    return files;
  }

  /** Old -> bridge: the old installer installs the bridge source release (no receipt). */
  installBridge(extra: Record<string, string> = {}): void {
    this.installPreBridge(extra);
    this.publishForGh("v0.0.41", { "envswitch_0.0.41_source.tar.gz": this.sourceArchive("0.0.41", this.bridgeFiles()) });
    const result = this.oldUpdate(extra);
    expectSuccess(result);
    expect(result.stdout).toContain("success: installed envswitch v0.0.41\n");
    expect(readFileSync(join(this.appDir, "src", "cli.ts"))).toEqual(readFileSync(join(REPO_ROOT, "src", "cli.ts")));
    expect(readFileSync(join(this.appDir, "scripts", "install.sh"))).toEqual(readFileSync(join(REPO_ROOT, "scripts", "install.sh")));
    expect(readFileSync(join(this.appDir, ".version"), "utf8")).toBe("v0.0.41\n");
    expect(existsSync(join(this.appDir, "install.json"))).toBe(false);
    expect(readFileSync(this.launcher, "utf8")).toBe(legacyLauncher(this.appDir));
  }

  snapshot() {
    return {
      launcher: readFileSync(this.launcher, "utf8"),
      version: readFileSync(join(this.appDir, ".version"), "utf8"),
      cli: readFileSync(join(this.appDir, "src", "cli.ts"), "utf8"),
      receipt: existsSync(join(this.appDir, "install.json")),
    };
  }

  leftovers(): string[] {
    return [
      ...readdirSync(this.tmp).filter((name) => name.startsWith("envswitch-update-")),
      ...readdirSync(this.appRoot).filter((name) => name.startsWith(".install")),
    ];
  }
}

describe.skipIf(WINDOWS)("bridge migration end-to-end", () => {
  test("old installer -> bridge source -> bridge update -> native launcher and receipt", async () => {
    const sandbox = new BridgeSandbox();
    sandbox.installBridge();
    const server = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO });
    let requests: Array<[string, string | null]>;
    try {
      // Stale caller pins/labels/attestation requests must not reach the installer.
      const stale = { ENVSWITCH_VERSION: "v0.0.1", ENVSWITCH_CURRENT_VERSION: "v0.0.7", ENVSWITCH_VERIFY_ATTESTATIONS: "1" };
      const update = sandbox.run([sandbox.launcher, "update"], { ...server.hooks(), ...stale, GITHUB_TOKEN: E2E_TOKEN });
      expectSuccess(update);
      expect(update.stderr).toBe("envswitch [info] updating envswitch v0.0.41 -> v0.0.42\n");
      expect(update.stdout).toContain("info: resolved envswitch v0.0.42 (native)\n");
      expect(update.stdout).toContain("success: installed envswitch v0.0.42\n");
      expect(update.stdout.endsWith("info: run 'envswitch --help' to verify the installation\nupdated envswitch v0.0.41 -> v0.0.42\n")).toBe(true);

      const receipt = JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"));
      expect(receipt).toMatchObject({
        schemaVersion: 1,
        kind: "native",
        repo: DEFAULT_REPO,
        appRoot: sandbox.appRoot,
        installDir: sandbox.binDir,
        launcherPath: sandbox.launcher,
        tag: "v0.0.42",
      });
      expect(readFileSync(sandbox.launcher, "utf8")).toBe(nativeLauncher(receipt.payloadPath));
      expect(readFileSync(join(sandbox.appDir, ".version"), "utf8")).toBe("v0.0.42\n");
      expect(sandbox.run([sandbox.launcher, "--version"])).toEqual({
        code: 0,
        stdout: `envswitch 0.0.42 ${receipt.target}\n`,
        stderr: "",
      });
      // Light migration keeps residual source files; no stub, no recursive prune.
      expect(readFileSync(join(sandbox.appDir, "src", "cli.ts"))).toEqual(readFileSync(join(REPO_ROOT, "src", "cli.ts")));
      expect(sandbox.leftovers()).toEqual([]);

      // A residual TS entry point reaching the native receipt: equality still runs the installer.
      const bridge = ["bun", join(sandbox.appDir, "src", "cli.ts"), "update"];
      const unchanged = sandbox.run(bridge, server.hooks());
      expectSuccess(unchanged);
      expect(unchanged.stderr).toBe("envswitch [info] updating envswitch v0.0.42 -> v0.0.42\n");
      expect(unchanged.stdout.endsWith(
        "success: envswitch is already up to date (v0.0.42)\nenvswitch is already up to date (v0.0.42)\n",
      )).toBe(true);

      // Missing receipt: the protocol native launcher identifies the directory and the installer repairs.
      unlinkSync(join(sandbox.appDir, "install.json"));
      const repaired = sandbox.run(bridge, server.hooks());
      expectSuccess(repaired);
      expect(repaired.stdout).toContain("success: repaired envswitch v0.0.42\n");
      expect(repaired.stdout.endsWith("repaired envswitch v0.0.42\n")).toBe(true);
      expect(JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"))).toEqual(receipt);
    } finally {
      requests = await server.close();
    }

    const paths = requests.map(([path]) => path);
    // One latest selection per bridge run; every installer lookup used the pin.
    expect(paths.filter((path) => path.endsWith("/releases/latest"))).toEqual(
      Array(3).fill(`/repos/${DEFAULT_REPO}/releases/latest`),
    );
    expect(paths.filter((path) => path.includes("/releases/tags/"))).toEqual(
      Array(3).fill(`/repos/${DEFAULT_REPO}/releases/tags/v0.0.42`),
    );
    // The bridge never sends GITHUB_TOKEN to the loopback override; the first
    // run's installer requests show the token was present in its environment.
    expect(requests.filter(([path]) => path.endsWith("/releases/latest")).map(([, auth]) => auth)).toEqual([null, null, null]);
    expect(requests.find(([path]) => path.includes("/releases/tags/"))?.[1]).toBe(`Bearer ${E2E_TOKEN}`);
    expect(existsSync(join(sandbox.home, "op.log"))).toBe(false);
    expect(readFileSync(join(sandbox.project, ".env"), "utf8")).toBe("SECRET=keep\n");
  }, 240_000);

  test("bridge failures leave the bridge install usable: no latest, rejected release, latest changed after pinning", async () => {
    const sandbox = new BridgeSandbox();
    sandbox.installBridge();
    const before = sandbox.snapshot();
    const update = [sandbox.launcher, "update"];

    const noLatest = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, statuses: [404] });
    const unresolved = sandbox.run(update, noLatest.hooks());
    expect((await noLatest.close()).map(([path]) => path)).toEqual([`/repos/${DEFAULT_REPO}/releases/latest`]);
    expect(unresolved).toEqual({
      code: 1,
      stdout: "",
      stderr: `envswitch: no GitHub Release found for ${DEFAULT_REPO}, or access is denied\n`,
    });

    const corrupt = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, fault: "mismatch" });
    const rejected = sandbox.run(update, corrupt.hooks());
    await corrupt.close();
    expect(rejected.code).toBe(1);
    expect(rejected.stderr.startsWith("envswitch [info] updating envswitch v0.0.41 -> v0.0.42\n")).toBe(true);
    expect(rejected.stderr).toMatch(/error: SHA-256 mismatch for envswitch_0\.0\.42_[a-z0-9_-]+\.tar\.gz\n/);
    expect(rejected.stderr.endsWith("envswitch: update failed: installer exited with code 1\n")).toBe(true);
    expect(rejected.stdout).not.toContain("success:");
    expect(rejected.stdout).not.toContain("updated envswitch");

    const moving = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, changeLatest: true });
    const changed = sandbox.run(update, moving.hooks());
    const movingPaths = (await moving.close()).map(([path]) => path);
    expect(movingPaths).toEqual([`/repos/${DEFAULT_REPO}/releases/latest`, `/repos/${DEFAULT_REPO}/releases/tags/v0.0.42`]);
    expect(changed.code).toBe(1);
    expect(changed.stderr).toContain(`error: invalid envswitch release metadata for ${DEFAULT_REPO}\n`);
    expect(changed.stderr.endsWith("envswitch: update failed: installer exited with code 1\n")).toBe(true);

    expect(sandbox.snapshot()).toEqual(before);
    expect(sandbox.leftovers()).toEqual([]);
    const help = sandbox.run([sandbox.launcher, "--help"]);
    expect(help.code).toBe(0);
    expect(help.stdout).toContain("envswitch update");
  }, 240_000);

  test("skipped bridge: the old updater fails on a native latest and the README installer recovers", async () => {
    const sandbox = new BridgeSandbox();
    const custom = { REPO: "demo/suite" };
    sandbox.installPreBridge(custom);
    const before = sandbox.snapshot();

    // S2 latest is native: no source archive exists for the old installer.
    sandbox.publishForGh("v0.0.42", { SHA256SUMS: Buffer.from("fixture\n") });
    const old = sandbox.oldUpdate(custom);
    expect(old.code).toBe(1);
    expect(old.stdout).toContain("info: GitHub CLI download did not find envswitch_0.0.42_source.tar.gz; trying direct download\n");
    expect(old.stderr.endsWith(
      "error: release asset not found or unavailable: envswitch_0.0.42_source.tar.gz\n" +
        "error: expected asset URL: https://github.com/demo/suite/releases/download/v0.0.42/envswitch_0.0.42_source.tar.gz\n",
    )).toBe(true);
    expect(sandbox.snapshot()).toEqual(before);

    // Recovery per README: pipe the main installer to bash with the original custom repository.
    const server = await MockReleaseServer.start(sandbox, { repo: "demo/suite" });
    let reinstall: ProcessResult;
    try {
      reinstall = sandbox.run(["bash"], { ...custom, ...server.hooks() }, readFileSync(join(REPO_ROOT, "scripts", "install.sh")));
    } finally {
      await server.close();
    }
    expectSuccess(reinstall);
    expect(reinstall.stdout).toContain("success: installed envswitch v0.0.42\n");
    const receipt = JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"));
    expect(receipt).toMatchObject({ kind: "native", repo: "demo/suite", installDir: sandbox.binDir, tag: "v0.0.42" });
    expect(readFileSync(sandbox.launcher, "utf8")).toBe(nativeLauncher(receipt.payloadPath));
    expect(sandbox.run([sandbox.launcher, "--version"]).stdout).toBe(`envswitch 0.0.42 ${receipt.target}\n`);
    // Bun and the user's project .env are untouched.
    expect(lstatSync(join(sandbox.tools, "bun")).isFile()).toBe(true);
    expect(readFileSync(join(sandbox.project, ".env"), "utf8")).toBe("SECRET=keep\n");
  }, 240_000);
});

/** Exactly what the pre-bridge PowerShell installer's Set-Content -Encoding ASCII wrote. */
function windowsLegacyLauncher(appDir: string): string {
  const cli = `${appDir}\\src\\cli.ts`;
  return [
    "@echo off",
    "setlocal",
    "where bun >NUL 2>NUL",
    "if %ERRORLEVEL% EQU 0 (",
    `  bun "${cli}" %*`,
    "  exit /b %ERRORLEVEL%",
    ")",
    'if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" (',
    `  "%USERPROFILE%\\.bun\\bin\\bun.exe" "${cli}" %*`,
    "  exit /b %ERRORLEVEL%",
    ")",
    "echo envswitch: bun is not installed or not on PATH 1>&2",
    "exit /b 127",
    "",
  ].join("\r\n");
}

/** D2b install.ps1 New-Launcher output (UTF-8, CRLF); checked against real PowerShell. */
function windowsSourceLauncher(cli: string): string {
  const path = cli.replace(/%/g, "%%");
  return [
    "@echo off",
    "setlocal DisableDelayedExpansion",
    "for /f \"tokens=2 delims=:.\" %%C in ('chcp') do set \"_ENVSWITCH_CP=%%C\"",
    "chcp 65001 >nul",
    "where bun >NUL 2>NUL",
    "if %ERRORLEVEL% EQU 0 goto envswitch_bun_path",
    'if exist "%USERPROFILE%\\.bun\\bin\\bun.exe" goto envswitch_bun_home',
    "echo envswitch: bun is not installed or not on PATH 1>&2",
    'set "_ENVSWITCH_EXIT=127"',
    "goto envswitch_finish",
    ":envswitch_bun_path",
    `bun "${path}" %*`,
    'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"',
    "goto envswitch_finish",
    ":envswitch_bun_home",
    `"%USERPROFILE%\\.bun\\bin\\bun.exe" "${path}" %*`,
    'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"',
    ":envswitch_finish",
    "chcp %_ENVSWITCH_CP% >nul 2>&1",
    "exit /b %_ENVSWITCH_EXIT%",
    "",
  ].join("\r\n");
}

let windowsPayloadPath: string | undefined;

/** A real .NET Framework console EXE (as D2b's Windows suite builds), compiled once. */
function windowsPayload(): string {
  if (windowsPayloadPath !== undefined) {
    return windowsPayloadPath;
  }
  const root = realTempDir("envswitch-winpayload-");
  const out = join(root, "payload-0.0.42.exe");
  const code =
    'public class Payload { public static int Main(string[] args) { if (args.Length > 0 && args[0] == "--version") { System.Console.Write("envswitch 0.0.42 x86_64-pc-windows-msvc\\n"); return 0; } if (args.Length > 0 && args[0] == "--help") { return 0; } foreach (var arg in args) { System.Console.Write(arg + "\\n"); } return 23; } }';
  const script = join(root, "build.ps1");
  writeFileSync(
    script,
    `Add-Type -TypeDefinition '${code}' -OutputAssembly '${out.replace(/'/g, "''")}' -OutputType ConsoleApplication\r\n`,
    { flag: "wx" },
  );
  const result = Bun.spawnSync([WINDOWS_POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script], {
    env: { ...windowsBaseEnv(), PATH: windowsSystemPath().join(";"), USERPROFILE: root, LOCALAPPDATA: root, APPDATA: root, TEMP: root, TMP: root },
    stdout: "pipe",
    stderr: "pipe",
  });
  if (result.exitCode !== 0) {
    throw new Error(`payload compile failed: ${result.stdout.toString()}${result.stderr.toString()}`);
  }
  windowsPayloadPath = out;
  return out;
}

function lf(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

/**
 * Isolated Windows profile: USERPROFILE/LOCALAPPDATA/APPDATA/TEMP are suite
 * paths, and every installer run uses the mocked persistent User PATH file.
 */
class WindowsBridgeSandbox {
  readonly root: string;
  readonly home: string;
  readonly localAppData: string;
  readonly tools: string;
  readonly temp: string;
  readonly project: string;
  readonly appRoot: string;
  readonly appDir: string;
  readonly binDir: string;
  readonly launcher: string;
  readonly userPathFile: string;
  readonly env: Record<string, string>;

  /** `label` (e.g. non-ASCII) becomes part of every profile, temp, and install path. */
  constructor(label = "") {
    const base = realTempDir("envswitch-winbridge-");
    this.root = label === "" ? base : join(base, label);
    this.home = join(this.root, "home");
    this.localAppData = join(this.root, "localappdata");
    this.tools = join(this.root, "tools");
    this.temp = join(this.root, "temp");
    this.project = join(this.root, "project");
    this.appRoot = join(this.localAppData, "Programs", "envswitch");
    this.appDir = join(this.appRoot, "current");
    this.binDir = join(this.appRoot, "bin");
    this.launcher = join(this.binDir, "envswitch.cmd");
    this.userPathFile = join(this.home, "user-path.txt");
    for (const dir of [this.home, this.localAppData, this.tools, this.temp, join(this.project, ".git"), this.appDir, this.binDir]) {
      mkdirSync(dir, { recursive: true });
    }
    writeFileSync(join(this.project, ".env"), "SECRET=keep\r\n");
    writeFileSync(join(this.tools, "gh.cmd"), "@echo off\r\nexit /b 1\r\n", { flag: "wx" });
    writeFileSync(join(this.tools, "op.cmd"), '@echo off\r\necho forbidden>>"%USERPROFILE%\\op.log"\r\nexit /b 99\r\n', { flag: "wx" });
    this.env = {
      ...windowsBaseEnv(),
      HOME: this.home,
      USERPROFILE: this.home,
      LOCALAPPDATA: this.localAppData,
      APPDATA: join(this.root, "appdata"),
      TEMP: this.temp,
      TMP: this.temp,
      TMPDIR: this.temp,
      BUN_INSTALL: join(this.root, "bun-install"),
      GH_CONFIG_DIR: join(this.root, "gh-config"),
      XDG_DATA_HOME: join(this.root, "xdg-data"),
      XDG_CONFIG_HOME: join(this.root, "xdg-config"),
      XDG_CACHE_HOME: join(this.root, "xdg-cache"),
      XDG_STATE_HOME: join(this.root, "xdg-state"),
      PATH: [this.tools, dirname(process.execPath), ...windowsSystemPath()].join(";"),
      HTTPS_PROXY: "http://127.0.0.1:9",
      NO_PROXY: "127.0.0.1,localhost,::1",
      NO_COLOR: "1",
      ENVSWITCH_INSTALL_TEST_MODE: "install",
      ENVSWITCH_INSTALL_TEST_USER_PATH_FILE: this.userPathFile,
    };
  }

  serverEnv(): Record<string, string> {
    return {
      ...windowsBaseEnv(),
      PATH: windowsSystemPath().join(";"),
      HOME: this.home,
      USERPROFILE: this.home,
      LOCALAPPDATA: this.localAppData,
      APPDATA: join(this.root, "appdata"),
      TEMP: this.temp,
      TMP: this.temp,
    };
  }

  run(argv: string[], extra: Record<string, string> = {}): ProcessResult {
    const result = Bun.spawnSync(argv, {
      cwd: this.project,
      env: { ...this.env, ...extra },
      stdout: "pipe",
      stderr: "pipe",
      timeout: 180_000,
    });
    return { code: result.exitCode ?? -1, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
  }

  /** The user's path: cmd runs the managed launcher. (The pre-bridge launcher masks exit codes.) */
  runLauncher(args: string[], extra: Record<string, string> = {}): ProcessResult {
    return this.run([join(SYSTEM_ROOT, "System32", "cmd.exe"), "/d", "/c", this.launcher, ...args], extra);
  }

  /** The installed TS entry point run directly, for exact exit statuses. */
  bridge(args: string[], extra: Record<string, string> = {}): ProcessResult {
    return this.run([process.execPath, join(this.appDir, "src", "cli.ts"), ...args], extra);
  }

  /**
   * "legacy": a pre-receipt install as the old PowerShell installer left it
   * (ASCII-only launcher). "receipt": a bridge source install as the
   * dual-mode installer leaves it (protocol launcher, source receipt).
   */
  installCopy(kind: "pre-bridge" | "bridge", tag: string, layout: "legacy" | "receipt" = "legacy"): void {
    const files: Record<string, Uint8Array | string> =
      kind === "bridge"
        ? Object.fromEntries(
            ["package.json", "bun.lock", "README.md", "tsconfig.json", "scripts/install.sh", "scripts/install.ps1"]
              .map((name) => [name, readFileSync(join(REPO_ROOT, name))] as const)
              .concat(readdirSync(join(REPO_ROOT, "src")).map((name) => [`src/${name}`, readFileSync(join(REPO_ROOT, "src", name))] as const)),
          )
        : {
            "package.json": '{"name":"envswitch"}\n',
            "bun.lock": "",
            "src/cli.ts": "// pre-bridge envswitch fixture\n",
            "scripts/install.ps1": "# pre-bridge fixture\r\n",
          };
    for (const [name, bytes] of Object.entries(files)) {
      mkdirSync(dirname(join(this.appDir, name)), { recursive: true });
      writeFileSync(join(this.appDir, name), bytes);
    }
    if (kind === "bridge") {
      // Read-only dependency tree from the checkout (a junction needs no privilege).
      symlinkSync(join(REPO_ROOT, "node_modules"), join(this.appDir, "node_modules"), "junction");
    }
    if (layout === "legacy") {
      writeFileSync(join(this.appDir, ".version"), `${tag}\r\n`);
      writeFileSync(this.launcher, windowsLegacyLauncher(this.appDir), { flag: "wx" });
      return;
    }
    const cli = join(this.appDir, "src", "cli.ts");
    writeFileSync(join(this.appDir, ".version"), `${tag}\n`);
    writeFileSync(this.launcher, windowsSourceLauncher(cli), { flag: "wx" });
    const receipt = {
      schemaVersion: 1,
      kind: "source",
      repo: DEFAULT_REPO,
      appRoot: this.appRoot,
      installDir: this.binDir,
      launcherPath: this.launcher,
      target: null,
      tag,
      payloadPath: cli,
    };
    writeFileSync(join(this.appDir, "install.json"), `${JSON.stringify(receipt, null, 4)}\n`);
  }

  /** Read-only check that the real persistent User PATH is never written. */
  realUserPath(): string {
    const result = Bun.spawnSync(
      [WINDOWS_POWERSHELL, "-NoProfile", "-Command", "[Console]::Out.Write([Environment]::GetEnvironmentVariable('Path','User'))"],
      { env: this.serverEnv(), stdout: "pipe", stderr: "pipe" },
    );
    expect(result.exitCode).toBe(0);
    return result.stdout.toString();
  }

  snapshot() {
    return {
      launcher: readFileSync(this.launcher, "utf8"),
      version: readFileSync(join(this.appDir, ".version"), "utf8"),
      cli: readFileSync(join(this.appDir, "src", "cli.ts"), "utf8"),
      receipt: existsSync(join(this.appDir, "install.json")),
      userPathFile: existsSync(this.userPathFile),
    };
  }

  leftovers(): string[] {
    return [
      ...readdirSync(this.temp).filter((name) => name.startsWith("envswitch-update-")),
      ...readdirSync(this.appRoot).filter((name) => name.startsWith(".install")),
    ];
  }
}

describe.skipIf(!WINDOWS)("bridge migration end-to-end (Windows)", () => {
  test("bridge launcher -> native launcher, receipt, and mocked User PATH; equality and repair still run the installer", async () => {
    const sandbox = new WindowsBridgeSandbox();
    sandbox.installCopy("bridge", "v0.0.41");
    const realUserPath = sandbox.realUserPath();
    const server = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, windowsBinary: windowsPayload() });
    let requests: Array<[string, string | null]>;
    try {
      const stale = { ENVSWITCH_VERSION: "v0.0.1", ENVSWITCH_CURRENT_VERSION: "v0.0.7", ENVSWITCH_VERIFY_ATTESTATIONS: "1" };
      const update = sandbox.runLauncher(["update"], { ...server.hooks(), ...stale, GITHUB_TOKEN: E2E_TOKEN });
      expectSuccess(update);
      expect(lf(update.stderr)).toContain("envswitch [info] updating envswitch v0.0.41 -> v0.0.42\n");
      expect(update.stderr).not.toContain("error:");
      expect(lf(update.stdout)).toContain("info: resolved envswitch v0.0.42 (native)\n");
      expect(lf(update.stdout)).toContain("success: installed envswitch v0.0.42\n");
      expect(lf(update.stdout).endsWith("updated envswitch v0.0.41 -> v0.0.42\n")).toBe(true);

      const receipt = JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"));
      expect(receipt).toMatchObject({ schemaVersion: 1, kind: "native", repo: DEFAULT_REPO, target: "x86_64-pc-windows-msvc", tag: "v0.0.42" });
      expect(receipt.appRoot.toLowerCase()).toBe(sandbox.appRoot.toLowerCase());
      expect(receipt.installDir.toLowerCase()).toBe(sandbox.binDir.toLowerCase());
      expect(receipt.launcherPath.toLowerCase()).toBe(sandbox.launcher.toLowerCase());
      const launcher = readFileSync(sandbox.launcher);
      expect(isManagedLauncher(launcher, sandbox.appRoot, true)).toBe(true);
      expect(launcher.toString()).toContain(`"${receipt.payloadPath.replace(/%/g, "%%")}" %*`);
      expect(readFileSync(join(sandbox.appDir, ".version"), "utf8")).toBe("v0.0.42\n");
      expect(sandbox.runLauncher(["--version"]).stdout.trim()).toBe("envswitch 0.0.42 x86_64-pc-windows-msvc");
      expect(readFileSync(sandbox.userPathFile, "utf8").toLowerCase()).toContain(sandbox.binDir.toLowerCase());
      expect(existsSync(join(sandbox.appDir, "src", "cli.ts"))).toBe(true);
      expect(sandbox.leftovers()).toEqual([]);

      const unchanged = sandbox.bridge(["update"], server.hooks());
      expectSuccess(unchanged);
      expect(lf(unchanged.stderr)).toContain("envswitch [info] updating envswitch v0.0.42 -> v0.0.42\n");
      expect(lf(unchanged.stdout).endsWith(
        "success: envswitch is already up to date (v0.0.42)\nenvswitch is already up to date (v0.0.42)\n",
      )).toBe(true);

      unlinkSync(join(sandbox.appDir, "install.json"));
      const repaired = sandbox.bridge(["update"], server.hooks());
      expectSuccess(repaired);
      expect(lf(repaired.stdout)).toContain("success: repaired envswitch v0.0.42\n");
      expect(lf(repaired.stdout).endsWith("repaired envswitch v0.0.42\n")).toBe(true);
    } finally {
      requests = await server.close();
    }

    const latest = requests.filter(([path]) => path.endsWith("/releases/latest"));
    expect(latest).toEqual(Array(3).fill([`/repos/${DEFAULT_REPO}/releases/latest`, null]));
    expect(requests.filter(([path]) => path.includes("/releases/tags/")).map(([path]) => path)).toEqual(
      Array(3).fill(`/repos/${DEFAULT_REPO}/releases/tags/v0.0.42`),
    );
    expect(sandbox.realUserPath()).toBe(realUserPath);
    expect(existsSync(join(sandbox.home, "op.log"))).toBe(false);
    expect(readFileSync(join(sandbox.project, ".env"), "utf8")).toBe("SECRET=keep\r\n");
  }, 300_000);

  test("non-ASCII USERPROFILE, app root, install dir, and temp: receipt, launcher, and printed lines decode correctly", async () => {
    // The old PowerShell installer wrote ASCII-only launchers, so a non-ASCII
    // bridge install is the dual-mode installer's receipt-bearing layout.
    const sandbox = new WindowsBridgeSandbox("José-雪");
    sandbox.installCopy("bridge", "v0.0.41", "receipt");
    expect(sandbox.appRoot).toContain("José-雪");
    const realUserPath = sandbox.realUserPath();
    const server = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, windowsBinary: windowsPayload() });
    let update: ProcessResult;
    try {
      update = sandbox.runLauncher(["update"], server.hooks());
    } finally {
      await server.close();
    }
    expectSuccess(update);
    expect(update.stderr).not.toContain("warning");

    const receipt = JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"));
    const payload = join(sandbox.appDir, "bin", "envswitch_0.0.42_x86_64-pc-windows-msvc.exe");
    expect(receipt).toMatchObject({ kind: "native", repo: DEFAULT_REPO, tag: "v0.0.42", target: "x86_64-pc-windows-msvc" });
    expect(receipt.appRoot.toLowerCase()).toBe(sandbox.appRoot.toLowerCase());
    expect(receipt.installDir.toLowerCase()).toBe(sandbox.binDir.toLowerCase());
    expect(receipt.launcherPath.toLowerCase()).toBe(sandbox.launcher.toLowerCase());
    expect(receipt.payloadPath.toLowerCase()).toBe(payload.toLowerCase());

    const out = lf(update.stdout);
    expect(out).toContain(`info: binary: ${receipt.payloadPath}\n`);
    expect(out).toContain(`info: launcher: ${receipt.launcherPath}\n`);
    expect(out.endsWith("updated envswitch v0.0.41 -> v0.0.42\n")).toBe(true);

    const launcher = readFileSync(sandbox.launcher);
    expect(isManagedLauncher(launcher, sandbox.appRoot, true)).toBe(true);
    expect(launcher.toString("utf8")).toContain(`"${receipt.payloadPath}" %*`);
    expect(readFileSync(join(sandbox.appDir, ".version"), "utf8")).toBe("v0.0.42\n");
    expect(sandbox.runLauncher(["--version"]).stdout.trim()).toBe("envswitch 0.0.42 x86_64-pc-windows-msvc");
    expect(readFileSync(sandbox.userPathFile, "utf8").toLowerCase()).toContain(sandbox.binDir.toLowerCase());
    expect(sandbox.leftovers()).toEqual([]);
    expect(sandbox.realUserPath()).toBe(realUserPath);
  }, 300_000);

  test("bridge failures leave the bridge install usable", async () => {
    const sandbox = new WindowsBridgeSandbox();
    sandbox.installCopy("bridge", "v0.0.41");
    const realUserPath = sandbox.realUserPath();
    const before = sandbox.snapshot();

    const noLatest = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, statuses: [404], windowsBinary: windowsPayload() });
    const unresolved = sandbox.bridge(["update"], noLatest.hooks());
    expect((await noLatest.close()).map(([path]) => path)).toEqual([`/repos/${DEFAULT_REPO}/releases/latest`]);
    expect(unresolved.code).toBe(1);
    expect(lf(unresolved.stderr)).toBe(`envswitch: no GitHub Release found for ${DEFAULT_REPO}, or access is denied\n`);

    const corrupt = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, fault: "mismatch", windowsBinary: windowsPayload() });
    const rejected = sandbox.bridge(["update"], corrupt.hooks());
    await corrupt.close();
    expect(rejected.code).toBe(1);
    expect(lf(rejected.stderr)).toContain("error: SHA-256 mismatch for envswitch_0.0.42_x86_64-pc-windows-msvc.zip\n");
    expect(lf(rejected.stderr).endsWith("envswitch: update failed: installer exited with code 1\n")).toBe(true);
    expect(rejected.stdout).not.toContain("success:");

    expect(sandbox.snapshot()).toEqual(before);
    expect(sandbox.leftovers()).toEqual([]);
    expect(sandbox.realUserPath()).toBe(realUserPath);
    expect(sandbox.runLauncher(["--help"]).stdout).toContain("envswitch update");
  }, 300_000);

  test("README reinstall (irm | iex form) recovers a skipped-bridge install from the test host shell", async () => {
    const sandbox = new WindowsBridgeSandbox();
    sandbox.installCopy("pre-bridge", "v0.0.40");
    const realUserPath = sandbox.realUserPath();
    // bridge.yml runs this from pwsh and from Windows PowerShell 5.1.
    const host = processEnv("ENVSWITCH_TEST_POWERSHELL") || WINDOWS_POWERSHELL;
    const script = join(REPO_ROOT, "scripts", "install.ps1").replace(/'/g, "''");
    const server = await MockReleaseServer.start(sandbox, { repo: DEFAULT_REPO, windowsBinary: windowsPayload() });
    let reinstall: ProcessResult;
    try {
      reinstall = sandbox.run(
        [host, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", `Get-Content -LiteralPath '${script}' -Raw | Invoke-Expression`],
        server.hooks(),
      );
    } finally {
      await server.close();
    }
    expectSuccess(reinstall);
    expect(lf(reinstall.stdout)).toContain("success: installed envswitch v0.0.42\n");
    const receipt = JSON.parse(readFileSync(join(sandbox.appDir, "install.json"), "utf8"));
    expect(receipt).toMatchObject({ kind: "native", repo: DEFAULT_REPO, tag: "v0.0.42" });
    expect(receipt.installDir.toLowerCase()).toBe(sandbox.binDir.toLowerCase());
    expect(isManagedLauncher(readFileSync(sandbox.launcher), sandbox.appRoot, true)).toBe(true);
    expect(sandbox.runLauncher(["--version"]).stdout.trim()).toBe("envswitch 0.0.42 x86_64-pc-windows-msvc");
    expect(readFileSync(sandbox.userPathFile, "utf8").toLowerCase()).toContain(sandbox.binDir.toLowerCase());
    expect(sandbox.realUserPath()).toBe(realUserPath);
    expect(readFileSync(join(sandbox.project, ".env"), "utf8")).toBe("SECRET=keep\r\n");
  }, 300_000);
});

async function runCli(
  argv: string[],
  options: Parameters<typeof main>[1] = {},
): Promise<{ code: number; stderr: string[] }> {
  const originalError = console.error;
  const originalLog = console.log;
  const stderr: string[] = [];
  try {
    console.error = (...args: unknown[]) => {
      stderr.push(args.map(String).join(" "));
    };
    console.log = () => {};
    return {
      code: await main(argv, options),
      stderr,
    };
  } finally {
    console.error = originalError;
    console.log = originalLog;
  }
}

class FakePrompt {
  readonly selects: Array<{
    message: string;
    choices: Array<{ label: string; value: unknown }>;
    defaultValue: unknown;
  }> = [];
  readonly inputs: Array<{
    message: string;
    defaultValue: string | undefined;
    required: boolean | undefined;
  }> = [];

  constructor(
    private readonly selectAnswers: unknown[],
    private readonly inputAnswers: string[] = [],
  ) {}

  async select<T>(options: {
    message: string;
    choices: Array<{ label: string; value: T }>;
    defaultValue?: T | undefined;
  }): Promise<T> {
    this.selects.push({
      message: options.message,
      choices: options.choices,
      defaultValue: options.defaultValue,
    });
    if (this.selectAnswers.length === 0) {
      if (options.defaultValue !== undefined) {
        return options.defaultValue;
      }
      throw new Error(`unexpected select prompt: ${options.message}`);
    }
    return this.selectAnswers.shift() as T;
  }

  async input(options: {
    message: string;
    defaultValue?: string | undefined;
    required?: boolean;
  }): Promise<string> {
    this.inputs.push({
      message: options.message,
      defaultValue: options.defaultValue,
      required: options.required,
    });
    const answer = this.inputAnswers.shift();
    if (answer !== undefined) {
      return answer;
    }
    if (options.defaultValue !== undefined) {
      return options.defaultValue;
    }
    throw new Error(`unexpected input prompt: ${options.message}`);
  }
}

class CancelPrompt {
  async select(): Promise<never> {
    const error = new Error("User force closed the prompt with 0 null");
    error.name = "ExitPromptError";
    throw error;
  }

  async input(): Promise<never> {
    return this.select();
  }
}

class DeferredVaultScan {
  readonly started: string[] = [];
  private readonly waiters: Array<() => void> = [];
  private readonly resolvers = new Map<string, (items: Array<{ id: string; title: string; tags: string[] }>) => void>();

  start(vault: string): Promise<Array<{ id: string; title: string; tags: string[] }>> {
    this.started.push(vault);
    this.notify();
    return new Promise((resolve) => {
      this.resolvers.set(vault, resolve);
    });
  }

  resolve(vault: string, items: Array<{ id: string; title: string; tags: string[] }>): void {
    const resolver = this.resolvers.get(vault);
    if (!resolver) {
      throw new Error(`vault scan did not start: ${vault}`);
    }
    resolver(items);
  }

  async waitForStarted(count: number): Promise<void> {
    while (this.started.length < count) {
      await new Promise<void>((resolve) => {
        this.waiters.push(resolve);
      });
    }
  }

  private notify(): void {
    for (const waiter of this.waiters.splice(0)) {
      waiter();
    }
  }
}

class FakeClientFactory {
  readonly vaults: string[] = [];
  readonly clients: FakeClient[] = [];

  create(vault: string): FakeClient {
    this.vaults.push(vault);
    const configItems = vault === "vault-a"
      ? [{ id: "cfg", title: "demo config", tags: ["envswitch-config"] }]
      : [];
    const client = new FakeClient(vault, undefined, configItems);
    this.clients.push(client);
    return client;
  }

  allTagCalls(): string[][] {
    return this.clients.flatMap((client) => client.listedTagCalls);
  }

  clientFor(vault: string): FakeClient | undefined {
    return this.clients.find((client) => client.vaultName === vault);
  }
}

class FakeClient {
  listedTags: string[] = [];
  listedTagCalls: string[][] = [];
  readConfigItemIds: string[] = [];

  constructor(
    private readonly vault: string = "vault-a",
    private readonly config: Record<string, unknown> = {
      project: { id: "demo" },
      clients: ["alpha", "beta"],
      environments: ["local", "dev"],
      targets: [
        { name: "api", output: "services/api/.env", groups: ["apis"] },
        { name: "web", output: "apps/web/.env", groups: ["apps"] },
      ],
      tags: { base: ["envswitch"] },
    },
    private readonly configItems: Array<{ id: string; title: string; tags: string[] }> = [
      { id: "cfg", title: "demo config", tags: ["envswitch-config"] },
    ],
    private readonly options: {
      failConfigList?: boolean;
      deferredScan?: DeferredVaultScan;
    } = {},
  ) {}

  get vaultName(): string {
    return this.vault;
  }

  async assertAuthenticated(): Promise<void> {}

  async readConfigItem(item = ""): Promise<string> {
    this.readConfigItemIds.push(item);
    return JSON.stringify(this.config);
  }

  async listItemsByTags(tags: string[]): Promise<Array<{ id: string; title: string; tags: string[] }>> {
    this.listedTags = tags;
    this.listedTagCalls.push(tags);
    if (tags.length === 1 && tags[0] === "envswitch-config") {
      if (this.options.failConfigList) {
        throw new Error("cannot list items");
      }
      if (this.options.deferredScan) {
        return this.options.deferredScan.start(this.vault);
      }
      return this.configItems;
    }

    return [
      {
        id: `${this.vault}-beta-client`,
        title: "beta-client",
        tags: ["envswitch", "project:demo", "env:dev", "layer:client", "client:beta"],
      },
    ];
  }

  async readEnvItem(): Promise<Record<string, string>> {
    return { CLIENT_ONLY: "1" };
  }
}
