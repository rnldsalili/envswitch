import { mkdtemp, cp, appendFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
export async function buildFake() {
  const dir = await mkdtemp(join(tmpdir(), "envswitch-fake-build-"));
  try {
    const crate = join(dir, "fake-op");
    await cp(fileURLToPath(new URL("../crates/fake-op", import.meta.url)), crate, { recursive: true, filter: source => !source.split(/[\\/]/).includes("target") });
    // Temporary copy only: never modify the shared workspace manifest or lockfile.
    await appendFile(join(crate, "Cargo.toml"), "\n[workspace]\n");
    const home = join(dir, "home");
    await (await import("node:fs/promises")).mkdir(home);
    const locate = Bun.spawn(["rustup", "which", "cargo"], { stdout: "pipe", stderr: "inherit" });
    const cargo = (await new Response(locate.stdout).text()).trim();
    if (await locate.exited || !cargo) throw new Error("cannot locate installed Cargo toolchain");
    const binDir = (await import("node:path")).dirname(cargo);
    const env = { ...process.env, HOME: home, USERPROFILE: home, CARGO_HOME: join(dir, "cargo-home"), RUSTUP_HOME: join(dir, "rustup-home"),
      XDG_CACHE_HOME: join(home, "cache"), XDG_CONFIG_HOME: join(home, "config"), XDG_DATA_HOME: join(home, "data"),
      LOCALAPPDATA: join(home, "local"), APPDATA: join(home, "roaming"), PATH: binDir + (process.platform === "win32" ? ";" : ":") + process.env.PATH };
    for (const key of Object.keys(env)) if (key.toUpperCase() === "PATH" && key !== "PATH") delete (env as Record<string, string | undefined>)[key];
    for (const args of [["build", "--quiet"], ["test", "--quiet"], ["fmt", "--check"], ["clippy", "--quiet"]]) {
      const p = Bun.spawn([cargo, ...args, "--manifest-path", join(crate, "Cargo.toml"), ...(args[0] === "clippy" ? ["--", "-D", "warnings"] : [])], { env, stdout: "inherit", stderr: "inherit" });
      if (await p.exited) throw new Error(`fake-op cargo ${args[0]} failed`);
    }
    return { binary: join(crate, "target", "debug", process.platform === "win32" ? "op.exe" : "op"), cleanup: () => rm(dir, { recursive: true, force: true }) };
  } catch (e) { await rm(dir, { recursive: true, force: true }); throw e; }
}
