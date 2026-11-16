#!/usr/bin/env bun
export type Difference = { path: string; expected: unknown; actual: unknown };
export function differences(expected: any, actual: any, path = "$", result: Difference[] = []): Difference[] {
  if (Object.is(expected, actual)) return result;
  if (expected && actual && typeof expected === "object" && typeof actual === "object" && Array.isArray(expected) === Array.isArray(actual)) {
    if (!Array.isArray(expected) && JSON.stringify(Object.keys(expected)) !== JSON.stringify(Object.keys(actual))) result.push({ path: `${path}.[key order]`, expected: Object.keys(expected), actual: Object.keys(actual) });
    for (const key of [...new Set([...Object.keys(expected), ...Object.keys(actual)])].sort()) differences(expected[key], actual[key], `${path}.${key}`, result);
  } else result.push({ path, expected, actual });
  return result;
}
export function compare(expected: any, actual: any, label = expected.id): boolean {
  const diff = differences(expected, actual);
  if (expected.divergence) console.error(`DIVERGENCE ${expected.divergence}: ${label}${diff.length ? " (replacement assertion required for Rust)" : " (TS baseline reproduced)"}`);
  for (const d of diff) {
    const show = (value: any) => {
      if (/\.(stdout|stderr)Base64$/.test(d.path) && typeof value === "string") return JSON.stringify(Buffer.from(value, "base64").toString("utf8"));
      return JSON.stringify(value) ?? "<missing>";
    };
    console.error(`${label} ${d.path}\n  expected: ${show(d.expected)}\n  actual:   ${show(d.actual)}`);
  }
  return diff.length === 0;
}
if (import.meta.main) {
  const [expected, actual] = process.argv.slice(2);
  if (!expected || !actual) throw new Error("usage: bun parity/diff/compare.ts <expected.json> <actual.json>");
  process.exit(compare(await Bun.file(expected).json(), await Bun.file(actual).json()) ? 0 : 1);
}
