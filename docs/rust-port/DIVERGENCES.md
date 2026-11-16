# Approved divergences from the TS oracle

The oracle is commit `53de029` running on Bun 1.4.0. Any difference not listed here is a bug.

| # | Area | Example input | TS outcome | Rust outcome | Owner | Replacement assertion |
|---|---|---|---|---|---|---|
| a1 | Template placeholders that name JS prototype members | tag template `x:{constructor}` | renders JS function source | renders `""` (same as any unknown placeholder) | C1 | Golden expects `x:` |
| a2 | 1Password field labeled `__proto__` | env item with label `__proto__` | dropped silently | kept as a normal env var | C2 | Golden expects the key present |
| a3 | Flag `--__proto__ x` | `switch --__proto__ x` | silently dropped | parsed as unknown flag `__proto__` → `unknown flag: --__proto__` | C4 | Exit 1 with that message |
| a4 | Integer-like flag names order | `--1 a --vault v` | integer keys are validated first | insertion order | C4 | Only the first-error order differs; asserted by insertion order |
| a5 | JSON parse error suffix | malformed config JSON | `... is not valid JSON: <engine text>` | same prefix + serde_json text | C1 | Match the prefix up to `: ` |
| a6 | Spawn failure when `op` is missing | `op` not on PATH | `Executable not found in $PATH: "op"` (Bun) | `failed to run op: <io error>` | C2 | Exit 2 + prefix `envswitch: ` |
| a7 | Lone UTF-16 surrogate escapes in `op` JSON | item field value `"\ud800"` | JS keeps the lone surrogate; written as U+FFFD | serde_json rejects → `op item get output is not valid JSON: …`, exit 2 | C2 | Scenario marked `a7`; assert the prefix and exit 2 |
| a8 | JSON accepted by JS but rejected by serde_json (config item, package.json) | lone surrogate escapes, numbers outside f64 range (`1e400`), nesting deeper than 128 | parsed (Infinity, lone surrogate kept) | `<source> is not valid JSON: <serde text>` (same prefix as a5), exit per TS parse-error path | C1 | Prefix assertion |
| b1 | Ctrl-C / Ctrl-Break on Windows outside prompts | Ctrl-C or Ctrl-Break during `op` call | process killed (0xC000013A) | exit 130 via handler | C4 | Windows-only test where possible |
| c1 | Prompt render stream | guided mode | prompts drawn on stdout | prompts drawn on stderr (`inquire`) | C4 | Prompt log is compared through the `Prompter` trait, not the stream |
| c2 | Prompt navigation | arrow keys at list ends; typing | no wraparound (`loop: false`); prefix jump | `inquire` behavior: wraps at list ends; substring filter | C4 | PTY smoke only |
| c3 | Esc key in prompts | Esc | ignored | cancel → exit 130, nothing printed | C4 | Scripted prompter returns Cancelled → 130 |
| d1 | Filesystem error text | `ENOTDIR` during the lstat walk | `ENOTDIR: not a directory, lstat '<path>'` | Node format reproduced for ENOENT, ENOTDIR, EACCES, EPERM, EEXIST, EISDIR; other codes `<io error>` | C1, C2 | Exact for listed codes, prefix match otherwise |
| e1 | `update` current version | installed binary | reads `current/.version` | compiled version (`CARGO_PKG_VERSION`) | D3 | Replacement for 01 §9 rows 16–22 |
| e2 | `update` installer source | — | bundled `current/scripts/install.*` must exist | embedded installer (`include_str!`) | D3 | Missing bundled script is no longer an error |
| e3 | `update` install locations | custom `INSTALL_DIR` | not passed (launcher recreated in default dir) | read from `install.json` receipt and passed | D3 | Receipt values forwarded to the installer |
| e4 | `update` on a copied binary with no receipt | — | n/a | reinstall guidance, exit 1 | D3 | Message + exit 1 |
| e5 | Installer output lines | install/update | Bun/source lines | native lines per `INSTALLER-PROTOCOL.md` | D2b | Installer test suites |
| f1 | `--version` | `envswitch --version` | `unknown command: --version`, exit 1 | `envswitch <ver> <target>\n`, exit 0 | C4 | Snapshot |
| g1 | Same-layer title ordering | (1) same-rank titles `ä…`/`z…` on a Swedish-locale host; (2) titles differing only in Han ideographs, e.g. `密钥` vs `密鑰`; (3) characters newer than the oracle's ICU data (Unicode 17+) | host-locale `localeCompare`; Bun/ICU 78 orders Han by radical-stroke | fixed `en-US` ICU4X collation (tertiary, no numeric); Han ideographs in code-point order; newer characters per ICU4X data | C3 | Goldens generated with oracle in en-US match exactly except Han/new-code-point cases, which are pinned by `known_divergence_han_ideographs_use_code_point_order` |
| h1 | Guided vault-scan skip messages | two unreadable vaults scanned concurrently | emitted in completion order (nondeterministic with real `op`; vault order with synchronous fakes) | always emitted in vault order | C4 | Goldens (fakes) match exactly; no extra assertion |
| i1 | Windows drive-relative output whose drive letter case differs from the root | output `c:x`, root `C:\Proj` | symlink-check loop never terminates (hang) | loop stops at the root (case-insensitive) and accepts `c:\Proj\x` | C1 | Windows test asserts acceptance |

Windows drive-relative paths (`C:x`) keep **parity**; no tightening.
