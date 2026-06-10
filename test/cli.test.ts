import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { formatProgressEvent, main, parseArgs } from "../src/cli.ts";

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

describe("cli update command", () => {
  const installerName = process.platform === "win32" ? "install.ps1" : "install.sh";

  async function makeInstalledAppDir(options: {
    version?: string;
    installer?: boolean;
    gitCheckout?: boolean;
  } = {}): Promise<string> {
    const appRoot = await mkdtemp(join(tmpdir(), "envswitch-update-"));
    const appDir = join(appRoot, "current");
    await mkdir(appDir, { recursive: true });
    if (options.installer !== false) {
      await mkdir(join(appDir, "scripts"), { recursive: true });
      await writeFile(join(appDir, "scripts", installerName), "#!/usr/bin/env bash\n");
    }
    if (options.version !== undefined) {
      await writeFile(join(appDir, ".version"), `${options.version}\n`);
    }
    if (options.gitCheckout) {
      await mkdir(join(appDir, ".git"), { recursive: true });
    }
    return appDir;
  }

  test("short-circuits when already on the latest version", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.5" });
    const output: string[] = [];
    const installerCalls: Array<[string, string]> = [];

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async (scriptPath, appRoot) => {
        installerCalls.push([scriptPath, appRoot]);
        return 0;
      },
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(installerCalls).toEqual([]);
    expect(output).toEqual(["envswitch is already up to date (v0.0.5)"]);
  });

  test("runs the installer and reports old -> new version", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.4" });
    const output: string[] = [];
    const installerCalls: Array<[string, string]> = [];

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async (scriptPath, appRoot) => {
        installerCalls.push([scriptPath, appRoot]);
        await writeFile(join(appDir, ".version"), "v0.0.5\n");
        return 0;
      },
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(installerCalls).toEqual([[join(appDir, "scripts", installerName), dirname(appDir)]]);
    expect(output).toEqual(["updated envswitch v0.0.4 -> v0.0.5"]);
  });

  test("rejects updates from a source checkout", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.4", gitCheckout: true });
    let installerCalled = false;

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async () => {
        installerCalled = true;
        return 0;
      },
      log: () => {},
    });

    expect(result.code).toBe(1);
    expect(installerCalled).toBe(false);
    expect(result.stderr).toEqual([
      "envswitch: envswitch update only works for installed copies; this looks like a source checkout (use git pull instead)",
    ]);
  });

  test("fails when the bundled installer is missing", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.4", installer: false });

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async () => 0,
      log: () => {},
    });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual([
      `envswitch: installer not found at ${join(appDir, "scripts", installerName)}; reinstall envswitch with the install script from the README`,
    ]);
  });

  test("fails when the installer exits non-zero", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.4" });

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async () => 1,
      log: () => {},
    });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: update failed: installer exited with code 1"]);
  });

  test("updates when the installed version is unknown", async () => {
    const appDir = await makeInstalledAppDir();
    const output: string[] = [];

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async () => {
        await writeFile(join(appDir, ".version"), "v0.0.5\n");
        return 0;
      },
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(output).toEqual(["updated envswitch (unknown) -> v0.0.5"]);
  });

  test("updates even when the latest version cannot be resolved", async () => {
    const appDir = await makeInstalledAppDir({ version: "v0.0.4" });
    const output: string[] = [];
    let installerCalled = false;

    const result = await runCli(["update"], {
      appDir,
      fetchLatestVersion: async () => undefined,
      runInstaller: async () => {
        installerCalled = true;
        return 0;
      },
      log: () => {},
      writeLine: (line) => output.push(line),
    });

    expect(result.code).toBe(0);
    expect(installerCalled).toBe(true);
    expect(output).toEqual(["updated envswitch v0.0.4 -> v0.0.4"]);
  });

  test("rejects unknown update flags", async () => {
    const result = await runCli(["update", "--vault", "x"], {
      appDir: await makeInstalledAppDir({ version: "v0.0.4" }),
      fetchLatestVersion: async () => "v0.0.5",
      runInstaller: async () => 0,
      log: () => {},
    });

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --vault"]);
  });
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
