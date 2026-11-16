// Generates crates/envswitch/src/switcher/collation_golden.rs from the TS
// oracle (src/switcher.ts at 53de029) running on Bun 1.4.0. Owner: C3.
//
// Run from a checkout whose src/ matches 53de029, then rustfmt the output:
//
//   LANG=en_US.UTF-8 bun crates/envswitch/tests/switcher_oracle/collation_golden.ts \
//     . crates/envswitch/src/switcher/collation_golden.rs && cargo fmt
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const repo = resolve(process.argv[2]!);
const outPath = process.argv[3]!;
const { parseConfig } = await import(join(repo, "src/config.ts"));
const { switchEnvironment } = await import(join(repo, "src/switcher.ts"));

// API order (deliberately not sorted). Index = item id `tNNN`.
const TITLES: string[] = [
  // 02 §4.3 probed list, scrambled
  "Zeta", "item2", "a.b", "_x", "B", "éclair", "x-1", "ab", "~t", "alpha", "-x", "item10", "a b",
  "1abc", "A", "ezra", "a", "b", "x_1",
  // case
  "abc", "ABC", "Abc", "aBc", "aBC",
  // envswitch-style titles (README recommends envswitch/<project>/<env>/<desc>)
  "envswitch/demo/local/api", "envswitch/demo/local/API", "envswitch/demo/local/api-2",
  "envswitch/demo/local/api_2", "envswitch/demo/local/api 2", "envswitch/demo/local/api.2",
  "envswitch/demo/local/api/2", "envswitch/demo/local/api2", "envswitch-demo-local-api",
  "envswitch_demo_local_api", "envswitch demo local api", "envswitch/Demo/local/api",
  "envswitch/demo/staging/api", "envswitch/demo/local/web",
  // locale-sensitive letters and accents
  "ä", "z", "ärlig", "zebra", "Äpfel", "apple", "Apple", "öl", "ol", "ø", "o",
  "å", "aa", "æ", "ae", "ß", "ss", "straße", "strasse", "Straße", "resume",
  "résumé", "Résumé", "resumé", "cote", "coté", "côte", "côté",
  "naïve", "naive", "ñ", "n", "nz", "ł", "l", "lz", "ı", "i", "İ", "I", "ç",
  "c", "cz", "ş", "s", "đ", "d", "þ", "th",
  // canonically equivalent pairs (must compare equal -> API order)
  "café", "café", "Ångström", "Ångström", "Ångström",
  "á̧", "á̧", "가", "가",
  // compatibility variants (tertiary differences)
  "ﬁle", "file", "ｆull", "full", "①", "1", "²", "2", "\u{1d400}", "Ａ",
  // ignorables and whitespace
  "a​b", "a­b", "a\u0001b", " leading", "a b", "a　b", "a\tb", "trailing ",
  "a b",
  // punctuation and symbols
  "(paren)", "[bracket]", "{brace}", "'quote", "\"dq", "!bang", "?q", "*star", "&amp", "+plus", "=eq",
  "<lt", ">gt", "|pipe", "$dollar", "%pct", "^caret", "`tick", "\\back", "/slash", ",comma", ";semi",
  ":colon", "@at", "#hash", ".dot", "…ellipsis", "–endash", "—emdash",
  "“curly”", "«guillemet»", "§section", "©copy", "€euro",
  "£pound", "¥yen",
  // digits
  "item1", "item01", "item 2", "item-1", "0", "00", "9", "10", "١٢", "३", "１",
  // non-Latin scripts (Han limited to titles where ICU4C and ICU4X agree; see report)
  "Москва", "москва", "Ёлка",
  "елка", "Елка", "Яблоко", "Жук",
  "Αθήνα", "αλφα", "Ωμέγα",
  "東京", "北京", "大阪", "中文", "上海",
  "あ", "ア", "か", "カ", "ｱ", "が",
  "한국", "서울", "가나",
  "مرحبا", "שלום", "ไทย", "เก",
  "กา", "हिन्दी", "Երևան",
  "თბილისი", "ሰላም",
  // emoji
  "\u{1f600}", "\u{1f600} smile", "\u{1f680}", "\u{1f44d}\u{1f3fd}", "\u{1f44d}", "❤️", "❤",
  // empty and equal titles
  "", "dup", "dup", "dup", "Dup",
];

// Pairs called out in the brief, plus a few more. Each is run through the
// oracle in both API orders.
const PAIRS: Array<[string, string]> = [
  ["_x", "-x"],
  ["a", "A"],
  ["item10", "item2"],
  ["ä", "z"],
  ["café", "café"],
  ["Ångström", "Ångström"],
  ["envswitch/demo/local/api", "envswitch-demo-local-api"],
  ["envswitch/demo/local/api", "envswitch/demo/local/API"],
  ["a.b", "a b"],
  ["~t", "_x"],
  ["1abc", "abc"],
  ["Äpfel", "apple"],
  ["résumé", "resume"],
  ["straße", "strasse"],
  ["ø", "o"],
  ["Москва", "москва"],
  ["Москва", "zebra"],
  ["東京", "大阪"],
  ["東京", "zebra"],
  ["가나", "あ"],
  ["\u{1f600}", "a"],
  ["a​b", "ab"],
  ["", "a"],
  ["dup", "dup"],
];

const config = parseConfig(JSON.stringify({
  project: { id: "demo" },
  targets: [{ name: "api", output: ".env" }],
  tags: { base: ["envswitch"] },
}));
const SHARED_TAGS = ["envswitch", "project:demo", "env:local", "layer:shared"];

async function runOracle(titles: Array<[string, string]>): Promise<{ layerIds: string[]; env: string }> {
  const root = await mkdtemp(join(tmpdir(), "envswitch-collation-"));
  try {
    const items = titles.map(([id, title]) => ({ id, title, tags: SHARED_TAGS }));
    const op = {
      listItemsByTags: async () => items,
      readEnvItem: async (item: { id: string }) => ({ COMMON: item.id }),
    };
    const results = await switchEnvironment(config, op, {
      root,
      env: "local",
      client: "alpha",
      targetName: "api",
      dryRun: false,
    });
    return {
      layerIds: results[0].layers.map((layer: { item: { id: string } }) => layer.item.id),
      env: await readFile(join(root, ".env"), "utf8"),
    };
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

const id = (i: number) => `t${String(i).padStart(3, "0")}`;

// 1. Equal-groups in ascending order, straight from localeCompare.
const order = TITLES.map((_, i) => i);
order.sort((a, b) => TITLES[a]!.localeCompare(TITLES[b]!));
const groups: number[][] = [];
for (const i of order) {
  const last = groups[groups.length - 1];
  if (last && TITLES[last[0]!]!.localeCompare(TITLES[i]!) === 0) last.push(i);
  else groups.push([i]);
}
// Sanity: the comparator is a consistent total preorder on this set.
const groupOf = new Map<number, number>();
groups.forEach((g, gi) => g.forEach((i) => groupOf.set(i, gi)));
for (let a = 0; a < TITLES.length; a++) {
  for (let b = 0; b < TITLES.length; b++) {
    const expected = Math.sign(groupOf.get(a)! - groupOf.get(b)!);
    const actual = Math.sign(TITLES[a]!.localeCompare(TITLES[b]!));
    if (expected !== actual) throw new Error(`inconsistent comparator at ${a},${b}`);
  }
}

// 2. Full merge through the oracle's switchEnvironment.
const full = await runOracle(TITLES.map((title, i) => [id(i), title]));
const flattened = groups.flatMap((g) => [...g].sort((x, y) => x - y)).map(id);
if (JSON.stringify(flattened) !== JSON.stringify(full.layerIds)) {
  throw new Error("oracle layer order differs from stable localeCompare order");
}

// 3. Pair winners in both API orders.
const pairResults: Array<{ left: string; right: string; leftFirst: string; rightFirst: string }> = [];
for (const [left, right] of PAIRS) {
  const lf = await runOracle([["L", left], ["R", right]]);
  const rf = await runOracle([["R", right], ["L", left]]);
  pairResults.push({
    left,
    right,
    leftFirst: lf.env.trim().replace("COMMON=", ""),
    rightFirst: rf.env.trim().replace("COMMON=", ""),
  });
}

// --- Rust emission ---------------------------------------------------------
function rustStr(s: string): string {
  let out = "";
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    if (ch === "\\") out += "\\\\";
    else if (ch === "\"") out += "\\\"";
    else if (cp >= 0x20 && cp < 0x7f) out += ch;
    else out += `\\u{${cp.toString(16)}}`;
  }
  return `"${out}"`;
}
function visible(s: string): string {
  // Comment text: printable as-is, controls and invisible format chars as U+XXXX.
  return [...s]
    .map((ch) => {
      const cp = ch.codePointAt(0)!;
      return cp < 0x20 || cp === 0x7f || cp === 0x2028 || cp === 0xad || cp === 0x200b ||
          (cp >= 0x300 && cp < 0x370) || cp === 0x3000 || cp === 0xa0
        ? `<U+${cp.toString(16).toUpperCase().padStart(4, "0")}>`
        : ch;
    })
    .join("");
}
const quoted = (s: string) => `"${visible(s)}"`;

const lines: string[] = [];
lines.push("//! Collation golden data generated from the TS oracle (D8, plan risk 1).");
lines.push("//!");
lines.push("//! Generated by running `src/switcher.ts` (commit `53de029`) on Bun 1.4.0");
lines.push("//! (`Intl.Collator().resolvedOptions()`: locale `en-US`, ICU 78.1, Unicode 17.0):");
lines.push("//!");
lines.push("//! ```text");
lines.push("//! LANG=en_US.UTF-8 bun crates/envswitch/tests/switcher_oracle/collation_golden.ts \\");
lines.push("//!   . crates/envswitch/src/switcher/collation_golden.rs && cargo fmt");
lines.push("//! ```");
lines.push("//!");
lines.push("//! The generator builds `GROUPS` with `a.localeCompare(b)` (and checks that");
lines.push("//! the comparator is a consistent total preorder on the set), then runs the");
lines.push("//! oracle's `switchEnvironment` (not dry-run) with one `layer:shared` item per");
lines.push("//! title, `id` = `tNNN` in `TITLES` order, values `{COMMON: id}`, and checks");
lines.push("//! that the oracle's layer order equals the stable sort implied by `GROUPS`.");
lines.push("//! `FULL_ENV` is the oracle's `.env` bytes. `PAIRS` repeats that for two");
lines.push("//! items (ids `L` and `R`) in both API orders and records the `COMMON` winner.");
lines.push("//!");
lines.push("//! Han titles are limited to ones where ICU4C and ICU4X agree; the known");
lines.push("//! Han divergence is pinned separately in `tests.rs`.");
lines.push("");
lines.push("/// Item titles in `op` API order; item `i` has id `t{i:03}`.");
lines.push("pub(super) const TITLES: &[&str] = &[");
TITLES.forEach((t, i) => {
  const escaped = rustStr(t) !== `"${t.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"`;
  lines.push(`    ${rustStr(t)}, // ${id(i)}${escaped ? ` ${quoted(t)}` : ""}`);
});
lines.push("];");
lines.push("");
lines.push("/// Indices into `TITLES`, grouped by `localeCompare(...) === 0`, groups in");
lines.push("/// ascending order. Within a group, indices are in API order.");
lines.push("pub(super) const GROUPS: &[&[usize]] = &[");
for (const g of groups) {
  const names = g.map((i) => quoted(TITLES[i]!)).join(" | ");
  lines.push(`    &[${g.join(", ")}], // ${names}`);
}
lines.push("];");
lines.push("");
lines.push("/// The oracle's `.env` for the full list: the last layer wins `COMMON`.");
lines.push(`pub(super) const FULL_ENV: &str = ${rustStr(full.env)};`);
lines.push("");
lines.push("/// `(left, right, winner with [L, R] API order, winner with [R, L] API order)`;");
lines.push("/// the left title belongs to item `L`, the right one to item `R`.");
lines.push("pub(super) const PAIRS: &[(&str, &str, &str, &str)] = &[");
for (const p of pairResults) {
  lines.push(`    (${rustStr(p.left)}, ${rustStr(p.right)}, ${rustStr(p.leftFirst)}, ${rustStr(p.rightFirst)}), // ${quoted(p.left)} vs ${quoted(p.right)}`);
}
lines.push("];");
lines.push("");
await Bun.write(outPath, lines.join("\n"));
console.log(`titles=${TITLES.length} groups=${groups.length} pairs=${pairResults.length}`);
