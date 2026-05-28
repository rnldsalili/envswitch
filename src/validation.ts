import { fail } from "./errors.ts";

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseJson(raw: string, source: string): unknown {
  try {
    return JSON.parse(raw);
  } catch (error) {
    fail(`${source} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export function requireRecord(value: unknown, message: string): Record<string, unknown> {
  if (!isRecord(value)) {
    fail(message);
  }
  return value;
}
