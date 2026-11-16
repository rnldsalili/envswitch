// Maintainer tool: expands the contract tables to input-only, individually reviewable JSON.
import { mkdir, writeFile, readdir, rm } from "node:fs/promises";
import { join } from "node:path";
const dir = join(import.meta.dir, "scenarios"); await mkdir(dir, { recursive: true });
const scenarios: any[] = [];
const add = (kind: string, name: string, input: any = {}) => scenarios.push({ id: `${kind}-${name}`, kind, ...input });
const target = { name: "api", output: "apps/api/.env", groups: ["apis"] };
const minimal = { targets: [target] };
const config = { project: { id: "demo-suite" }, clients: ["alpha", "beta"], environments: ["local", "dev"], targets: [target, { name: "web", output: "apps/web/.env", groups: ["apps"] }], tags: { base: ["envswitch"] } };
const common = ["envswitch", "project:demo-suite", "env:local"];
const item = (id: string, title: string, tags: string[]) => ({ id, title, tags: [...common, ...tags] });
const switchArgs = ["switch", "--vault", "alpha", "--config-item", "cfg", "--client", "alpha"];
const listArgs = ["list", "--vault", "alpha", "--config-item", "cfg"];
const doctorArgs = ["doctor", "--vault", "alpha", "--config-item", "cfg"];
const get = (id = "cfg", vault = "alpha") => ["item", "get", id, "--vault", vault, "--format", "json", "--reveal"];
const list = (tags?: string, vault = "alpha") => ["item", "list", "--vault", vault, "--format", "json", ...(tags === undefined ? [] : ["--tags", tags])];
const raw = (argv: string[], stdout: string, exitCode = 0, stderr = "") => ({ argv, stdout, stderr, exitCode });

const parser: Record<string, string[]> = {
  empty: [], defaults: switchArgs, "empty-command": [""], "flag-command": ["--vault", "alpha", "switch"],
  "empty-help-next": ["switch", "--help", ""], "empty-dry-next": ["switch", "--dry-run", ""],
  "equals-empty": ["switch", "--vault="], "equals-many": ["switch", "--a=b=c"], "single-dash-value": ["switch", "--client", "-x"],
  "short-positionals": ["switch", "-h", "-x", "-"], "bare-double-dash": ["switch", "--"], "bare-double-dash-value": ["switch", "--", "alpha"],
  "repeat-value": ["switch", "--vault", "alpha", "--vault=beta"], "repeat-insertion": ["switch", "--z", "--a", "--z=2"],
  "boolean-greedy": ["switch", "--dry-run", "foo"], "boolean-string": ["switch", "--dry-run=false"], "next-flag": ["switch", "--vault", "--client", "alpha"],
  "empty-token": ["switch", "", "--vault", "alpha"], "case-sensitive": ["switch", "--VAULT", "alpha"],
  "proto": ["switch", "--__proto__", "x"], "integer-order": ["switch", "--z", "--1", "a"], "constructor": ["switch", "--constructor", "x"],
  "unicode": ["switch", "--é", "α"], "missing-value": ["switch", "--vault"], "first-help": ["--help", "--ignored"],
};
for (const [name, argv] of Object.entries(parser)) add("parseArgs", name, { argv, ...(name === "proto" ? { divergence: "a3" } : name === "integer-order" ? { divergence: "a4" } : {}) });
for (const status of ["info", "done", "dry-run"]) for (const color of [false, true]) add("formatProgressEvent", `${status}-${color ? "color" : "plain"}`, { event: { status, message: "demo-suite\r\nalpha" }, color });

const cliCases: Record<string, string[]> = {
  "help-long": ["--help", "--ignored"], "help-short": ["-h", "--ignored"], "help-command": ["help", "--unknown"],
  "help-unknown-command": ["foo", "--help"], "help-before-validation": ["switch", "--unknown", "--help"], "help-empty-next": parser["empty-help-next"],
  "dry-empty-next": parser["empty-dry-next"], "help-string-false": ["switch", "--help=false"], "help-string": ["switch", "--help", "me"],
  "help-string-unknown": ["foo", "--help=1"], "short-ignored": ["switch", "-h"], "empty-command": [""], "unknown-command": ["foo"],
  "flag-command": ["--vault", "alpha", "switch"], "version": ["--version"], "version-short": ["-v"], "version-capital": ["-V"],
  "bare-double-dash": ["switch", "--"], "case-sensitive": ["switch", "--VAULT", "alpha"], "unknown-switch": ["switch", "--unknown"],
  "boolean-value": ["switch", "--dry-run=false"], "boolean-greedy": ["switch", "--dry-run", "foo"],
  "unknown-list": ["list", "--client", "alpha"], "unknown-doctor": ["doctor", "--client", "alpha"], "unknown-update": ["update", "--vault", "alpha"],
  "proto": parser.proto, "integer-order": parser["integer-order"],
};
for (const [name, argv] of Object.entries(cliCases)) add("cli", name, { argv, ...(name === "version" ? { divergence: "f1" } : name === "proto" ? { divergence: "a3" } : name === "integer-order" ? { divergence: "a4" } : {}) });
for (const flag of ["vault", "config-item", "client", "env", "target"]) {
  for (const [form, value] of [["missing", null], ["bare", true], ["empty", ""], ["space", "  "], ["bom", "\uFEFF"]] as const) {
    if (form === "missing" && ["env", "target"].includes(flag)) continue;
    const argv = [...switchArgs]; const i = argv.indexOf(`--${flag}`); if (i >= 0) argv.splice(i, 2);
    if (value !== null) argv.push(value === true ? `--${flag}` : `--${flag}=${value}`);
    add("cli", `flag-${flag}-${form}`, { argv, config });
  }
}
add("cli", "required-order", { argv: ["switch", "--env"] });
add("cli", "untrimmed-client", { argv: [...switchArgs.slice(0, -1), " alpha "], config });
for (const subject of ["clients", "targets", "layers", "bogus", ""]) add("cli", `list-${subject || "empty"}`, { argv: [...listArgs, subject], config });
add("cli", "list-default", { argv: listArgs, config });
add("cli", "list-extra-ignored", { argv: [...listArgs, "clients", "bogus"], config });
for (const clients of [undefined, []]) add("cli", `list-none-${clients ? "empty" : "absent"}`, { argv: [...listArgs, "clients"], config: { ...minimal, ...(clients ? { clients } : {}) } });
add("cli", "list-load-before-subject", { argv: [...listArgs, "bogus"], configRaw: "{}" });
add("cli", "doctor", { argv: doctorArgs, config, files: [{ path: "apps/api", type: "dir" }] });
add("cli", "doctor-auth-failure", { argv: doctorArgs, op: { responses: [raw(["whoami", "--format", "json"], "demo-suite-private", 1, "  not signed in\n")] } });
add("cli", "doctor-marker-failure", { argv: doctorArgs, config: { ...minimal, project: { rootMarkers: [{ path: "missing" }] } } });
for (const dry of [false, true]) add("cli", `switch-${dry ? "dry" : "write"}`, { argv: [...switchArgs, ...(dry ? ["--dry-run"] : [])], config, op: { items: [item("alpha", "alpha", ["layer:shared"])], values: { alpha: { A: "1", B: "two words" } } } });
add("cli", "switch-extra-ignored", { argv: [...switchArgs, "ignored"], config });
add("cli", "switch-env-dev", { argv: [...switchArgs, "--env", "dev", "--target", "web", "--dry-run"], config });
for (const [name, tty, env] of [
  ["tty", true, {}], ["non-tty", false, {}], ["ci-empty", true, { CI: "" }], ["ci-one", true, { CI: "1" }],
  ["ci-false", true, { CI: "false" }], ["ci-zero", true, { CI: "0" }], ["no-color-empty", true, { NO_COLOR: "" }],
  ["no-color-one", true, { NO_COLOR: "1" }], ["force-ignored", false, { FORCE_COLOR: "1", CLICOLOR_FORCE: "1" }],
] as const) add("cli", `color-${name}`, { argv: [...switchArgs, "--dry-run"], config, tty: { stderr: tty, stdout: !tty }, env });

// Every reachable config diagnostic, including key presence, JS trim, and check order.
add("config", "minimal", { config: minimal }); add("config", "generic", { config });
for (const [name, value] of [["root-null", null], ["root-array", []], ["root-string", "alpha"], ["root-number", 1], ["targets-absent", {}], ["targets-null", { targets: null }], ["targets-empty", { targets: [] }], ["targets-object", { targets: {} }]]) add("config", name as string, { config: value });
for (const [name, rawConfig] of [["malformed", "{"], ["empty", ""], ["bom-json", '\uFEFF{"targets":[]}'], ["duplicates-json", '{"targets":[],"targets":[{"name":"api","output":".env"}]}']]) add("config", name, { configRaw: rawConfig, ...(name !== "duplicates-json" ? { divergence: "a5" } : {}) });
for (const [name, bad] of [["target-null", null], ["name-absent", { output: ".env" }], ["name-empty", { name: "", output: ".env" }], ["name-number", { name: 1, output: ".env" }], ["name-space", { name: " ", output: ".env" }], ["output-absent", { name: "api" }], ["output-null", { name: "api", output: null }], ["output-space", { name: "api", output: " " }], ["name-identifier", { name: "bad name", output: ".env" }], ["groups-null", { ...target, groups: null }], ["groups-empty-entry", { ...target, groups: [""] }], ["groups-bad-id", { ...target, groups: ["bad group"] }]]) add("config", name as string, { config: { targets: [bad] } });
for (const [key, values] of Object.entries({ project: [null, [], 3], clients: [null, "alpha", [""], [null], ["  "], ["\uFEFF"], ["\u0085"], []], environments: [null, "local", [""], [1], []], tags: [null, [], 3] })) values.forEach((value, i) => add("config", `${key}-${i}`, { config: { ...minimal, [key]: value } }));
for (const [name, project] of Object.entries({ "id-null": { id: null }, "id-number": { id: 1 }, "id-empty": { id: "" }, "id-bad": { id: "bad project" }, "markers-null": { rootMarkers: null }, "marker-null": { rootMarkers: [null] }, "marker-empty": { rootMarkers: [{}] }, "marker-path-null": { rootMarkers: [{ path: null }] }, "marker-path-space": { rootMarkers: [{ path: " " }] }, "marker-package-null": { rootMarkers: [{ packageJsonName: null }] }, "marker-package-empty": { rootMarkers: [{ packageJsonName: "" }] } })) add("config", name, { config: { ...minimal, project } });
for (const key of ["base", "project", "env", "client", "target", "group", "layer"]) for (const [name, value] of key === "base" ? [["null", null], ["string", "alpha"], ["entry", [""]], ["empty", []]] : [["null", null], ["empty", " "], ["number", 3]]) add("config", `tags-${key}-${name}`, { config: { ...minimal, tags: { [key]: value } } });
add("config", "duplicate-target", { config: { targets: [target, target] } });
add("config", "validation-order", { config: { targets: [target, { name: "a b", output: "x" }], project: 3 } });
add("config", "duplicate-after-tags", { config: { targets: [target, target], tags: 3 } });
add("config", "unknown-keys", { config: { ...minimal, ignored: 1, project: { extra: true }, targets: [{ ...target, extra: 1 }], tags: { unknown: null } } });
for (const [name, val] of [["bom", "\uFEFF"], ["nel", "\u0085"], ["untrimmed", " apps/api/.env "], ["newline-id", "alpha\n"]]) add("config", `trim-${name}`, { config: { targets: [{ name: name === "newline-id" ? val : "api", output: name === "newline-id" ? ".env" : val }] } });

const outputs = ["apps/api/.env", "./.env", "apps//api/./.env", "apps/api/", ".", "./", "a/..", "a/../../x", "../outside/.env", "..env", "..foo/.env", "{OUTSIDE}/.env", "~/.env", "\\\\server\\share\\x", "C:\\x", "a\\..\\..\\x", "", "\uFEFF", "\u0085", "{DRIVE}.env"];
outputs.forEach((output, i) => add("path", `output-${i}`, { output }));
for (const [index, output] of ["\\foo", "C:\\x", "\\\\srv\\sh\\x", "\\\\?\\C:\\x", "/foo", "{DRIVE}x", "{DRIVE}", "con", "nul"].entries()) add("path", `windows-${index}`, { output, platforms: ["windows"] });
add("path", "relative-root", { root: "relative-root", output: "apps/api/.env" });
add("path", "missing-root", { root: "{TMP}/missing", output: "apps/api/.env" });
add("path", "symlink-parent", { output: "linked/.env", files: [{ path: "linked", link: "{OUTSIDE}" }] });
add("path", "symlink-root", { root: "{TMP}/linked-root", output: ".env", files: [{ path: "{TMP}/linked-root", link: "{ROOT}" }] });
add("path", "symlink-normalized-away", { output: "linked/../.env", files: [{ path: "linked", link: "{OUTSIDE}" }] });
add("path", "enotdir", { output: "afile/sub/.env", files: [{ path: "afile", content: "alpha" }] });
add("inspect", "parent-existence", { config, files: [{ path: "apps/api", type: "dir" }] });
for (const [name, marker, files] of [
  ["success", { path: "services", packageJsonName: "demo-suite" }, [{ path: "services", type: "dir" }, { path: "package.json", content: '{"name":"demo-suite"}' }]],
  ["missing-path", { path: "services" }, []], ["missing-package", { packageJsonName: "demo-suite" }, []],
  ["wrong-name", { packageJsonName: "demo-suite" }, [{ path: "package.json", content: '{"name":"alpha"}' }]],
  ["bad-json", { packageJsonName: "demo-suite" }, [{ path: "package.json", content: "{" }]],
  ["bom-json", { packageJsonName: "demo-suite" }, [{ path: "package.json", content: '\uFEFF{"name":"demo-suite"}' }]],
  ["nonobject", { packageJsonName: "demo-suite" }, [{ path: "package.json", content: "[]" }]],
  ["directory-package", { packageJsonName: "demo-suite" }, [{ path: "package.json", type: "dir" }]],
  ["path-before-package", { path: "missing", packageJsonName: "demo-suite" }, []],
  ["path-escape", { path: "..env" }, []],
  ["path-symlink", { path: "linked" }, [{ path: "linked", link: "{OUTSIDE}" }]],
] as any[]) add("inspect", `marker-${name}`, { config: { ...minimal, project: { rootMarkers: [marker] } }, files, ...(["bad-json", "bom-json"].includes(name) ? { divergence: "a5" } : {}) });
add("inspect", "markers-and", { config: { ...minimal, project: { rootMarkers: [{ path: "services" }, { path: "missing" }] } }, files: [{ path: "services", type: "dir" }] });

for (const [name, conf, params] of [
  ["default", config, { env: "local", client: "alpha", target: "api", group: "apis", layer: "shared" }],
  ["fallback-cwd", { ...minimal, tags: { base: ["p:{project}"] } }, { env: "local" }],
  ["empty-project", { ...minimal, project: { id: "" }, tags: { base: ["p:{project}"] } }, { env: "local" }],
  ["unknown-and-escaping", { ...minimal, tags: { base: ["{unknown}", "{ env }", "{{env}}", "{bad.dot}", "{env}-{target}"] } }, { env: "local" }],
  ["dedup", { ...config, tags: { base: ["{target}", "x", "x"] } }, { env: "local" }],
  ["empty-rendered", { ...minimal, tags: { base: ["{target}", "x", "x"], env: "{client}" } }, { env: "local" }],
  ["prototype", { ...minimal, tags: { base: ["x:{constructor}", "{__proto__}", "{toString}"] } }, {}],
  ["comma", { ...minimal, tags: { base: ["a,b", "c"] } }, { env: "local" }],
] as any[]) add("tags", name, { config: conf, params, ...(name === "prototype" ? { divergence: "a1" } : {}) });
add("tags", "cwd-differs-root", { config: { ...minimal, tags: { base: ["p:{project}"] } }, cwd: "{TMP}/alpha", files: [{ path: "{TMP}/alpha", type: "dir" }] });

const classify: Record<string, string[]> = {
  shared: ["layer:shared"], group: ["layer:group", "group:apis"], target: ["layer:target", "target:api"], client: ["layer:client", "client:alpha"], "client-target": ["layer:client-target", "client:alpha", "group:apis"],
  "no-layer": [], "unknown-layer": ["layer:foo"], "foreign-client": ["layer:shared", "client:beta"], "foreign-target": ["layer:shared", "target:web"], "foreign-group": ["layer:shared", "group:other"],
  "shared-extra": ["layer:shared", "misc"], "shared-empty-scope": ["layer:shared", "client:"],
  "group-extra-group": ["layer:group", "group:apis", "group:other"], "group-has-client": ["layer:group", "group:apis", "client:alpha"], "group-has-target": ["layer:group", "group:apis", "target:api"],
  "target-multiple": ["layer:target", "target:api", "target:web"], "target-has-group": ["layer:target", "target:api", "group:apis"], "target-has-client": ["layer:target", "target:api", "client:alpha"],
  "client-multiple": ["layer:client", "client:alpha", "client:beta"], "client-group": ["layer:client", "client:alpha", "group:apis"], "client-has-target": ["layer:client", "client:alpha", "target:api"],
  "client-target-has-target": ["layer:client-target", "client:alpha", "group:apis", "target:api"], "client-target-no-group": ["layer:client-target", "client:alpha"], "client-target-foreign-group": ["layer:client-target", "client:alpha", "group:other"],
  "branch-precedence": ["layer:shared", "layer:client", "client:alpha"], "client-before-client-target": ["layer:client", "layer:client-target", "client:alpha", "group:apis"],
};
for (const [name, tags] of Object.entries(classify)) add("classify", name, { config, item: item("alpha", "alpha", tags) });
for (const tag of common) add("classify", `missing-common-${common.indexOf(tag)}`, { config, item: { id: "alpha", title: "alpha", tags: common.filter(t => t !== tag).concat("layer:shared") } });
add("classify", "missing-tags", { config, item: { id: "alpha", title: "alpha" } });
add("classify", "client-group-other-target", { config, item: item("alpha", "alpha", ["layer:client", "client:alpha", "group:apis"]), params: { target: "web" } });
for (const [name, term] of [["cr", "\r"], ["lf", "\n"], ["ls", "\u2028"], ["ps", "\u2029"]]) for (const scope of ["client", "target", "group"]) for (const position of ["middle", "end"]) add("classify", `scope-${scope}-${name}-${position}`, { config, item: item("alpha", "alpha", ["layer:shared", `${scope}:fore${position === "middle" ? term + "ign" : "ign" + term}`]) });
for (const scope of ["client", "target", "group"]) add("classify", `custom-${scope}`, { config: { ...config, tags: { base: ["envswitch"], client: "scope/client/[{client}]", target: "scope/target/({target})", group: "scope/group/{group}." } }, item: item("alpha", "alpha", ["layer:shared", scope === "client" ? "scope/client/[beta]" : scope === "target" ? "scope/target/(web)" : "scope/group/other."]) });
add("classify", "bare-placeholder", { config: { ...config, tags: { ...config.tags, client: "{client}" } }, item: item("alpha", "alpha", ["layer:shared"]) });
add("classify", "constant-scope", { config: { ...config, tags: { ...config.tags, client: "fixed" } }, item: item("alpha", "alpha", ["layer:shared", "fixed"]) });
add("classify", "repeated-placeholder", { config: { ...config, tags: { ...config.tags, client: "c:{client}/{client}" } }, item: item("alpha", "alpha", ["layer:shared", "c:alpha/beta"]) });
add("classify", "empty-layer-template", { config: { ...config, tags: { ...config.tags, layer: "{target}" } }, item: item("alpha", "alpha", ["layer:shared"]) });

// Full layer order, API read order, cache sharing, overwritten keys, and byte winners.
const layerItems = [item("shared", "shared", ["layer:shared"]), item("group-api", "api-group", ["layer:group", "group:apis"]), item("group-web", "app-group", ["layer:group", "group:apps"]), item("target-api", "api-target", ["layer:target", "target:api"]), item("target-web", "web-target", ["layer:target", "target:web"]), item("client", "alpha", ["layer:client", "client:alpha"]), item("ct-api", "alpha-api", ["layer:client-target", "client:alpha", "group:apis"]), item("ct-web", "alpha-web", ["layer:client-target", "client:alpha", "group:apps"]), item("ignored", "ignored", ["layer:client-target", "client:alpha", "target:web"])];
const values = Object.fromEntries(layerItems.map(i => [i.id, { COMMON: i.title, [i.id.replace(/-/g, "_").toUpperCase()]: "1" }]));
for (const [name, options, log] of [["all-cache", { targetName: "all" }, true], ["dry-api", { targetName: "api", dryRun: true }, true], ["write-web", { targetName: "web" }, true], ["silent-library", { targetName: "api", dryRun: true }, false]] as any[]) add("switch", name, { config, options, log, op: { items: layerItems, values } });
add("cli", "switch-full-summary", { argv: switchArgs, config, op: { items: layerItems, values } });
for (const [name, options] of Object.entries({ "bad-env": { env: "bad env" }, "bad-client": { client: "bad client" }, "unlisted-env": { env: "other" }, "unlisted-client": { client: "other" }, "bad-target": { targetName: "bad target" }, "missing-target": { targetName: "worker" } })) add("switch", name, { config, options });
add("switch", "empty-lists-allow", { candidateTags: ["env:other"], config: { ...minimal, clients: [], environments: [] }, options: { env: "other", client: "other" } });
add("switch", "target-named-all", { config: { targets: [{ name: "all", output: "all/.env" }, target] }, options: { targetName: "all" } });
add("switch", "all-paths-before-op", { config: { targets: [target, { name: "web", output: "../outside" }] } });
add("switch", "empty-merge", { config: minimal });
add("switch", "overwrite-identical-and-reverted", { config: minimal, op: { items: ["a", "b", "c"].map(id => ({ id, title: id, tags: ["env:local", "layer:shared"] })), values: { a: { Z: "1", A: "x", SAME: "same" }, b: { Z: "2", A: "y", SAME: "same" }, c: { Z: "1", A: "x", SAME: "same" } } } });
add("switch", "duplicate-item", { config: minimal, op: { items: [{ id: "alpha", title: "alpha", tags: ["env:local", "layer:shared"] }, { id: "alpha", title: "alpha", tags: ["env:local", "layer:shared"] }], values: { alpha: { A: "1" } } } });
add("switch", "title-cache-key", { config: minimal, op: { items: [{ title: "alpha", tags: ["env:local", "layer:shared"] }], values: { alpha: { A: "1" } } } });
const collation = {
  "expanded": ["_x", "-x", "~t", "1abc", "a", "A", "a b", "a.b", "ab", "alpha", "b", "B", "éclair", "ezra", "item10", "item2", "x_1", "x-1", "Zeta", "ä", "z", "e\u0301", "é", "Ω", "中", "あ"],
  "accent-winner": ["z", "ä"], "canonical-equal": ["é", "e\u0301"], "punctuation-winner": ["-x", "_x"], "case-winner": ["A", "a"], "numeric-winner": ["item2", "item10"], "non-latin": ["中", "Ω", "あ"], "equal-title-api-order": ["same", "same", "same"],
};
for (const [name, titles] of Object.entries(collation)) add("switch", `collation-${name}`, { config: minimal, op: { items: titles.map((title, i) => ({ id: `alpha-${i}`, title, tags: ["env:local", "layer:shared"] })), values: Object.fromEntries(titles.map((title, i) => [`alpha-${i}`, { WINNER: `${i}:${title}` }])) } });
add("switch", "symlink-swap-before-write", { config: minimal, files: [{ path: "apps/api", type: "dir" }], op: { items: [{ id: "alpha", title: "alpha", tags: ["env:local", "layer:shared"] }], values: { alpha: { A: "1" } } }, mutateOnRead: { item: "alpha", files: [{ path: "apps/api", remove: true }, { path: "apps/api", link: "{OUTSIDE}" }] } });
add("switch", "nontransactional-writes", { config, op: { items: [item("alpha", "alpha", ["layer:shared"])], values: { alpha: { A: "1" } } }, files: [{ path: "apps/web/.env", type: "dir" }] });

const dotenvValues = ["", "plain", "two words", "a=b", 'q"x', "s'x", "b`x", "$HOME", "back\\slash", "hash#", "tab\t", "nl\nx", "cr\rx", "\u0001ctl", "nbsp\u00A0", "nel\u0085", "bom\uFEFF", "ls\u2028", "é", "\u007F", "x\u000B", "\u0000"];
dotenvValues.forEach((v, i) => add("dotenv", `serialize-${String(i + 1).padStart(2, "0")}`, { values: { K: v } }));
add("dotenv", "sort-empty", { values: { B: "two words", A: "1", EMPTY: "" } });
add("dotenv", "empty-map", { values: {} });
add("dotenv", "ps", { values: { K: "ps\u2029" } });
for (const [index, key] of ["BAD-KEY", "9KEY", "", "K\n", "é"].entries()) add("dotenv", `invalid-${index}`, { values: { [key]: "alpha" } });
add("dotenv", "invalid-sorted-first", { values: { "B-1": "alpha", "A-1": "beta" } });
for (const umask of [0o022, 0o077, 0]) add("dotenv", `write-mode-${umask}`, { values: { TOKEN: "alpha", NAME: "demo-suite" }, write: true, umask });
add("dotenv", "replace-existing", { values: { A: "new" }, write: true, output: ".env", files: [{ path: ".env", content: "A=old\n", mode: 0o644 }] });
add("dotenv", "invalid-preserves-existing", { values: { "BAD-KEY": "nope" }, write: true, output: ".env", files: [{ path: ".env", content: "EXISTING=1\n", mode: 0o600 }] });
add("dotenv", "invalid-before-mkdir", { values: { "BAD-KEY": "nope" }, write: true, output: "missing/.env" });
add("dotenv", "rename-cleanup", { values: { A: "1" }, write: true, output: ".env", files: [{ path: ".env", type: "dir" }] });
add("dotenv", "mkdir-enotdir", { values: { A: "1" }, write: true, output: "afile/sub/.env", files: [{ path: "afile", content: "alpha" }] });
add("dotenv", "replace-symlink", { values: { A: "new" }, write: true, output: ".env", platforms: ["macos", "linux"], files: [{ path: "{OUTSIDE}/alpha", content: "A=old\n" }, { path: ".env", link: "{OUTSIDE}/alpha", type: "file" }] });

// Exact command argv and lenient parsing through exported OnePasswordClient methods.
for (const [name, response] of [["normal", [{ id: "alpha", name: "alpha" }]], ["fallback", [{ id: "alpha" }, { name: "beta" }]], ["nonarray", {}], ["entry-null", [null]], ["entry-empty", [{}]], ["entry-number", [{ id: 1, name: false }]]]) add("op", `vault-${name}`, { method: "vaults", op: { responses: [raw(["vault", "list", "--format", "json"], JSON.stringify(response))] } });
for (const [name, stdout] of [["malformed", "not-json"], ["empty", ""], ["nonarray", "{}"], ["null-entry", "[null]"], ["empty-entry", "[{}]"], ["lenient", '[{"id":1,"title":"alpha","tags":["a",1,null,"b"]},{"id":"beta","title":null,"tags":"x"}]']]) add("op", `item-list-${name}`, { method: "items", op: { responses: [raw(list(), stdout)] }, ...(["malformed", "empty"].includes(name) ? { divergence: "a5" } : {}) });
add("op", "tags-comma-argv", { method: "items", tags: ["a,b", "c"], candidateTags: ["a,b,c"] });
for (const [name, detail] of [["no-fields", {}], ["array", []], ["fields-null", { fields: null }], ["null-field", { fields: [null] }], ["empty-fields", { fields: [] }], ["empty-config", { fields: [{ label: "config", value: " " }] }], ["untrimmed-label", { fields: [{ label: " config", value: "{}" }] }], ["first-config", { fields: [{ label: "config", value: "" }, { label: "config", value: "{}" }] }], ["untrimmed-value", { fields: [{ label: "config", value: "  {}  " }] }], ["nonstring-config", { fields: [{ label: "config", value: 1 }] }]]) add("op", `config-${name}`, { method: "config", op: { details: { cfg: detail } } });
add("op", "env-labels", { method: "env", item: { id: "alpha", title: "alpha" }, op: { details: { alpha: { fields: [
  { id: "IGNORED_ID", label: " VALID_KEY ", value: "1" }, { label: "VALID_KEY", value: "2", section: { id: "beta" } }, { label: "invalid-key", value: "ignored" }, { label: "9KEY", value: "ignored" }, { label: "notesPlain", value: "ignored" }, { label: " notesPlain ", value: "ignored" }, { label: "", value: "ignored" }, { label: "\uFEFFBOM\uFEFF", value: "1" }, { label: "\u0085NEL", value: "ignored" }, { label: "NOVAL" }, { label: "NUMBER", value: 5 }, { label: "username", value: "alpha" }, { label: "password", value: "demo-suite" }, { label: "OTP", value: "otpauth://demo-suite" }, { label: null, value: "ignored" },
] } } } });
add("op", "env-proto", { method: "env", item: { id: "alpha", title: "alpha" }, divergence: "a2", op: { details: { alpha: { fields: [{ label: "__proto__", value: "alpha" }, { label: "A", value: "1" }] } } } });
add("op", "title-fallback-argv", { method: "env", item: { id: "", title: "My Title" }, op: { details: { "My Title": { fields: [] } } } });
add("op", "leading-dash-argv", { method: "config", item: "-alpha", op: { details: { "-alpha": { fields: [{ label: "config", value: "{}" }] } } } });
add("op", "get-malformed", { method: "config", divergence: "a5", op: { responses: [raw(get(), "{")] } });
add("op", "auth-stdout-ignored", { method: "auth", op: { responses: [raw(["whoami", "--format", "json"], "not-json")] } });

// Guided paths use prompt answers only; no terminal UI is synthesized.
const guided = { config, argv: [], prompts: ["alpha", "local", "all", true] };
add("cli", "guided-auto-single", { argv: [], config: { ...minimal, clients: ["alpha"], environments: ["local"] }, prompts: [true] });
add("cli", "guided-dry", guided);
add("cli", "guided-write-default", { ...guided, prompts: ["alpha", { default: true }, { default: true }, { default: true }] });
add("cli", "guided-input-fallback", { config: minimal, argv: [], prompts: ["alpha", { default: true }, true] });
add("cli", "guided-empty-arrays", { config: { ...minimal, clients: [], environments: [] }, argv: [], prompts: ["alpha", "local", true] });
add("cli", "guided-env-first-default", { config: { ...config, environments: ["dev", "prod"] }, argv: [], prompts: ["alpha", { default: true }, { default: true }, true] });
add("cli", "guided-whitespace-input", { config: minimal, argv: [], prompts: ["  ", "local", true] });
add("cli", "guided-non-tty", { argv: [], isInteractive: false });
add("cli", "guided-stdin-not-tty", { argv: [], tty: { stdin: false, stdout: true, stderr: true } });
add("cli", "guided-stdout-not-tty", { argv: [], tty: { stdin: true, stdout: false, stderr: true } });
add("cli", "guided-fake-bypasses-guard", { argv: [], isInteractive: false, config: { ...minimal, clients: ["alpha"], environments: ["local"] }, prompts: [true] });
add("cli", "guided-no-vaults", { argv: [], prompts: [], op: { vaults: [] } });
add("cli", "guided-no-configs", { argv: [], prompts: [], op: { configItems: [] } });
const vaults = [{ id: "alpha", name: "alpha vault" }, { id: "beta", name: "beta vault" }];
add("cli", "guided-all-prompts", { config, argv: [], op: { vaults, byVault: { beta: { configItems: [{ id: "cfg", title: "demo-suite" }, { id: "cfg-beta", title: "beta config" }] } } }, prompts: ["beta", "cfg", "beta", "dev", "web", true] });
add("cli", "guided-parallel-reuse", { config, argv: [], parallelScans: 2, op: { vaults, byVault: { beta: { configItems: [] } } }, prompts: ["alpha", "local", "all", true] });
add("cli", "guided-skip-unreadable", { config, argv: [], parallelScans: 2, op: { vaults, responses: [raw(list("envswitch-config", "beta"), "", 1, "unreadable")] }, prompts: ["alpha", "local", "all", true] });
add("cli", "guided-cancel", { config, argv: [], op: { vaults }, prompts: [{ cancel: true }] });
add("cli", "guided-abort-is-not-cancel", { config, argv: [], op: { vaults }, prompts: [{ error: { name: "AbortPromptError", message: "aborted" } }] });
add("cli", "guided-other-prompt-error", { config, argv: [], op: { vaults }, prompts: [{ error: { name: "Error", message: "demo-suite prompt failure" } }] });

const installFiles = [{ path: "{APP}/scripts/{INSTALLER}", content: "fixture only; never executed" }];
const installed = [...installFiles, { path: "{APP}/.version", content: "v0.0.4\n" }];
for (const [name, files, update, divergence] of [
  ["same", installed, { latest: "0.0.4" }, "e1"], ["install", installed, { latest: "v0.0.5", updatedVersion: "v0.0.5\n" }, "e3"],
  ["source-checkout", [...installed, { path: "{APP}/.git", type: "dir" }], {}, "e1"], ["missing-installer", [], {}, "e2"],
  ["failed-installer", installed, { latest: "v0.0.5", exitCode: 1 }, "e1"], ["unknown-current", installFiles, { latest: "v0.0.5", updatedVersion: "v0.0.5\n" }, "e1"],
  ["unknown-latest", installed, {}, "e1"], ["empty-current", [...installFiles, { path: "{APP}/.version", content: " \n" }], {}, "e1"],
  ["no-reread-fallback", installFiles, { latest: "v0.0.5" }, "e1"], ["fetch-throws", installed, { fetchError: "demo-suite fetch failure" }, "e1"],
  ["installer-throws", installed, { installerError: "demo-suite installer failure" }, "e2"],
] as any[]) add("cli", `update-${name}`, { argv: ["update", "ignored"], files, update, divergence: name === "install" ? ["e1", "e2", "e3"] : [...new Set(["e1", "e2", divergence])] });

for (const [name, argv, extra] of [
  ["help", ["--help"], {}], ["guard", [], {}], ["doctor", doctorArgs, { config }], ["list", listArgs, { config }],
  ["switch", switchArgs, { config, op: { items: layerItems, values } }], ["dry-run", [...switchArgs, "--dry-run"], { config, op: { items: layerItems, values } }],
  ["failure-stdout-secret", doctorArgs, { op: { responses: [raw(["whoami", "--format", "json"], "DEMO_SUITE_PRIVATE", 7, "  safe failure\r\n")] } }],
  ["item-failure", listArgs, { op: { responses: [raw(get(), "DEMO_SUITE_PRIVATE", 3, "safe item failure\n")] } }],
  ["failure-empty-stderr", doctorArgs, { op: { responses: [raw(["whoami", "--format", "json"], "DEMO_SUITE_PRIVATE", 7)] } }],
  ["malformed-list", [...listArgs, "clients"], { configRaw: "{" , divergence: "a5" }],
] as any[]) add("blackbox", name, { argv, ...extra });


add("inspect", "nonexistent-root-no-markers", { config: minimal, root: "{TMP}/missing" });
add("classify", "foreign-group-only", { config, item: item("alpha", "alpha", ["layer:group", "group:other"]) });
add("classify", "foreign-target-only", { config, item: item("alpha", "alpha", ["layer:target", "target:web"]) });
add("classify", "foreign-client-only", { config, item: item("alpha", "alpha", ["layer:client", "client:beta"]) });
add("classify", "foreign-client-target", { config, item: item("alpha", "alpha", ["layer:client-target", "client:beta", "group:apps"]), params: { target: "web" } });
add("cli", "guided-skip-shape-error", { config, argv: [], op: { vaults, responses: [raw(list("envswitch-config", "beta"), "{}")] }, prompts: ["alpha", "local", "all", true] });
add("cli", "guided-id-title-fallback", { config: { ...minimal, clients: ["alpha"], environments: ["local"] }, argv: [], op: { vaults: [{ name: "alpha" }], configItems: [{ title: "cfg" }] }, prompts: [true] });
add("switch", "read-failure-aborts", { config, op: { items: layerItems, values, responses: [raw(get("shared"), "DEMO_SUITE_PRIVATE", 1, "safe failure")] } });
add("op", "vault-malformed", { method: "vaults", divergence: "a5", op: { responses: [raw(["vault", "list", "--format", "json"], "{")] } });
add("op", "env-null-fields", { method: "env", item: { id: "alpha", title: "alpha" }, op: { details: { alpha: { fields: [null] } } } });
add("op", "lone-surrogate-value", { divergence: "a7", method: "env", item: { id: "alpha", title: "alpha" }, op: { responses: [raw(get("alpha"), '{"fields":[{"label":"A","value":"\\ud800"}]}')] } });
add("blackbox", "maxbuffer-failure", { argv: doctorArgs, op: { responses: [{ argv: ["whoami", "--format", "json"], stdoutRepeat: { value: "x", count: 52428801 }, stderr: "", exitCode: 0 }] } });


add("blackbox", "missing-op", { argv: doctorArgs, missingOp: true, divergence: "a6" });
add("cli", "guided-vault-list-failure", { argv: [], prompts: [], op: { responses: [raw(["vault", "list", "--format", "json"], "", 7, "safe vault failure")] } });
add("cli", "guided-vault-list-malformed", { argv: [], prompts: [], divergence: "a5", op: { responses: [raw(["vault", "list", "--format", "json"], "{")] } });
add("cli", "guided-cancel-mode", { argv: [], config: { ...minimal, clients: ["alpha"], environments: ["local"] }, prompts: [{ cancel: true }] });
add("cli", "guided-duplicate-vault-ids", { argv: [], config: { ...minimal, clients: ["alpha"], environments: ["local"] }, op: { vaults: [{ id: "alpha", name: "alpha" }, { id: "alpha", name: "beta" }] }, prompts: [{ index: 1 }, true] });
add("cli", "guided-duplicate-item-ids", { argv: [], config: { ...minimal, clients: ["alpha"], environments: ["local"] }, op: { configItems: [{ id: "cfg", title: "alpha" }, { id: "cfg", title: "beta" }] }, prompts: [{ index: 1 }, true] });
add("blackbox", "maxbuffer-exact-limit", { argv: doctorArgs, expectOpCalls: true, op: { responses: [{ argv: ["whoami", "--format", "json"], stdoutSize: { byte: 120, size: 52428800 }, stderr: "", exitCode: 0 }] } });
add("blackbox", "non-utf8-stdout", { argv: doctorArgs, expectOpCalls: true, op: { responses: [{ argv: ["whoami", "--format", "json"], stdoutBase64: "/wD+", stderr: "", exitCode: 0 }] } });
add("blackbox", "non-utf8-json", { divergence: "a5", argv: listArgs, expectOpCalls: true, op: { responses: [{ argv: get(), stdoutBase64: Buffer.from('{"fields":[{"label":"config","value":"').toString("base64") } ] } });
// Invalid UTF-8 inside otherwise valid JSON decodes to U+FFFD before JSON parsing.
scenarios.at(-1).op.responses[0].stdoutBase64 = Buffer.concat([Buffer.from('{"fields":[{"label":"config","value":"'), Buffer.from([255]), Buffer.from('"}]}')]).toString("base64");
add("path", "eacces", { output: "locked/sub/.env", platforms: ["macos", "linux"], divergence: "d1", files: [{ path: "locked", type: "dir", mode: 0 }] });
add("cli", "update-extra-positional", { argv: ["update", "extra"], files: installed, update: { latest: "0.0.4" }, divergence: ["e1", "e2", "e3"] });
for (const s of scenarios) if (s.id.startsWith("cli-update-")) {
  s.divergence = ["e1", "e2", "e3"];
  s.rustUpdate = { repo: "demo-suite/alpha", installDir: "{TMP}/launcher" };
}
for (const s of scenarios) if (["path-enotdir", "dotenv-mkdir-enotdir", "dotenv-rename-cleanup", "switch-nontransactional-writes", "inspect-marker-directory-package"].includes(s.id)) s.divergence = "d1";

const ids = new Set();
const check = process.argv.includes("--check");
for (const s of scenarios) {
  if (ids.has(s.id)) throw new Error(`duplicate id: ${s.id}`); ids.add(s.id);
  const serialized = JSON.stringify(s, null, 2) + "\n";
  if (check) { if (!await Bun.file(join(dir, s.id + ".json")).exists() || await Bun.file(join(dir, s.id + ".json")).text() !== serialized) throw new Error(`scenario generator drift: ${s.id}`); }
  else await writeFile(join(dir, s.id + ".json"), serialized);
}
for (const file of await readdir(dir)) if (file.endsWith(".json") && !ids.has(file.slice(0, -5))) { if (check) throw new Error(`stale scenario: ${file}`); await rm(join(dir, file)); }
console.log(`Wrote ${scenarios.length} input scenarios`);
