import { mkdtemp, cp, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname, delimiter } from "node:path";
import { fileURLToPath } from "node:url";
export async function buildFake() {
  const dir = await mkdtemp(join(tmpdir(), "envswitch-fake-build-"));
  try {
    const repo = fileURLToPath(new URL("../", import.meta.url));
    for (const name of ["Cargo.toml", "Cargo.lock", "crates"]) await cp(join(repo, name), join(dir, name), { recursive: true, filter: source => !source.split(/[\\/]/).includes("target") });
    const home = join(dir, "home"); await mkdir(home);
    const locate = Bun.spawn(["rustup", "which", "cargo"], { stdout: "pipe", stderr: "inherit" });
    const cargo = (await new Response(locate.stdout).text()).trim();
    if (await locate.exited || !cargo) throw new Error("cannot locate installed Cargo toolchain");
    const env = { ...process.env, HOME: home, USERPROFILE: home, CARGO_HOME: process.env.ENVSWITCH_PARITY_CARGO_HOME ?? join(dir, "cargo-home"), RUSTUP_HOME: join(dir, "rustup-home"),
      XDG_CACHE_HOME: join(home, "cache"), XDG_CONFIG_HOME: join(home, "config"), XDG_DATA_HOME: join(home, "data"),
      LOCALAPPDATA: join(home, "local"), APPDATA: join(home, "roaming"), PATH: dirname(cargo) + delimiter + process.env.PATH };
    for (const key of Object.keys(env)) if (key.toUpperCase() === "PATH" && key !== "PATH") delete (env as Record<string, string | undefined>)[key];
    for (const args of [["build", "--locked", "--quiet"], ["test", "--locked", "--quiet"], ["fmt", "--check"], ["clippy", "--locked", "--quiet"]]) {
      const p = Bun.spawn([cargo, ...args, "--manifest-path", join(dir, "Cargo.toml"), "-p", "envswitch-fake-op", ...(args[0] === "clippy" ? ["--", "-D", "warnings"] : [])], { env, stdout: "inherit", stderr: "inherit" });
      if (await p.exited) throw new Error(`fake-op cargo ${args[0]} failed`);
    }
    return { binary: join(dir, "target", "debug", process.platform === "win32" ? "op.exe" : "op"), cleanup: () => rm(dir, { recursive: true, force: true }) };
  } catch (e) { await rm(dir, { recursive: true, force: true }); throw e; }
}
