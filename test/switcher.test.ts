import { mkdir, readFile, mkdtemp, rm, symlink } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { describe, expect, test } from "bun:test";
import { parseConfig } from "../src/config.ts";
import { classifyItemForTarget, switchEnvironment } from "../src/switcher.ts";
import type { OnePasswordItemSummary, ProgressEvent } from "../src/types.ts";

const config = parseConfig(JSON.stringify({
  project: { id: "demo" },
  clients: ["alpha", "beta"],
  environments: ["local"],
  targets: [
    { name: "api", output: "apps/api/.env", groups: ["apis"] },
    { name: "web", output: "apps/web/.env", groups: ["apps"] },
  ],
  tags: { base: ["envswitch"] },
}));

const items: OnePasswordItemSummary[] = [
  {
    id: "shared",
    title: "shared",
    tags: ["envswitch", "project:demo", "env:local", "layer:shared"],
  },
  {
    id: "api-group",
    title: "api-group",
    tags: ["envswitch", "project:demo", "env:local", "layer:group", "group:apis"],
  },
  {
    id: "app-group",
    title: "app-group",
    tags: ["envswitch", "project:demo", "env:local", "layer:group", "group:apps"],
  },
  {
    id: "shared-with-group",
    title: "shared-with-group",
    tags: ["envswitch", "project:demo", "env:local", "layer:shared", "group:apis"],
  },
  {
    id: "api-target",
    title: "api-target",
    tags: ["envswitch", "project:demo", "env:local", "layer:target", "target:api"],
  },
  {
    id: "web-target",
    title: "web-target",
    tags: ["envswitch", "project:demo", "env:local", "layer:target", "target:web"],
  },
  {
    id: "target-with-group",
    title: "target-with-group",
    tags: ["envswitch", "project:demo", "env:local", "layer:target", "target:api", "group:apis"],
  },
  {
    id: "target-with-client",
    title: "target-with-client",
    tags: ["envswitch", "project:demo", "env:local", "layer:target", "target:api", "client:alpha"],
  },
  {
    id: "alpha",
    title: "alpha",
    tags: ["envswitch", "project:demo", "env:local", "layer:client", "client:alpha"],
  },
  {
    id: "alpha-web",
    title: "alpha-web",
    tags: ["envswitch", "project:demo", "env:local", "layer:client-target", "client:alpha", "group:apps"],
  },
  {
    id: "alpha-api",
    title: "alpha-api",
    tags: ["envswitch", "project:demo", "env:local", "layer:client-target", "client:alpha", "group:apis"],
  },
  {
    id: "beta-web",
    title: "beta-web",
    tags: ["envswitch", "project:demo", "env:local", "layer:client-target", "client:beta", "group:apps"],
  },
  {
    id: "alpha-target-web",
    title: "alpha-target-web",
    tags: ["envswitch", "project:demo", "env:local", "layer:client-target", "client:alpha", "target:web"],
  },
  {
    id: "shared-unknown-client",
    title: "shared-unknown-client",
    tags: ["envswitch", "project:demo", "env:local", "layer:shared", "client:gamma"],
  },
  {
    id: "shared-unknown-target",
    title: "shared-unknown-target",
    tags: ["envswitch", "project:demo", "env:local", "layer:shared", "target:worker"],
  },
  {
    id: "shared-unknown-group",
    title: "shared-unknown-group",
    tags: ["envswitch", "project:demo", "env:local", "layer:shared", "group:workers"],
  },
];

const values: Record<string, Record<string, string>> = {
  shared: { SHARED: "1", COMMON: "shared" },
  "api-group": { API_GROUP: "1", COMMON: "api-group" },
  "app-group": { APP_GROUP: "1", COMMON: "app-group" },
  "shared-with-group": { SHOULD_NOT_APPLY: "1" },
  "api-target": { API_ONLY: "1", COMMON: "api" },
  "web-target": { WEB_ONLY: "1", COMMON: "web" },
  "target-with-group": { SHOULD_NOT_APPLY: "1" },
  "target-with-client": { SHOULD_NOT_APPLY: "1" },
  alpha: { CLIENT_ONLY: "1", COMMON: "alpha" },
  "alpha-web": { WEB_CLIENT_ONLY: "1", COMMON: "alpha-web" },
  "alpha-api": { API_CLIENT_ONLY: "1", COMMON: "alpha-api" },
  "beta-web": { SHOULD_NOT_APPLY: "1" },
  "alpha-target-web": { SHOULD_NOT_APPLY: "1" },
  "shared-unknown-client": { SHOULD_NOT_APPLY: "1" },
  "shared-unknown-target": { SHOULD_NOT_APPLY: "1" },
  "shared-unknown-group": { SHOULD_NOT_APPLY: "1" },
};

describe("switcher", () => {
  test("classifies only items that belong to the selected target", () => {
    expect(classifyItemForTarget(config, items[0]!, { env: "local", client: "alpha", target: "api" })).toBe("shared");
    expect(classifyItemForTarget(config, items[1]!, { env: "local", client: "alpha", target: "api" })).toBe("group");
    expect(classifyItemForTarget(config, items[2]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[2]!, { env: "local", client: "alpha", target: "web" })).toBe("group");
    expect(classifyItemForTarget(config, items[3]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[4]!, { env: "local", client: "alpha", target: "api" })).toBe("target");
    expect(classifyItemForTarget(config, items[5]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[6]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[7]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[8]!, { env: "local", client: "alpha", target: "api" })).toBe("client");
    expect(classifyItemForTarget(config, items[9]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[9]!, { env: "local", client: "alpha", target: "web" })).toBe("client-target");
    expect(classifyItemForTarget(config, items[10]!, { env: "local", client: "alpha", target: "api" })).toBe("client-target");
    expect(classifyItemForTarget(config, items[11]!, { env: "local", client: "alpha", target: "web" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[12]!, { env: "local", client: "alpha", target: "web" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[13]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[14]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    expect(classifyItemForTarget(config, items[15]!, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
  });

  test("recognizes unknown scopes with custom tag templates", () => {
    const customConfig = parseConfig(JSON.stringify({
      project: { id: "demo" },
      clients: ["alpha"],
      targets: [{ name: "api", output: ".env", groups: ["apis"] }],
      tags: {
        base: ["envswitch"],
        client: "scope/client/{client}",
        target: "scope/target/{target}",
        group: "scope/group/{group}",
      },
    }));
    const commonTags = ["envswitch", "project:demo", "env:local", "layer:shared"];

    for (const tag of ["scope/client/beta", "scope/target/worker", "scope/group/workers"]) {
      expect(classifyItemForTarget(customConfig, {
        id: tag,
        title: tag,
        tags: [...commonTags, tag],
      }, { env: "local", client: "alpha", target: "api" })).toBeUndefined();
    }
  });

  test("dry run merges supported layers without writing files", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-"));
    try {
      const originalError = console.error;
      const stderrLogs: string[] = [];
      const op = {
        listItemsByTags: async () => items,
        readEnvItem: async (item: OnePasswordItemSummary) => values[item.id] ?? {},
      };

      console.error = (...args: unknown[]) => {
        stderrLogs.push(args.map(String).join(" "));
      };
      let results;
      try {
        results = await switchEnvironment(config, op, {
          root,
          env: "local",
          client: "alpha",
          targetName: "api",
          dryRun: true,
        });
      } finally {
        console.error = originalError;
      }

      expect(stderrLogs).toEqual([]);
      expect(results).toHaveLength(1);
      expect(results[0]?.keyCount).toBe(6);
      expect(results[0]?.layers.map((layer) => layer.item.id)).toEqual(["shared", "api-group", "api-target", "alpha", "alpha-api"]);
      expect(results[0]?.overwrittenKeys).toEqual(["COMMON"]);
      expect(await Bun.file(join(root, "apps/api/.env")).exists()).toBe(false);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("dry run logs progress when a logger is provided", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-"));
    try {
      const logs: ProgressEvent[] = [];
      const op = {
        listItemsByTags: async () => items,
        readEnvItem: async (item: OnePasswordItemSummary) => values[item.id] ?? {},
      };

      await switchEnvironment(config, op, {
        root,
        env: "local",
        client: "alpha",
        targetName: "api",
        dryRun: true,
        log: (event) => logs.push(event),
      });

      const outputPath = join(root, "apps/api/.env");
      expect(logs).toEqual([
        { status: "info", message: `validating project root ${root}` },
        { status: "info", message: "listing candidate 1Password items" },
        { status: "done", message: "found 16 candidate item(s)" },
        { status: "info", message: `processing api -> ${outputPath}` },
        { status: "info", message: "reading shared" },
        { status: "info", message: "reading api-group" },
        { status: "info", message: "reading api-target" },
        { status: "info", message: "reading alpha" },
        { status: "info", message: "reading alpha-api" },
        { status: "dry-run", message: `would write ${outputPath}` },
      ]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("writes merged values in group-aware layer order", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-"));
    try {
      const op = {
        listItemsByTags: async () => items,
        readEnvItem: async (item: OnePasswordItemSummary) => values[item.id] ?? {},
      };

      const results = await switchEnvironment(config, op, {
        root,
        env: "local",
        client: "alpha",
        targetName: "web",
        dryRun: false,
      });

      expect(results[0]?.layers.map((layer) => layer.item.id)).toEqual([
        "shared",
        "app-group",
        "web-target",
        "alpha",
        "alpha-web",
      ]);
      expect(await readFile(join(root, "apps/web/.env"), "utf8")).toBe([
        "APP_GROUP=1",
        "CLIENT_ONLY=1",
        "COMMON=alpha-web",
        "SHARED=1",
        "WEB_CLIENT_ONLY=1",
        "WEB_ONLY=1",
        "",
      ].join("\n"));
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("caches 1Password item reads across targets in one switch", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-"));
    try {
      const readCounts: Record<string, number> = {};
      const op = {
        listItemsByTags: async () => items,
        readEnvItem: async (item: OnePasswordItemSummary) => {
          readCounts[item.id] = (readCounts[item.id] ?? 0) + 1;
          return values[item.id] ?? {};
        },
      };

      const results = await switchEnvironment(config, op, {
        root,
        env: "local",
        client: "alpha",
        targetName: "all",
        dryRun: true,
      });

      expect(results.map((result) => result.target.name)).toEqual(["api", "web"]);
      expect(readCounts.shared).toBe(1);
      expect(readCounts.alpha).toBe(1);
      expect(readCounts["api-group"]).toBe(1);
      expect(readCounts["app-group"]).toBe(1);
      expect(readCounts["api-target"]).toBe(1);
      expect(readCounts["web-target"]).toBe(1);
      expect(readCounts["alpha-api"]).toBe(1);
      expect(readCounts["alpha-web"]).toBe(1);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("rechecks output parents immediately before writing", async () => {
    const base = await mkdtemp(join(tmpdir(), "envswitch-symlink-write-"));
    const root = join(base, "project");
    const outside = join(base, "outside");
    const outputDir = join(root, "apps/api");
    try {
      await mkdir(outputDir, { recursive: true });
      await mkdir(outside);
      const op = {
        listItemsByTags: async () => [items[0]!],
        readEnvItem: async () => {
          await rm(outputDir, { recursive: true });
          await symlink(outside, outputDir, process.platform === "win32" ? "junction" : "dir");
          return { TOKEN: "secret" };
        },
      };

      await expect(switchEnvironment(config, op, {
        root,
        env: "local",
        client: "alpha",
        targetName: "api",
        dryRun: false,
      })).rejects.toThrow("symbolic-link parent");
      expect(await Bun.file(join(outside, ".env")).exists()).toBe(false);
    } finally {
      await rm(base, { recursive: true, force: true });
    }
  });

});
