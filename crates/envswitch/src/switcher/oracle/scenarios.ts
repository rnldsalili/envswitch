// Prints oracle results (src/switcher.ts at 53de029, Bun 1.4.0) for the C3
// classification errata and switch scenarios asserted in
// tests/switcher_classify.rs and tests/switcher_switch.rs. Owner: C3.
//
// Run from a checkout whose src/ matches 53de029:
//
//   LANG=en_US.UTF-8 bun crates/envswitch/src/switcher/oracle/scenarios.ts .
import { mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const repo = resolve(process.argv[2]!);
const { parseConfig } = await import(join(repo, "src/config.ts"));
const { classifyItemForTarget, switchEnvironment, inspectProject } = await import(join(repo, "src/switcher.ts"));

const show = (s: string) => JSON.stringify(s);

// --- 1. Line terminators in scope tags (02 §3.5 erratum) -------------------
const base = parseConfig(JSON.stringify({
  project: { id: "demo" },
  targets: [{ name: "api", output: ".env", groups: ["apis"] }],
  tags: { base: ["envswitch"] },
}));
const common = ["envswitch", "project:demo", "env:local"];
const params = { env: "local", client: "alpha", target: "api" };
console.log("## line terminators");
for (const scope of ["client", "target", "group"]) {
  for (const lt of ["\r", "\n", " ", " ", "\u0085", "\t", ""]) {
    for (const tag of [`${scope}:foreign${lt}`, `${scope}:${lt}`, `${scope}:a${lt}b`, `${lt}${scope}:foreign`]) {
      const r = classifyItemForTarget(base, { id: "x", title: "x", tags: [...common, "layer:shared", tag] }, params);
      console.log(`${show(tag)} -> ${r}`);
    }
  }
}

// --- 2. Custom templates -----------------------------------------------------
console.log("## custom templates");
const custom = (tags: Record<string, unknown>) =>
  parseConfig(JSON.stringify({ project: { id: "demo" }, targets: [{ name: "api", output: ".env", groups: ["apis"] }], tags: { base: ["envswitch"], ...tags } }));
const cases: Array<[string, Record<string, unknown>, string[][]]> = [
  ["metachars", { client: "c.{client}+(x)", target: "t[{target}]{2}", group: "g^{group}$|*?\\" }, [
    ["layer:shared"], ["layer:shared", "c.beta+(x)"], ["layer:shared", "cZbeta+(x)"], ["layer:shared", "c.beta+(x)x"],
    ["layer:shared", "c.+(x)"], ["layer:shared", "t[worker]{2}"], ["layer:shared", "t[worker]]"], ["layer:shared", "tworker"],
    ["layer:shared", "g^workers$|*?\\"], ["layer:shared", "gworkers"],
    ["layer:client", "c.alpha+(x)"], ["layer:target", "t[api]{2}"], ["layer:group", "g^apis$|*?\\"],
  ]],
  ["multi-sentinel", { client: "{client}/{client}" }, [
    ["layer:shared", "a/b"], ["layer:shared", "a/"], ["layer:shared", "/b"], ["layer:shared", "a/b/c"], ["layer:client", "alpha/alpha"],
  ]],
  ["no-placeholder", { client: "client" }, [
    ["layer:shared", "client"], ["layer:shared", "client:alpha"], ["layer:client", "client"],
  ]],
  ["bare", { client: "{client}" }, [
    ["layer:shared"], ["layer:target", "target:api"], ["layer:client", "alpha"], ["layer:client", "alpha", "target:api"],
  ]],
  ["env-only-client", { client: "{env}" }, [
    ["layer:shared"], ["layer:shared", ""], ["layer:client", ""], ["layer:client", "local"],
  ]],
  ["empty-layer", { layer: "{env}" }, [
    ["layer:shared"], [""], ["local"],
  ]],
  ["project-in-scope", { client: "{project}/{client}" }, [
    ["layer:shared", "demo/beta"], ["layer:shared", "other/beta"], ["layer:client", "demo/alpha"],
  ]],
];
for (const [name, tags, itemTagSets] of cases) {
  const cfg = custom(tags);
  for (const extra of itemTagSets) {
    const r = classifyItemForTarget(cfg, { id: "x", title: "x", tags: [...common, ...extra] }, params);
    console.log(`${name} ${JSON.stringify(extra)} -> ${r}`);
  }
}

// --- 3. {project} fallback uses process.cwd() -------------------------------
console.log("## project fallback");
{
  const dir = await mkdtemp(join(tmpdir(), "envswitch-cwd-"));
  const cwd = join(dir, "demo-suite");
  await mkdir(cwd);
  const prev = process.cwd();
  process.chdir(cwd);
  try {
    const cfg = parseConfig(JSON.stringify({ targets: [{ name: "api", output: ".env" }], tags: { base: ["app:{project}"] } }));
    for (const tags of [["app:demo-suite", "env:local", "layer:shared"], ["app:other", "env:local", "layer:shared"], ["env:local", "layer:shared"]]) {
      console.log(`${JSON.stringify(tags)} -> ${classifyItemForTarget(cfg, { id: "x", title: "x", tags }, params)}`);
    }
  } finally {
    process.chdir(prev);
    await rm(dir, { recursive: true, force: true });
  }
}

// --- 4. Event sequences ------------------------------------------------------
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
const T = (id: string, ...tags: string[]) => ({ id, title: id, tags: ["envswitch", "project:demo", "env:local", ...tags] });
const items = [
  T("shared", "layer:shared"),
  T("api-group", "layer:group", "group:apis"),
  T("app-group", "layer:group", "group:apps"),
  T("shared-with-group", "layer:shared", "group:apis"),
  T("api-target", "layer:target", "target:api"),
  T("web-target", "layer:target", "target:web"),
  T("target-with-group", "layer:target", "target:api", "group:apis"),
  T("target-with-client", "layer:target", "target:api", "client:alpha"),
  T("alpha", "layer:client", "client:alpha"),
  T("alpha-web", "layer:client-target", "client:alpha", "group:apps"),
  T("alpha-api", "layer:client-target", "client:alpha", "group:apis"),
  T("beta-web", "layer:client-target", "client:beta", "group:apps"),
  T("alpha-target-web", "layer:client-target", "client:alpha", "target:web"),
  T("shared-unknown-client", "layer:shared", "client:gamma"),
  T("shared-unknown-target", "layer:shared", "target:worker"),
  T("shared-unknown-group", "layer:shared", "group:workers"),
];
const values: Record<string, Record<string, string>> = {
  shared: { SHARED: "1", COMMON: "shared" },
  "api-group": { API_GROUP: "1", COMMON: "api-group" },
  "app-group": { APP_GROUP: "1", COMMON: "app-group" },
  "api-target": { API_ONLY: "1", COMMON: "api" },
  "web-target": { WEB_ONLY: "1", COMMON: "web" },
  alpha: { CLIENT_ONLY: "1", COMMON: "alpha" },
  "alpha-web": { WEB_CLIENT_ONLY: "1", COMMON: "alpha-web" },
  "alpha-api": { API_CLIENT_ONLY: "1", COMMON: "alpha-api" },
};
for (const dryRun of [true, false]) {
  console.log(`## events all dryRun=${dryRun}`);
  const root = await mkdtemp(join(tmpdir(), "envswitch-events-"));
  try {
    const logs: string[] = [];
    const op = {
      listItemsByTags: async () => items,
      readEnvItem: async (item: { id: string }) => values[item.id] ?? { SHOULD_NOT_APPLY: "1" },
    };
    const results = await switchEnvironment(config, op, {
      root, env: "local", client: "alpha", targetName: "all", dryRun,
      log: (e: { status: string; message: string }) => logs.push(`${e.status}:${e.message.replaceAll(root, "{ROOT}")}`),
    });
    for (const l of logs) console.log(l);
    for (const r of results) {
      console.log(`result ${r.target.name} written=${r.written} keyCount=${r.keyCount} layers=${r.layers.map((l: any) => `${l.name}:${l.item.id}`).join(",")} overwritten=${r.overwrittenKeys.join(",")}`);
    }
    if (!dryRun) {
      console.log("api.env=" + show(await readFile(join(root, "apps/api/.env"), "utf8")));
      console.log("web.env=" + show(await readFile(join(root, "apps/web/.env"), "utf8")));
      console.log("api dir=" + JSON.stringify(await readdir(join(root, "apps/api"))));
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

// --- 5. Read failure on the second target ------------------------------------
console.log("## read failure");
{
  const root = await mkdtemp(join(tmpdir(), "envswitch-fail-"));
  try {
    const logs: string[] = [];
    const op = {
      listItemsByTags: async () => items,
      readEnvItem: async (item: { id: string }) => {
        if (item.id === "app-group") throw new Error("op read failed");
        return values[item.id] ?? {};
      },
    };
    try {
      await switchEnvironment(config, op, {
        root, env: "local", client: "alpha", targetName: "all", dryRun: false,
        log: (e: { status: string; message: string }) => logs.push(`${e.status}:${e.message.replaceAll(root, "{ROOT}")}`),
      });
    } catch (error) {
      console.log(`error: ${(error as Error).message}`);
    }
    for (const l of logs) console.log(l);
    console.log("api.env exists=" + (await Bun.file(join(root, "apps/api/.env")).exists()));
    console.log("web.env exists=" + (await Bun.file(join(root, "apps/web/.env")).exists()));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

// --- 6. Validation happens before any op call -----------------------------------
console.log("## validation before op");
for (const [label, cfgJson, opts] of [
  ["escape", { targets: [{ name: "api", output: "../x/.env" }] }, { env: "local", client: "alpha", targetName: "api" }],
  ["bad env", { targets: [{ name: "api", output: ".env" }] }, { env: "bad env", client: "alpha", targetName: "api" }],
  ["unknown target", { targets: [{ name: "api", output: ".env" }] }, { env: "local", client: "alpha", targetName: "worker" }],
  ["marker", { project: { rootMarkers: [{ path: "services" }] }, targets: [{ name: "api", output: ".env" }] }, { env: "local", client: "alpha", targetName: "api" }],
] as const) {
  const root = await mkdtemp(join(tmpdir(), "envswitch-validate-"));
  try {
    const logs: string[] = [];
    let calls = 0;
    const op = { listItemsByTags: async () => { calls++; return []; }, readEnvItem: async () => { calls++; return {}; } };
    try {
      await switchEnvironment(parseConfig(JSON.stringify(cfgJson)), op, { root, dryRun: true, ...opts, log: (e: any) => logs.push(`${e.status}:${e.message.replaceAll(root, "{ROOT}")}`) });
      console.log(`${label}: ok`);
    } catch (error) {
      console.log(`${label}: error: ${(error as Error).message} calls=${calls} logs=${JSON.stringify(logs)}`);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

// --- 7. Empty candidate list writes "\n" ----------------------------------------
console.log("## empty");
{
  const root = await mkdtemp(join(tmpdir(), "envswitch-empty-"));
  try {
    const logs: string[] = [];
    const op = { listItemsByTags: async () => [], readEnvItem: async () => ({}) };
    const results = await switchEnvironment(config, op, { root, env: "local", client: "alpha", targetName: "api", dryRun: false, log: (e: any) => logs.push(`${e.status}:${e.message.replaceAll(root, "{ROOT}")}`) });
    for (const l of logs) console.log(l);
    console.log(`keyCount=${results[0].keyCount} env=${show(await readFile(join(root, "apps/api/.env"), "utf8"))}`);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

// --- 8. inspectProject -----------------------------------------------------------
console.log("## inspect");
{
  const root = await mkdtemp(join(tmpdir(), "envswitch-inspect-"));
  try {
    await mkdir(join(root, "apps/api"), { recursive: true });
    const report = await inspectProject(config, root);
    for (const r of report) console.log(`${r.target.name} ${r.outputPath.replaceAll(root, "{ROOT}")} parentExists=${r.parentExists}`);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}
