#!/usr/bin/env bun
import { mkdir, readdir, writeFile, readFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
export const ORACLE = "53de029a25a717c8ac824ca8553035e2b0fec991";
export async function materialize(destination: string) {
  if (Bun.version !== "1.4.0") throw new Error(`oracle requires Bun 1.4.0, got ${Bun.version}`);
  const dir = resolve(destination);
  await mkdir(dir, { recursive: true });
  if ((await readdir(dir)).length) throw new Error(`oracle destination must be empty: ${dir}`);
  const repo = fileURLToPath(new URL("../../", import.meta.url));
  const archive = Bun.spawn(["git", "-C", repo, "-c", "core.autocrlf=false", "archive", ORACLE], { stdout: "pipe", stderr: "inherit" });
  const extract = Bun.spawn(["tar", "-xf", "-", "-C", dir], { stdin: archive.stdout, stderr: "inherit" });
  if (await extract.exited || await archive.exited) throw new Error("git archive/extraction failed");
  const home = join(dir, ".parity-home");
  await mkdir(home);
  const env = { ...process.env, LANG: "en_US.UTF-8", LC_ALL: "en_US.UTF-8", HOME: home, USERPROFILE: home,
    XDG_CACHE_HOME: join(home, "cache"), XDG_CONFIG_HOME: join(home, "config"), XDG_DATA_HOME: join(home, "data"),
    LOCALAPPDATA: join(home, "local"), APPDATA: join(home, "roaming"), BUN_INSTALL_CACHE_DIR: join(home, "bun-cache"), BUN_RUNTIME_TRANSPILER_CACHE_PATH: "" };
  const install = Bun.spawn([process.execPath, "install", "--frozen-lockfile"], { cwd: dir, env, stdout: "inherit", stderr: "inherit" });
  if (await install.exited) throw new Error("oracle bun install failed");
  const probe = Bun.spawn([process.execPath, "-e", 'console.log(JSON.stringify({bun:Bun.version,icu:process.versions.icu,unicode:process.versions.unicode,locale:Intl.DateTimeFormat().resolvedOptions().locale}))'], { env, stdout: "pipe" });
  const metadata = { commit: ORACLE, ...JSON.parse(await new Response(probe.stdout).text()), LANG: env.LANG, LC_ALL: env.LC_ALL };
  if (await probe.exited || metadata.locale !== "en-US") throw new Error("oracle locale must resolve to en-US");
  await writeFile(join(dir, "oracle-metadata.json"), JSON.stringify(metadata, null, 2) + "\n");
  await verifyOracle(dir);
  return dir;
}
// Hash exact blob bytes, executable bits and filenames against the immutable Git tree.
export async function verifyOracle(dir: string) {
  const repo = fileURLToPath(new URL("../../", import.meta.url));
  const git = Bun.spawn(["git", "-C", repo, "ls-tree", "-rz", `${ORACLE}:src`], { stdout: "pipe", stderr: "inherit" });
  const tree = (await new Response(git.stdout).text()).split("\0").filter(Boolean);
  if (await git.exited) throw new Error("cannot read pinned oracle src tree");
  const actual: string[] = [];
  async function walk(path: string, prefix = "") {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const name = prefix + entry.name;
      if (entry.isDirectory()) await walk(join(path, entry.name), name + "/");
      else { if (!entry.isFile()) throw new Error(`unexpected oracle source entry: ${name}`); actual.push(name); }
    }
  }
  await walk(join(dir, "src"));
  const expectedNames = tree.map(entry => entry.slice(entry.indexOf("\t") + 1));
  if (JSON.stringify(actual.sort()) !== JSON.stringify(expectedNames.sort())) throw new Error("oracle src tree filenames differ from pinned commit");
  const { createHash } = await import("node:crypto");
  const { stat } = await import("node:fs/promises");
  for (const entry of tree) {
    const [header, name] = entry.split("\t"), [mode, , hash] = header.split(" ");
    const bytes = await readFile(join(dir, "src", name));
    const actualHash = createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
    if (actualHash !== hash || (process.platform !== "win32" && Boolean((await stat(join(dir, "src", name))).mode & 0o111) !== (mode === "100755"))) throw new Error(`oracle src tree hash differs: ${name}`);
  }
}

if (import.meta.main) {
  if (process.argv.length !== 3) throw new Error("usage: bun parity/oracle/materialize.ts <empty-directory>");
  await materialize(process.argv[2]);
}
