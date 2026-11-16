import { mkdtemp, mkdir, writeFile, chmod, rm, symlink, readdir, lstat, readFile, readlink, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, dirname, sep, relative, isAbsolute } from "node:path";

export async function context(s: any, resume?: { tmp: string; before: any }) {
  const previousUmask = process.umask(0o022);
  let tempToClean: string | undefined;
  try {
  const tmp = resume?.tmp ?? await realpath(await mkdtemp(join(tmpdir(), "envswitch-parity-")));
  tempToClean = tmp;
  const paths: Record<string, string> = { ROOT: join(tmp, "demo-suite"), APP: join(tmp, "app", "current"), HOME: join(tmp, "home"), OUTSIDE: join(tmp, "outside"), TMP: tmp, SEP: sep, INSTALLER: process.platform === "win32" ? "install.ps1" : "install.sh", DRIVE: process.platform === "win32" ? resolve(tmp).slice(0, 2) : "C:" };
  const expandString = (str: string) => str.replace(/\{([A-Z]+)\}/g, (match, key) => paths[key] ?? match);
  function map(value: any, transform: (s: string) => string): any {
    if (typeof value === "string") return transform(value);
    if (Array.isArray(value)) return value.map(v => map(v, transform));
    if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [transform(k), map(v, transform)]));
    return value;
  }
  const expand = (v: any) => map(v, expandString);
  const replacements = Object.entries(paths).filter(([k]) => !["SEP", "INSTALLER", "DRIVE"].includes(k)).sort((a, b) => b[1].length - a[1].length);
  const normalizeString = (str: string) => {
    // Random writer names occur in fs error messages as well as leftover filenames.
    if (str.includes(tmp)) str = str.replace(/\.\d+\.[0-9a-f]{8}-[0-9a-f-]{27}\.tmp/g, ".{PID}.{UUID}.tmp");
    for (const [key, value] of replacements) str = str.split(value).join(`{${key}}`);
    return str;
  };
  const normalize = (v: any) => map(v, normalizeString);
  const bytes = (data: string | Buffer) => {
    // Replace only known UTF-8 path sequences; leave all other bytes untouched.
    let buffer = Buffer.from(data);
    // latin1 is a reversible byte view, including malformed UTF-8.
    if (buffer.includes(Buffer.from(tmp))) buffer = Buffer.from(buffer.toString("latin1").replace(/\.\d+\.[0-9a-f]{8}-[0-9a-f-]{27}\.tmp/g, ".{PID}.{UUID}.tmp"), "latin1");
    for (const [key, value] of replacements) {
      const needle = Buffer.from(value), parts: Buffer[] = [];
      let pos = 0, next;
      while ((next = buffer.indexOf(needle, pos)) !== -1) { parts.push(buffer.subarray(pos, next), Buffer.from(`{${key}}`)); pos = next + needle.length; }
      parts.push(buffer.subarray(pos)); buffer = Buffer.concat(parts);
    }
    return buffer.toString("base64");
  };
  const safe = (path: string) => {
    const absolute = resolve(paths.ROOT, expandString(path));
    const rel = relative(tmp, absolute);
    if (rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel)) throw new Error(`fixture escapes sandbox: ${path}`);
    return absolute;
  };
  for (const key of ["ROOT", "APP", "HOME", "OUTSIDE"]) await mkdir(paths[key], { recursive: true });
  async function actions(entries: any[] = []) {
    for (const f of entries) {
      const path = safe(f.path);
      if (f.remove) { await rm(path, { recursive: true, force: true }); continue; }
      await mkdir(dirname(path), { recursive: true });
      if (f.type === "dir") await mkdir(path, { recursive: true });
      else if (f.link) {
        const target = safe(f.link);
        await symlink(target, path, process.platform === "win32" && f.type !== "file" ? "junction" : f.type === "file" ? "file" : "dir");
      } else await writeFile(path, f.base64 !== undefined ? Buffer.from(f.base64, "base64") : expandString(f.content ?? ""));
      if (f.mode !== undefined) await chmod(path, f.mode);
    }
  }
  async function snapshot() {
    const files: any[] = [], directories: any[] = [];
    async function walk(dir: string) {
      for (const name of (await readdir(dir)).sort()) {
        if (dir === tmp && name === ".transport") continue;
        const path = join(dir, name), stat = await lstat(path), mode = process.platform === "win32" ? null : stat.mode & 0o777;
        if (stat.isSymbolicLink()) files.push({ path: normalizeString(path), kind: "symlink", target: normalizeString((await readlink(path)).replace(/^\\\\\?\\/, "")), mode });
        else if (stat.isDirectory()) {
          directories.push({ path: normalizeString(path), mode });
          try { await walk(path); } catch (error: any) { if (error.code !== "EACCES") throw error; }
        }
        else files.push({ path: normalizeString(path), kind: "file", bytesBase64: bytes(await readFile(path)), mode });
      }
    }
    await walk(tmp);
    return { files, directories };
  }
  const env = { ...process.env };
  for (const key of ["CI", "NO_COLOR", "FORCE_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "GITHUB_TOKEN", "OP_SERVICE_ACCOUNT_TOKEN", "OP_SESSION"]) delete env[key];
  Object.assign(env, { HOME: paths.HOME, USERPROFILE: paths.HOME, XDG_CACHE_HOME: join(paths.HOME, "cache"), XDG_CONFIG_HOME: join(paths.HOME, "config"), XDG_DATA_HOME: join(paths.HOME, "data"), LOCALAPPDATA: join(paths.HOME, "local"), APPDATA: join(paths.HOME, "roaming"), LANG: "en_US.UTF-8", LC_ALL: "en_US.UTF-8", BUN_RUNTIME_TRANSPILER_CACHE_PATH: "" });
  for (const [key, value] of Object.entries(s.env ?? {})) { if (value === null) delete env[key]; else env[key] = String(value); }
  // HOME/profile locations are never scenario-overridable.
  for (const key of ["HOME", "USERPROFILE", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "LOCALAPPDATA", "APPDATA", "BUN_RUNTIME_TRANSPILER_CACHE_PATH"]) if (s.env?.[key] !== undefined) throw new Error(`cannot override ${key}`);
  if (!resume) await actions(s.files);
  const before = resume?.before ?? await snapshot();
  return { tmp, paths, safe, expand, normalize, normalizeString, bytes, actions, snapshot, before, env, cleanup: async () => { for (const f of s.files ?? []) if (f.type === "dir" && f.mode !== undefined) await chmod(safe(f.path), 0o755).catch(() => {}); await rm(tmp, { recursive: true, force: true }); } };
  } catch (error) {
    if (tempToClean) await rm(tempToClean, { recursive: true, force: true });
    throw error;
  } finally { process.umask(previousUmask); }
}

export function transcript(s: any) {
  return { schemaVersion: 1, id: s.id, kind: s.kind, divergence: s.divergence ?? null, stdoutBase64: "", stderrBase64: "", exitCode: 0, prompts: [] as any[], progress: [] as any[], opCalls: [] as string[][], readCounts: {} as Record<string, number>, factoryCalls: [] as string[], updateCalls: [] as any[], files: [] as any[], writtenFiles: [] as any[], directories: [] as any[], leftoverTempFiles: [] as string[], result: null as any };
}
export function countRead(counts: Record<string, number>, args: string[]) {
  if (args[0] === "item" && args[1] === "get") { const key = `${args[args.indexOf("--vault") + 1]}/${args[2]}`; counts[key] = (counts[key] ?? 0) + 1; }
}
export async function finish(t: ReturnType<typeof transcript>, c: Awaited<ReturnType<typeof context>>) {
  const state = await c.snapshot();
  Object.assign(t, state);
  t.writtenFiles = state.files.filter(f => !c.before.files.some((before: any) => JSON.stringify(before) === JSON.stringify(f)));
  t.leftoverTempFiles = state.files.filter(f => /\.tmp$/.test(f.path)).map(f => f.path);
  return JSON.parse(JSON.stringify(c.normalize(t)));
}
