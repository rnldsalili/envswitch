import { randomUUID } from "node:crypto";
import { mkdir, writeFile, chmod, rename, rm } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

const ENV_KEY_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function stringifyEnv(values: Record<string, string>): string {
  const lines: string[] = [];
  for (const key of Object.keys(values).sort()) {
    if (!ENV_KEY_RE.test(key)) {
      throw new Error(`invalid environment variable key: ${key}`);
    }
    lines.push(`${key}=${formatValue(values[key] ?? "")}`);
  }
  return `${lines.join("\n")}\n`;
}

export async function writeEnvFile(path: string, values: Record<string, string>): Promise<void> {
  const content = stringifyEnv(values);
  const dir = dirname(path);
  const tempPath = join(dir, `.${basename(path)}.${process.pid}.${randomUUID()}.tmp`);

  await mkdir(dir, { recursive: true, mode: 0o750 });
  try {
    await writeFile(tempPath, content, { encoding: "utf8", mode: 0o600 });
    await chmod(tempPath, 0o600);
    await rename(tempPath, path);
  } catch (error) {
    await rm(tempPath, { force: true });
    throw error;
  }
  await chmod(path, 0o600);
}

function formatValue(value: string): string {
  if (value === "") {
    return "";
  }
  if (/[\n\r]/.test(value)) {
    return JSON.stringify(value);
  }
  if (/\s|[#"'`$\\]/.test(value)) {
    return JSON.stringify(value);
  }
  return value;
}
