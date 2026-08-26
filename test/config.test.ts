import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { parseConfig, resolveOutputPath, selectTargets, validateProjectRoot } from "../src/config.ts";

describe("config", () => {
  test("parses a valid generic config", () => {
    const config = parseConfig(JSON.stringify({
      project: { id: "demo" },
      clients: ["alpha"],
      environments: ["local"],
      targets: [{ name: "api", output: "apps/api/.env", groups: ["apis"] }],
      tags: { base: ["envswitch"], group: "group:{group}" },
    }));

    expect(config.project?.id).toBe("demo");
    expect(config.targets[0]?.name).toBe("api");
    expect(config.targets[0]?.groups).toEqual(["apis"]);
    expect(config.tags?.group).toBe("group:{group}");
  });

  test("rejects invalid target groups", () => {
    expect(() => parseConfig(JSON.stringify({
      targets: [{ name: "api", output: "apps/api/.env", groups: ["all apis"] }],
    }))).toThrow("unsupported characters");
  });

  test("rejects malformed config fields with specific messages", () => {
    expect(() => parseConfig("{")).toThrow("config field is not valid JSON");
    expect(() => parseConfig(JSON.stringify([]))).toThrow("config JSON must be an object");
    expect(() => parseConfig(JSON.stringify({
      clients: "alpha",
      targets: [{ name: "api", output: "apps/api/.env" }],
    }))).toThrow("config.clients must be an array");
    expect(() => parseConfig(JSON.stringify({
      project: { id: "bad project" },
      targets: [{ name: "api", output: "apps/api/.env" }],
    }))).toThrow("project id contains unsupported characters");
  });

  test("rejects duplicate targets", () => {
    expect(() => parseConfig(JSON.stringify({
      targets: [
        { name: "api", output: "apps/api/.env" },
        { name: "api", output: "apps/api-copy/.env" },
      ],
    }))).toThrow("duplicate target");
  });

  test("resolves target output inside project root", () => {
    expect(resolveOutputPath("/tmp/project", "apps/api/.env")).toBe("/tmp/project/apps/api/.env");
  });

  test("rejects output paths that escape the project root", () => {
    expect(() => resolveOutputPath("/tmp/project", "../outside/.env")).toThrow("escapes");
    expect(() => resolveOutputPath("/tmp/project", "/tmp/outside/.env")).toThrow("relative");
  });

  test("rejects output paths with symlinked parents", async () => {
    const base = await mkdtemp(join(tmpdir(), "envswitch-symlink-"));
    const root = join(base, "project");
    const outside = join(base, "outside");
    try {
      await mkdir(root);
      await mkdir(outside);
      await symlink(outside, join(root, "linked"), process.platform === "win32" ? "junction" : "dir");

      expect(() => resolveOutputPath(root, "linked/.env")).toThrow("symbolic-link parent");
      expect(await Bun.file(join(outside, ".env")).exists()).toBe(false);
    } finally {
      await rm(base, { recursive: true, force: true });
    }
  });

  test("selects all targets or one named target", () => {
    const config = parseConfig(JSON.stringify({
      targets: [
        { name: "api", output: "apps/api/.env" },
        { name: "web", output: "apps/web/.env" },
      ],
    }));

    expect(selectTargets(config, "all")).toHaveLength(2);
    expect(selectTargets(config, "web").map((target) => target.name)).toEqual(["web"]);
    expect(() => selectTargets(config, "worker")).toThrow("not defined");
  });

  test("validates directory and package root markers", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-root-"));
    try {
      await mkdir(join(root, "services"));
      await writeFile(join(root, "package.json"), JSON.stringify({ name: "demo-suite" }));
      const config = parseConfig(JSON.stringify({
        project: {
          id: "demo-suite",
          rootMarkers: [
            { path: "services" },
            { packageJsonName: "demo-suite" },
          ],
        },
        targets: [{ name: "api", output: "apps/api/.env" }],
      }));

      await expect(validateProjectRoot(config, root)).resolves.toBeUndefined();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
