import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { stringifyEnv, writeEnvFile } from "../src/dotenv.ts";

describe("dotenv", () => {
  test("stringifies dotenv output deterministically", () => {
    expect(stringifyEnv({ B: "two words", A: "1", EMPTY: "" })).toBe('A=1\nB="two words"\nEMPTY=\n');
  });

  test("writes env files atomically with private permissions", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-dotenv-"));
    try {
      const output = join(root, "nested/.env");

      await writeEnvFile(output, { TOKEN: "secret", NAME: "demo app" });

      expect(await readFile(output, "utf8")).toBe('NAME="demo app"\nTOKEN=secret\n');
      expect((await stat(output)).mode & 0o777).toBe(0o600);
      expect((await readdir(join(root, "nested"))).sort()).toEqual([".env"]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("does not replace an existing env file when serialization fails", async () => {
    const root = await mkdtemp(join(tmpdir(), "envswitch-dotenv-"));
    try {
      const output = join(root, ".env");
      await writeFile(output, "EXISTING=1\n", { mode: 0o600 });

      await expect(writeEnvFile(output, { "BAD-KEY": "nope" })).rejects.toThrow("invalid environment variable key");

      expect(await readFile(output, "utf8")).toBe("EXISTING=1\n");
      expect((await readdir(root)).sort()).toEqual([".env"]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
