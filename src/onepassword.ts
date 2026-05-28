import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fail } from "./errors.ts";
import { isRecord, parseJson } from "./validation.ts";
import type { OnePasswordItemSummary } from "./types.ts";

const execFileAsync = promisify(execFile);

export interface CommandRunner {
  run(command: string, args: string[]): Promise<string>;
}

export class ExecCommandRunner implements CommandRunner {
  async run(command: string, args: string[]): Promise<string> {
    try {
      const { stdout } = await execFileAsync(command, args, {
        maxBuffer: 50 * 1024 * 1024,
      });
      return stdout;
    } catch (error) {
      const maybeError = error as { stderr?: string; stdout?: string; message?: string };
      const output = [maybeError.stderr, maybeError.stdout, maybeError.message]
        .filter(Boolean)
        .join("\n")
        .trim();
      throw new Error(output || `${command} ${args.join(" ")} failed`);
    }
  }
}

export class OnePasswordClient {
  constructor(
    private readonly vault: string,
    private readonly runner: CommandRunner = new ExecCommandRunner(),
  ) {}

  async assertAuthenticated(): Promise<void> {
    await this.runner.run("op", ["whoami", "--format", "json"]);
  }

  async readConfigItem(item: string): Promise<string> {
    const detail = await this.getRawItem(item);
    const field = detail.fields.find((candidate) => candidate.label === "config");
    if (!field || typeof field.value !== "string" || field.value.trim() === "") {
      fail(`1Password config item ${item} must contain a non-empty field named config`);
    }
    return field.value;
  }

  async listItemsByTags(tags: string[]): Promise<OnePasswordItemSummary[]> {
    const args = ["item", "list", "--vault", this.vault, "--format", "json"];
    if (tags.length > 0) {
      args.push("--tags", tags.join(","));
    }
    const raw = await this.runner.run("op", args);
    const parsed = parseJson(raw, "op item list output");
    if (!Array.isArray(parsed)) {
      fail("op item list returned non-array JSON");
    }

    return parsed.map((item, index) => parseItemSummary(item, index));
  }

  async readEnvItem(item: OnePasswordItemSummary): Promise<Record<string, string>> {
    const detail = await this.getRawItem(item.id || item.title);
    const values: Record<string, string> = {};
    for (const field of detail.fields) {
      const label = field.label.trim();
      if (label === "" || label === "notesPlain") {
        continue;
      }
      if (!isEnvKey(label)) {
        continue;
      }
      values[label] = field.value;
    }
    return values;
  }

  private async getRawItem(item: string): Promise<RawItem> {
    const raw = await this.runner.run("op", [
      "item",
      "get",
      item,
      "--vault",
      this.vault,
      "--format",
      "json",
      "--reveal",
    ]);
    const parsed = parseJson(raw, `op item get ${item} output`);
    if (!isRecord(parsed) || !Array.isArray(parsed.fields)) {
      fail(`op item get ${item} returned unexpected JSON`);
    }
    return {
      fields: parsed.fields.map((field, index) => parseField(field, index)),
    };
  }
}

interface RawItem {
  fields: Array<{ label: string; value: string }>;
}

function parseItemSummary(value: unknown, index: number): OnePasswordItemSummary {
  if (!isRecord(value)) {
    fail(`op item list result[${index}] must be an object`);
  }
  const id = typeof value.id === "string" ? value.id : "";
  const title = typeof value.title === "string" ? value.title : "";
  const tags = Array.isArray(value.tags)
    ? value.tags.filter((tag): tag is string => typeof tag === "string")
    : [];
  if (id === "" && title === "") {
    fail(`op item list result[${index}] must include id or title`);
  }
  return { id, title, tags };
}

function parseField(value: unknown, index: number): { label: string; value: string } {
  if (!isRecord(value)) {
    fail(`op item field[${index}] must be an object`);
  }
  return {
    label: typeof value.label === "string" ? value.label : "",
    value: typeof value.value === "string" ? value.value : "",
  };
}

function isEnvKey(value: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(value);
}
