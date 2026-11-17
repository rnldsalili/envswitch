// Protocol-1 synthetic install state and installer output; no installer or payload is executed.
import { join, dirname, normalize } from "node:path";
import type { context } from "../../context.ts";

type Sandbox = Awaited<ReturnType<typeof context>>;
export async function nativeFixture(c: Sandbox, s: any, expected: any, build: { version: string; target: string }) {
  if (!s.rustUpdate) return null;
  const windows = process.platform === "win32", suffix = windows ? ".exe" : "";
  // normalize: scenarios spell paths with `/`; on Windows the receipt validator requires native separators.
  const appRoot = join(c.tmp, "app"), installDir = normalize(s.rustUpdate.installDir);
  const canonical = (v: string) => "v" + v.trim().replace(/^v/, "");
  const currentTag = canonical(build.version);
  const disposition = s.rustUpdate.disposition ?? "installed";
  if (!["installed", "unchanged", "repaired"].includes(disposition)) throw new Error("invalid native installer disposition");
  const tag = disposition === "installed" ? canonical(s.update?.updatedVersion ?? s.update?.latest ?? "v0.0.5") : currentTag;
  const payload = (version: string) => join(c.paths.APP, "bin", `envswitch_${version.slice(1)}_${build.target}${suffix}`);
  const launcher = join(installDir, windows ? "envswitch.cmd" : "envswitch");
  const launcherBytes = (binary: string) => windows ? [
    "@echo off", "setlocal DisableDelayedExpansion",
    'for /f "tokens=2 delims=:." %%C in (\'chcp\') do set "_ENVSWITCH_CP=%%C"',
    "chcp 65001 >nul", `"${binary.replaceAll("%", "%%")}" %*`,
    'set "_ENVSWITCH_EXIT=%ERRORLEVEL%"', "chcp %_ENVSWITCH_CP% >nul 2>&1", "exit /b %_ENVSWITCH_EXIT%", "",
  ].join("\r\n") : `#!/usr/bin/env bash\nset -euo pipefail\nexec '${binary.replaceAll("'", "'\\''")}' "$@"\n`;
  const recipe = (version: string) => {
    const receipt = { schemaVersion: 1, kind: "native", repo: s.rustUpdate.repo, appRoot, installDir, launcherPath: launcher, target: build.target, tag: version, payloadPath: payload(version) };
    return [
      { path: payload(version), content: "fixture payload; never executed\n", mode: 0o755 },
      { path: launcher, content: launcherBytes(payload(version)), mode: 0o755 },
      { path: join(c.paths.APP, "install.json"), content: JSON.stringify(receipt) + "\n", mode: 0o644 },
      { path: join(c.paths.APP, ".version"), content: version + "\n", mode: 0o644 },
      ...["install.sh", "install.ps1"].map(name => ({ path: join(c.paths.APP, "scripts", name), content: "fixture only; never executed\n", mode: 0o644 })),
    ];
  };
  await c.actions(recipe(currentTag));
  // Structurally valid receipts retain their locations while a missing marker/script is repaired.
  if (disposition === "repaired") {
    const missingScripts = !(s.files ?? []).some((f: any) => f.path.includes(`${c.paths.APP}`) && f.path.includes("scripts"));
    if (missingScripts) await c.actions([{ path: join(c.paths.APP, "scripts"), remove: true }]);
    else await c.actions([{ path: join(c.paths.APP, ".version"), remove: true }]);
  }
  const before = await c.snapshot();
  const sourceGuard = s.files?.some((f: any) => normalize(f.path) === join(c.paths.APP, ".git"));
  const success = !sourceGuard && !s.update?.installerError && !s.update?.exitCode;
  const onInstall = success && disposition !== "unchanged" ? recipe(tag).filter(entry =>
    !before.files.some((file: any) => file.path === c.normalizeString(entry.path) && file.kind === "file" && file.bytesBase64 === c.bytes(entry.content) && (windows || file.mode === entry.mode))) : [];
  const guidance = windows ? `info: restart your terminal, or run this command now:\ninfo: $env:Path = "${installDir.replace(/[\x60"$]/g, (character: string) => "`" + character)};$env:Path"\ninfo: then run 'envswitch --help' to verify the installation\n`
    : "info: run 'envswitch --help' to verify the installation\n";
  const stdout = !success ? "" : disposition === "unchanged" ? `success: envswitch is already up to date (${tag})\n`
    : `success: ${disposition} envswitch ${tag}\ninfo: binary: ${payload(tag)}\ninfo: launcher: ${launcher}\n${guidance}`;
  const summary = tag !== currentTag || disposition === "installed" ? `updated envswitch ${currentTag} -> ${tag}\n`
    : disposition === "repaired" ? `repaired envswitch ${tag}\n` : `envswitch is already up to date (${tag})\n`;

  // Predict only the fake installer's declared writes; every product-side mutation still compares.
  const predicted = new Map<string, any>(before.files.map((f: any) => [f.path, f]));
  const directories = new Map<string, any>(before.directories.map((d: any) => [d.path, d]));
  for (const f of onInstall) {
    const path = c.normalizeString(f.path);
    predicted.set(path, { path, kind: "file", bytesBase64: c.bytes(f.content), mode: windows ? null : f.mode });
    for (let dir = dirname(f.path); dir !== c.tmp; dir = dirname(dir)) {
      const path = c.normalizeString(dir);
      if (!directories.has(path)) directories.set(path, { path, mode: windows ? null : 0o755 });
    }
  }
  const sort = (a: any, b: any) => c.expand(a.path) < c.expand(b.path) ? -1 : c.expand(a.path) > c.expand(b.path) ? 1 : 0;
  expected.files = [...predicted.values()].sort(sort);
  expected.writtenFiles = expected.files.filter((f: any) => !before.files.some((v: any) => JSON.stringify(v) === JSON.stringify(f)));
  expected.directories = [...directories.values()].sort(sort);
  s.rustUpdate = { ...s.rustUpdate, tag, installerStdout: c.normalizeString(stdout), summary };
  return { before, onInstall, currentExe: payload(currentTag), installDir, stdout };
}
