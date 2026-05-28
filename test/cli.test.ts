import { describe, expect, test } from "bun:test";
import { formatProgressEvent, main, parseArgs } from "../src/cli.ts";

describe("cli progress formatter", () => {
  test("formats plain progress output without ANSI colors", () => {
    expect(formatProgressEvent(
      { status: "info", message: "reading config item demo-envswitch" },
      { color: false },
    )).toBe("envswitch [info] reading config item demo-envswitch");
  });

  test("formats colored status labels when color is enabled", () => {
    expect(formatProgressEvent(
      { status: "done", message: "found 5 candidate item(s)" },
      { color: true },
    )).toBe("\x1b[2menvswitch\x1b[0m \x1b[32m[done]\x1b[0m found 5 candidate item(s)");

    expect(formatProgressEvent(
      { status: "dry-run", message: "would write apps/api/.env" },
      { color: true },
    )).toBe("\x1b[2menvswitch\x1b[0m \x1b[33m[dry-run]\x1b[0m would write apps/api/.env");
  });

  test("rejects unknown flags before command execution", async () => {
    const result = await runCli(["switch", "--unknown"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --unknown"]);
  });

  test("rejects boolean flags passed with values", async () => {
    const result = await runCli(["switch", "--dry-run=false"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: --dry-run does not take a value"]);
  });

  test("uses command-specific flag allowlists", async () => {
    const result = await runCli(["list", "--client", "alpha"]);

    expect(result.code).toBe(1);
    expect(result.stderr).toEqual(["envswitch: unknown flag: --client"]);
  });

  test("parses omitted optional switch flags so defaults can apply", () => {
    expect(parseArgs(["switch", "--vault", "vault", "--config-item", "cfg", "--client", "alpha"])).toEqual({
      command: "switch",
      positional: [],
      flags: {
        vault: "vault",
        "config-item": "cfg",
        client: "alpha",
      },
    });
  });
});

async function runCli(argv: string[]): Promise<{ code: number; stderr: string[] }> {
  const originalError = console.error;
  const originalLog = console.log;
  const stderr: string[] = [];
  try {
    console.error = (...args: unknown[]) => {
      stderr.push(args.map(String).join(" "));
    };
    console.log = () => {};
    return {
      code: await main(argv),
      stderr,
    };
  } finally {
    console.error = originalError;
    console.log = originalLog;
  }
}
