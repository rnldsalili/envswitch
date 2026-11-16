# 02 — Core Logic Behavioral Contract (TypeScript → Rust)

Scope: `src/config.ts`, `src/switcher.ts`, `src/onepassword.ts`, `src/dotenv.ts`, `src/validation.ts`, `src/errors.ts`, `src/types.ts` and their tests. `src/cli.ts` is out of scope except where it consumes this logic.

Conventions:
- `file:line` refers to the TypeScript sources at commit `53de029`.
- "Verbatim" messages use `${x}` for interpolation.
- "Probed" means confirmed by running the real TS code under Bun 1.4.0 / macOS arm64 with fake runners (no `op` calls). Probe scripts were kept in the session scratchpad, not in the repo.

---

## 0. Error model

| Kind | Source | CLI exit code (`cli.ts:182-184`) |
|---|---|---|
| `EnvswitchError` (`errors.ts:1-10`, thrown via `fail()`) | All validation in config/onepassword/validation | `1` |
| Plain `Error` | `ExecCommandRunner` failures (`onepassword.ts:23`), `stringifyEnv` invalid key (`dotenv.ts:11`), all `fs` errors (mkdir/writeFile/chmod/rename/lstat non-ENOENT) | `2` |

CLI prints `envswitch: ${message}` to stderr. **Parity requirement:** the Rust error type must keep the "user error vs other error" distinction, because it controls the exit code. Notably, **every `op` failure (not signed in, missing binary, non-zero exit) is a plain Error → exit 2**, not 1.

Proposed Rust:
```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")] User(String),            // EnvswitchError → exit 1
    #[error("{0}")] Command(String),         // runner failure → exit 2
    #[error("{0}")] InvalidEnvKey(String),   // dotenv.ts:11 → exit 2
    #[error(transparent)] Io(#[from] std::io::Error), // → exit 2
}
```

---

## 1. Config schema and validation

### 1.1 Entry point and order of checks

`parseConfig(raw)` (`config.ts:27-64`). Checks run in exactly this order; the **first** failure wins:

1. `JSON.parse(raw)`; on failure `config field is not valid JSON: ${engineMessage}` (`config.ts:28`, `validation.ts:7-13`).
2. Root must be a non-null, non-array object → `config JSON must be an object` (`config.ts:28`, `validation.ts:15-20`).
3. `targets` must be an array with length > 0 → `config.targets must be a non-empty array` (`config.ts:31-33`). Missing, `null`, `{}` and `[]` all hit this.
4. Each target, in index order (`config.ts:36`, `parseTarget` `244-270`).
5. `project` (`config.ts:38`, `parseProject` `186-218`).
6. `clients` (`config.ts:39`).
7. `environments` (`config.ts:40`).
8. `tags` (`config.ts:41`, `parseTags` `272-296`).
9. Duplicate target names (`config.ts:55-61`).

Probed: `{"targets":[{"name":"a","output":"x"},{"name":"a b","output":"y"}],"project":3}` reports `target a b contains unsupported characters` (targets before project).

### 1.2 Fields

| Path | Type | Required | Default when absent | Validation (`file:line`) |
|---|---|---|---|---|
| `project` | object | no | `undefined` (no project tag; `{project}` falls back to `basename(process.cwd())`) | must be object (`config.ts:190-192`) |
| `project.id` | string | no | `undefined` | must be string (`195-197`); if **truthy**, must match identifier regex (`198-200`). `""` is accepted, stored, never validated, and behaves as absent (see §9). |
| `project.rootMarkers` | array of objects | no | `undefined` (= no checks) | must be array (`203-206`); `[]` accepted |
| `project.rootMarkers[i].path` | string | one of path/packageJsonName | — | if present: string and `trim() !== ""` (`226-231`) |
| `project.rootMarkers[i].packageJsonName` | string | one of path/packageJsonName | — | if present: string and `trim() !== ""` (`232-237`) |
| `clients` | string[] | no | `undefined` (any client allowed) | `parseStringArray(..., required=false)` (`298-316`). Items are **not** identifier-validated at parse time. |
| `environments` | string[] | no | `undefined` (any env allowed) | same as `clients` |
| `targets` | object[] (len ≥ 1) | **yes** | — | see 1.1 step 3 |
| `targets[i].name` | string | yes | — | string, `trim() !== ""` (`248-250`), then identifier regex (`255`) |
| `targets[i].output` | string | yes | — | string, `trim() !== ""` (`251-253`). The path is not checked at parse time; it is checked later in `resolveOutputPath`. Stored untrimmed. |
| `targets[i].groups` | string[] | no | `undefined` | `parseStringArray` (`261`), then each item must match the identifier regex (`262-267`). `[]` is accepted and stored. Duplicates are allowed. |
| `tags` | object | no | `undefined` | must be object (`276-278`) |
| `tags.base` | string[] | no | `[]` (`DEFAULT_TAGS`, `15-23`) | `parseStringArray(..., required=true)`, but only called when `base !== undefined`, so the "is required" branch is **unreachable** (`281-286`) |
| `tags.project` | string | no | `"project:{project}"` | non-empty after trim (`287-294`) |
| `tags.env` | string | no | `"env:{env}"` | same |
| `tags.client` | string | no | `"client:{client}"` | same |
| `tags.target` | string | no | `"target:{target}"` | same |
| `tags.group` | string | no | `"group:{group}"` | same |
| `tags.layer` | string | no | `"layer:{layer}"` | same |

Identifier regex (`config.ts:25`): `^[A-Za-z0-9][A-Za-z0-9_-]*$` (ASCII only; JS `$` = end of input, so a trailing `\n` fails). Same semantics as Rust `regex` `^[A-Za-z0-9][A-Za-z0-9_-]*$`, or hand-code the ASCII check.

**Unknown keys** are silently ignored at every level: root, `project`, `rootMarkers[i]`, `targets[i]` and `tags`. Templates are not checked for placeholders.

**`null` is not "absent".** Every optional check uses `!== undefined`, so JSON `null` reaches the type check and fails. Probed:

| Input | Message |
|---|---|
| `"project": null` | `config.project must be an object` |
| `"clients": null` | `config.clients must be an array` |
| `"tags": null` | `config.tags must be an object` |
| `"tags": {"base": null}` | `config.tags.base must be an array` |
| `"targets":[{..., "groups": null}]` | `config.targets[0].groups must be an array` |
| `"project": {"id": null}` | `config.project.id must be a string` |
| `rootMarkers[0].path: null` | `config.project.rootMarkers[0].path must be a non-empty string` |
| `"tags": {"env": null}` | `config.tags.env must be a non-empty string` |

→ Rust must **not** use `#[derive(Deserialize)]` with `Option<T>` (serde maps `null` to `None`). Parse into `serde_json::Value` and validate by hand, keeping key presence separate from `Value::Null`.

**Duplicate JSON keys:** last one wins (probed). `serde_json::Value` objects also keep the last value. Parity holds.

### 1.3 Verbatim messages (config)

| Message | Line |
|---|---|
| `config field is not valid JSON: ${err}` | `config.ts:28` via `validation.ts:11` |
| `config JSON must be an object` | `config.ts:28` |
| `config.targets must be a non-empty array` | `32` |
| `config.targets contains duplicate target ${name}` | `58` |
| `config.project must be an object` | `191` |
| `config.project.id must be a string` | `196` |
| `project id contains unsupported characters` | `199` → `320` |
| `config.project.rootMarkers must be an array` | `205` |
| `config.project.rootMarkers[${i}] must be an object` | `222` |
| `config.project.rootMarkers[${i}].path must be a non-empty string` | `228` |
| `config.project.rootMarkers[${i}].packageJsonName must be a non-empty string` | `234` |
| `config.project.rootMarkers[${i}] must define path or packageJsonName` | `239` (reachable only when both are absent) |
| `config.targets[${i}] must be an object` | `246` |
| `config.targets[${i}].name must be a non-empty string` | `249` |
| `config.targets[${i}].output must be a non-empty string` | `252` |
| `target ${name} contains unsupported characters` | `255` |
| `target ${name} group ${group} contains unsupported characters` | `264` |
| `config.tags must be an object` | `277` |
| `config.tags.${key} must be a non-empty string` (key ∈ project, env, client, target, group, layer, checked in that order) | `290` |
| `${name} must be an array` (name ∈ `config.clients`, `config.environments`, `config.targets[${i}].groups`, `config.tags.base`) | `306` |
| `${name}[${idx}] must be a non-empty string` | `311` |
| `${name} is required` | `301` (unreachable) |
| `environment contains unsupported characters` | `74` (note: the value is **not** included) |
| `client contains unsupported characters` | `75` |
| `environment ${env} is not listed in config.environments` | `78` |
| `client ${client} is not listed in config.clients` | `82` |
| `target contains unsupported characters` | `91` |
| `target ${name} is not defined in config.targets` | `94` |
| `project root marker path does not exist: ${marker.path}` | `328` |
| `project root marker packageJsonName requires package.json in the current directory` | `335` |
| `failed to read package.json for project root validation: ${err}` | `341` |
| `current directory package.json name does not match ${marker.packageJsonName}` | `344` |

The JSON parser's message (`${err}`) depends on the engine. Probed Bun: `JSON Parse error: Expected '}'`, `JSON Parse error: Unexpected EOF`. **The Rust port cannot match this byte for byte.** Use `serde_json::Error`'s `Display` and match only the prefix in tests, which is all the TS tests check.

### 1.4 `trim()` semantics (affects emptiness checks and env labels)

JS `String.prototype.trim` strips: `\t \n \v \f \r`, space, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, **U+FEFF**. It does **not** strip **U+0085**.

Rust `str::trim` uses Unicode White_Space. It strips U+0085 and does not strip U+FEFF. Probed: output `"\uFEFF"` is rejected as empty, and output `"\u0085"` is accepted. → Add a `js_trim()` / `js_is_space(char)` helper, and use it for all `trim()` call sites (`config.ts:107, 227, 233, 248, 251, 289, 310`; `onepassword.ts:51, 75`) and for `\s` in `dotenv.ts:42`.

### 1.5 Selection (`config.ts:73-97`)

- `validateSelection(config, env, client)`: identifier check on env, then client. Then, only if `environments` is defined **and non-empty**, env must be included (`===`). The same applies to `clients`. An empty list means "allow any".
- `selectTargets(config, name)`: the literal `"all"` returns every target in config order, **without** an identifier check. Otherwise the name is identifier-checked and the first exact match is returned. A target literally named `"all"` can never be selected alone (see §9).

### 1.6 Root markers (`config.ts:66-71, 324-347`)

- `validateProjectRoot(config, root)` checks markers in array order. All must pass (AND). No markers means nothing to check, and the root's existence is never checked.
- For a marker with both fields, both are checked: `path` first, then `packageJsonName`.
- `path` marker: `resolveOutputPath(root, marker.path)` (§2). It inherits the absolute, escape and symlinked-parent rejection, **with target-flavored messages** (for example, marker `"."` → `target output path escapes the project root: .`). Then `stat` (follows symlinks): any error, including EACCES or a dangling link, counts as "missing" → `project root marker path does not exist: ${path}`. The marker path may itself be a symlink because only its parents are lstat-checked.
- `packageJsonName` marker: `join(root, "package.json")` (`join`, not `resolve`). `stat` fails → "requires package.json" message. Read as UTF-8 and parse JSON; a read or parse error → "failed to read package.json…: ${err}". If the result is not an object, or `name !== marker.packageJsonName` (strict, untrimmed, case-sensitive) → "does not match". BOM handling: `readFile(..., "utf8")` **keeps** a leading BOM, and `JSON.parse` then fails. Rust `serde_json::from_str` also fails on a BOM. Parity holds.

---

## 2. Path safety — `resolveOutputPath(root, output)` (`config.ts:106-135`)

### 2.1 Exact algorithm (POSIX Node semantics)

1. If `output.trim() === ""` → `target output path cannot be empty` (`107-109`). This is not reachable for targets (the parser already rejects it), and the same holds for markers.
2. If `path.isAbsolute(output)` → `target output path must be relative to the project root: ${output}` (`110-112`). On POSIX, that means `output` starts with `/`.
3. `rootAbs = path.resolve(root)`: lexical. It prepends `process.cwd()` when `root` is relative, collapses `.`/`..`/`//`, strips trailing `/`, and does **not** resolve symlinks.
4. `outputAbs = path.resolve(rootAbs, path.normalize(output))`: lexical join plus normalization. `..` collapses lexically **before** any filesystem check (`linked/../x` → `x`, even when `linked` is a symlink).
5. `rel = path.relative(rootAbs, outputAbs)`. Reject when `rel === ""` **or** `rel.startsWith("..")` **or** `isAbsolute(rel)` → `target output path escapes the project root: ${output}` (`116-119`).
6. `rejectSymlinkedParents(rootAbs, outputAbs, output)` (`125-135`):
   ```
   current = dirname(outputAbs)
   loop:
     st = lstatSync(current, {throwIfNoEntry:false})   // ENOENT → undefined; any OTHER errno THROWS raw fs Error
     if st?.isSymbolicLink() → fail("target output path contains a symbolic-link parent: ${output}")
     if current === rootAbs → return
     current = dirname(current)
   ```
   - It checks every ancestor from the output's parent **up to and including `rootAbs` itself**. If the project root is itself a symlink, the call fails. Ancestors above the root are not checked; for example, macOS `/var` → `/private/var` is fine.
   - The output file itself is **not** lstat-checked. A symlinked `.env` is later replaced by `rename` and is not followed.
   - Missing directories are fine (`undefined`).
   - A non-directory parent makes `lstat` fail with **ENOTDIR**. That error is raw, not an `EnvswitchError`, so the exit code is 2. Probed message: `ENOTDIR: not a directory, lstat '<abs>/afile/sub'`.
   - The loop ends because step 5 guarantees `outputAbs` is lexically below `rootAbs`, so the string equality is reached.
7. Return `outputAbs`. The original casing is kept, there is no trailing slash, and the path is not canonicalized.

Probed (root `/tmp/nonexistent-root-xyz`):

| `output` | Result |
|---|---|
| `apps/api/.env` | `/tmp/nonexistent-root-xyz/apps/api/.env` |
| `./.env` | `<root>/.env` |
| `apps//api/./.env` | `<root>/apps/api/.env` |
| `apps/api/` | `<root>/apps/api` (trailing slash dropped; it will be written as a **file** named `api`) |
| `.`, `./`, `a/..` | escapes (rel `""`) |
| `a/../../x` | escapes |
| `..env`, `..foo/.env` | **escapes (false positive, §9)** |
| `/abs` | must be relative |
| `~/.env` | `<root>/~/.env` (no tilde expansion) |
| `\\server\share\x`, `C:\x`, `a\..\..\x` | accepted on POSIX as literal file names containing backslashes |
| relative root `relroot` | resolved against cwd |

### 2.2 Re-check before writing

`switchEnvironment` resolves every selected target before any `op` call (`switcher.ts:79`). Each target is resolved **again** right before it is written (`switcher.ts:103`). This catches a parent that was swapped for a symlink while `op` was running (test "rechecks output parents…"). There is still a race window between that lstat walk and `mkdir`/`rename` (comment at `config.ts:126`). This is accepted; do not "fix" it with `openat` unless that is a deliberate decision.

### 2.3 Node `path` vs Rust `std::path` differences and the proposed Rust approach

| Concern | Node | Rust `std::path` | Rust approach |
|---|---|---|---|
| Absolute detection (Windows) | `win32.isAbsolute`: `/foo`, `\foo`, `C:\x`, `\\srv\sh`, `//srv/sh`, `\\?\C:\x` → true; `C:x`, `C:` → false (probed) | `Path::is_absolute()` is **false** for `\foo` and `/foo` (root without prefix) | Reject when `p.has_root() \|\| matches!(p.components().next(), Some(Component::Prefix(_)))`. This also rejects `C:x` (Node accepts `C:x` when it is on the root's drive: `C:\Proj\x`). Document this as a deliberate Windows-only tightening. |
| Separators | `win32.normalize` turns `/` into `\` | Windows treats both as separators; POSIX treats `\` as a normal character | Use `Path::components()`. POSIX keeps backslashes as literal (parity). |
| `..`/`.` collapse | `resolve`/`normalize` are lexical | `components()` drops `.` but **keeps `..`**. `std::path::absolute` keeps `..` on POSIX. `canonicalize` follows symlinks and needs existence. | Write a lexical normalizer: start from `root_abs` components; for each output component, `Normal` → push, `CurDir` → skip, `ParentDir` → pop (and an escape if it pops past the root length). Do **not** use `canonicalize`. Crates like `path-clean` work, but a hand-written version is about 20 lines and fully controlled. |
| Escape test | `rel.startsWith("..")` (string) | n/a | For parity, replicate: compute `rel` as the joined remaining components and reject when it is empty or its **string** starts with `..`. That keeps the `..env` false positive (§9). Alternatively, fix it and document the divergence. Decide explicitly. |
| Case sensitivity | `win32.relative` compares case-insensitively (probed `relative("C:\\Proj","c:\\proj\\x") = "x"`) | Component equality is case-sensitive | Not observable, because `outputAbs` is built by appending to `rootAbs`, so the prefix is byte-identical. Containment by construction avoids the problem. |
| Trailing slash | stripped by `resolve` | `components()` ignores it | Parity holds. |
| UNC / verbatim `\\?\` | absolute → rejected | `Prefix` component | Rejected by the rule above. |
| Root = drive root `C:\` | `dirname("C:\\") === "C:\\"` | `parent()` of `C:\` is `None` | Loop on `current == root_abs` and stop when `parent()` is `None` (defensive). |
| Reserved device names (`con`, `nul`) | accepted (probed `C:\Proj\con`) | accepted | Parity holds (flag only). |
| Symlink detection | Node `lstat().isSymbolicLink()` is true for symlinks **and junctions** on Windows (the tests create a `"junction"` on win32) | `symlink_metadata()?.file_type().is_symlink()`: on Windows true for name-surrogate reparse points (symlinks and junctions) | Use `std::fs::symlink_metadata`. Map `NotFound` → continue; other errors → `Error::Io`, which keeps exit code 2 for parity. **Verify junction behavior on Windows CI.** |
| Relative `root` | `resolve` uses `process.cwd()` | — | `std::env::current_dir()?.join(root)` followed by the lexical normalizer. |

---

## 3. Tag templates (`config.ts:137-184, 349-351`)

### 3.1 `renderTemplate(template, ctx)` (`config.ts:349-351`)

- Regex `\{([A-Za-z0-9_-]+)\}`, global, **single pass**. Substituted values are not rescanned.
- `ctx` keys: `project, env, client, target, group, layer`. Unknown placeholders render as `""`.
- No escape mechanism. `{ env }` (with spaces) is left literal. `{{env}}` → `{local}` (probed).
- **Bug:** `ctx` is a plain object, so `ctx[key] ?? ""` reads prototype members. `{constructor}` → `function Object() { [native code] }` and `{__proto__}` → `[object Object]` (probed). See §9.

### 3.2 `tagValues(config, params)` (`config.ts:137-175`)

- `templates = {...DEFAULT_TAGS, ...config.tags}`. Keys absent from `config.tags` keep their defaults. Because parsing never stores `undefined`, there is no undefined-overrides-default problem.
- `ctx.project = config.project?.id ?? basename(process.cwd())`. **It uses `process.cwd()`, not the `root` option** (§9). If `project.id` is `""`, `??` keeps `""`, so `{project}` renders as `""`.
- Every missing `params.*` is `""` in `ctx`.
- Output:
  - `base`: render every base template, then drop empty strings (`.filter(Boolean)`). Order is kept, and duplicates are **kept** at this point.
  - `project`: rendered only if `config.project?.id` is **truthy**, otherwise `undefined`.
  - `env`/`client`/`target`/`group`/`layer`: rendered only if the matching param is truthy, otherwise `undefined`. A rendered result can be `""`, for example when the template is `"{client}"` but only `env` was passed. Callers mostly drop empty values through `compactUnique`, but `layerTags` (§4.1) drops them by truthiness.

### 3.3 `compactUnique(values)` (`config.ts:182-184`)

Drops `undefined` and `""`, then removes duplicates while keeping the **first** occurrence. Rust: an iterator with a `HashSet<&str>` "seen" set and a `Vec` result. **Do not use `BTreeSet`**, because order matters for the `--tags` argument.

### 3.4 `candidateListTags(config, env)` (`config.ts:177-180`)

`compactUnique([...base, project, env])`, rendered with `{env}` only. For the test config this is `["envswitch","project:demo","env:local"]`. It is passed to `op item list --tags` joined with `,`, and it is also the required "common tags" set in classification (`switcher.ts:168`, identical construction).

### 3.5 Scope patterns (`switcher.ts:282-286`)

For scope ∈ {target, client, group}: render the scope template with that scope's value set to `__ENVSWITCH_SCOPE__` and every other context value `""` (except `project`). Escape regex metacharacters `[.*+?^${}()|[\]\\]`, replace **every** sentinel with `.+`, and anchor as `^…$`. With no flags, JS `.` excludes all four JS line terminators: `\r`, `\n`, U+2028, U+2029. **Erratum (plan review 04 §8):** Rust `.` excludes only `\n`, so `.+` is wrong (a tag `client:foreign\r` is *not* a client scope in TS). Rust: replace each sentinel with `[^\r\n\u{2028}\u{2029}]+`, i.e. `regex::Regex::new(&format!("^{}$", regex::escape(&rendered).replace(SENTINEL, r"[^\r\n\x{2028}\x{2029}]+")))`. `regex::escape` escapes more characters, but the literal-match semantics are the same, and the sentinel is pure `[A-Z_]`, so escaping leaves it untouched.

Edge cases:
- If the template has no `{scope}` placeholder (for example `client: "client"`), the pattern is the literal `^client$`.
- A bare `{client}` template gives `^.+$`, which matches every tag, so `hasAnyClient` is always true and no `shared`/`target` item ever classifies (probed). See §9.

---

## 4. Layer classification and merge (`switcher.ts`)

### 4.1 Matcher (`buildTagMatcher`, `switcher.ts:144-177`)

- `LAYER_ORDER = ["shared","group","target","client","client-target"]` (`26`). Rank = index + 1, so shared = 1 … client-target = 5.
- `layerTags[layer] = tagValues({layer}).layer`, set only if it is truthy (`147-152`). With the default template, `layer:shared`, etc.
- `commonTags = compactUnique([...base, project, env])` (`168`).
- For each **configured** target: `currentTargetTags = {targetTag(name)}`, and `currentGroupTags = {groupTag(g) for g in target.groups}` (`156-165`).
- `currentClientTag = tagValues({client}).client` (`172`).
- The matcher is built once per `switchEnvironment` and shared by all targets (`83`).

### 4.2 `classifyItemWithMatcher(item, targetName, m)` (`switcher.ts:179-215`)

`tags = Set(item.tags ?? [])`.
1. If any `commonTags` entry is missing → `undefined` (`185-189`).
2. Flags:
   - `hasAnyTarget/Client/Group`: some tag matches the scope pattern. **This includes the current target, client or group.**
   - `hasCurrentTarget`: the item has this target's tag. An unknown target name gives an empty set, so the flag is false.
   - `hasCurrentGroup`: the item has **any** tag of this target's groups.
   - `hasCurrentClient`: `currentClientTag` is truthy and present.
3. Branches are tested **in order**, and the first match wins:

| # | Result | Condition (`line`) |
|---|---|---|
| 1 | `shared` | `layer:shared` ∧ ¬anyTarget ∧ ¬anyClient ∧ ¬anyGroup (`198`) |
| 2 | `group` | `layer:group` ∧ currentGroup ∧ ¬anyTarget ∧ ¬anyClient (`201`) |
| 3 | `target` | `layer:target` ∧ currentTarget ∧ ¬anyClient ∧ ¬anyGroup (`204`) |
| 4 | `client` | `layer:client` ∧ currentClient ∧ ¬anyTarget (`207`). **Group tags are allowed and ignored.** |
| 5 | `client-target` | `layer:client-target` ∧ currentClient ∧ currentGroup ∧ ¬anyTarget (`210`) |
| — | `undefined` | otherwise (`214`) |

Consequences (probed unless noted):
- No `layer:` tag → `undefined`. An unknown `layer:foo` → `undefined`, ignored silently with no warning.
- Several layer tags: the first passing branch wins. For example, `layer:shared` + `layer:client` + `client:alpha` → `client`.
- Extra **non-scope** tags (for example `misc`) are allowed (→ `shared`).
- A `group` item with an extra foreign group tag (`group:apis` + `group:workers`) is still `group` for `api`.
- A `target` item with several target tags (`target:api` + `target:web`) applies to each listed target.
- A `client` item with several client tags (`client:alpha` + `client:beta`) applies to each.
- A `client` item that also has `group:apis` applies to **every** target, including `web` (README wording agrees; looks like a pitfall).
- A `client-target` item with any `target:*` tag → `undefined` (`alpha-target-web` test).
- The "group" in "client-target" is a **group**, not a target (README §Tag Model item 5).

`classifyItemForTarget(config, item, {env, client, target})` (`136-142`) is the public test seam. It rebuilds the matcher on every call.

### 4.3 `mergeTarget` (`switcher.ts:217-266`)

1. Iterate `candidates` in **`op item list` API order** (`223`). For each classified item:
   - `key = item.id || item.title` (`297-299`).
   - On a cache miss: emit `info "reading ${item.title || item.id}"` and store the `readEnvItem(item)` promise (`230-233`). The cache is **shared across targets** in one switch (`84`), so each item is read at most once per run.
   - Push `{name, rank, item, values: await …}` (`234-239`).
   - **Reads are sequential.** Each one is awaited before the next candidate, so there is no concurrency.
   - A rejected read is cached, but the error propagates immediately and aborts the whole switch.
2. Sort layers (`242-247`): by `rank` ascending, then `left.item.title.localeCompare(right.item.title)`. `Array.prototype.sort` is **stable**, so ties (same rank, same title at tertiary strength) keep API order. The **id is not used** as a tie-breaker.
3. Merge (`249-259`): apply layers in sorted order, and later layers overwrite earlier ones. A key is added to `overwritten` only if it already existed **and the new value differs** (`!==`). Re-setting an identical value is not reported.
4. `overwrittenKeys = Array.from(set).sort()`, using UTF-16 code-unit order. Keys are ASCII (`isEnvKey`), so byte order is equivalent.
5. Duplicate items (the same id twice in the list) become two layers with the same values: read once, nothing reported as overwritten.

**`localeCompare` parity risk.** The order uses ICU collation (en-US under Bun, tertiary strength, no numeric collation). Probed order:
```
_x, -x, ~t, 1abc, a, A, a b, a.b, ab, alpha, b, B, éclair, ezra, item10, item2, x_1, x-1, Zeta
```
(`"a".localeCompare("A") === -1`, lowercase first; punctuation before digits before letters; `_` before `-`; accents are secondary.) The result also depends on the default locale, because the TS code passes no locale. Rust: use `icu_collator` (ICU4X) with the `und`/`en` locale and default options, and add a golden test with the list above. Byte-wise `cmp` would **break parity**: titles are mixed-case and use `/` and `-` (the README recommends `envswitch/<project>/<env>/<desc>`).

Observable effects of the ordering: (a) which value wins within one rank, (b) the order of `layers` in output (`cli.ts:420`), and (c) `overwrittenKeys` membership.

### 4.4 Result

`TargetSwitchResult` (`switcher.ts:60-67`): `{ target, outputPath (first resolution), written: !dryRun, keyCount: number of merged keys, layers (sorted), overwrittenKeys }`. The CLI uses `layer.name`, `layer.item.title`, `keyCount`, `overwrittenKeys` and `parentExists` (`cli.ts:329, 420-425`). It never prints `layer.values`.

### 4.5 `switchEnvironment` control flow (`switcher.ts:69-122`)

```
log info "validating project root ${root}"
validateProjectRoot          (§1.6)
validateSelection            (§1.5)
selectTargets                (§1.5)
resolveTargets               (§2; resolves every selected target, so an invalid path fails before any op call)
log info "listing candidate 1Password items"
op.listItemsByTags(candidateListTags(config, env))
log done "found ${candidates.length} candidate item(s)"     // count before classification
buildTagMatcher
for each resolved target, in config order (or the single selected target):
  log info "processing ${name} -> ${outputPath}"
  mergeTarget (emits "reading …" per cache miss)
  if !dryRun: outputPath2 = resolveOutputPath(root, output)   // re-check
              log info "writing ${outputPath2}"; writeEnvFile; log done "wrote ${outputPath2}"
  else:       log dry-run "would write ${outputPath}"
```

Targets are written **one at a time**. If target 2 fails, target 1's file is already written (not transactional; §9). The function never writes to stdout or stderr itself (the test checks that `console.error` is not called).

### 4.6 `inspectProject(config, root)` (`switcher.ts:124-134`)

It emits no events and makes no `op` calls. It runs `validateProjectRoot`, then `resolveTargets` for **all** config targets, and returns `{target, outputPath, parentExists: existsSync(dirname(outputPath))}`. `existsSync` follows symlinks and returns false on any error. Rust: `Path::exists()` (follows symlinks, errors → false). Parity holds.

---

## 5. 1Password interaction (`onepassword.ts`)

### 5.1 Process execution (`ExecCommandRunner`, `onepassword.ts:13-26`)

- `execFile(command, args, { maxBuffer: 50 * 1024 * 1024 })`: there is **no shell**, **no timeout**, the environment is inherited (so `OP_ACCOUNT`, `OP_SERVICE_ACCOUNT_TOKEN`, `OP_SESSION_*` and so on pass through), the working directory is inherited, stdout is decoded as UTF-8, and stdin is a pipe (not a TTY).
- The program is always `"op"`, resolved through `PATH`.
- **No `--account` flag is ever passed.**
- On success the code returns the **raw stdout string** and ignores stderr.
- On failure (spawn error, non-zero exit, or maxBuffer overflow), the message is `stderr.trim()`, else `error.message.trim()`, else `${command} ${args.join(" ")} failed`. The error is a plain `Error`, so the exit code is 2. **Stdout is never included** (security requirement; test "does not include failed command stdout").
- Probed messages:
  - Non-zero exit with stderr `"  err msg  \n"` → `err msg`.
  - Non-zero exit with empty stderr → `Command failed: <cmd> <args…>`. This is the Bun/Node `error.message`, and it includes the item id and vault name but no secrets.
  - Binary missing (Bun) → `Executable not found in $PATH: "<cmd>"`. Node would give `spawn op ENOENT`.

  → The "missing" and "empty stderr" texts are runtime-specific, so **exact parity is impossible**. Proposed Rust: `io::ErrorKind::NotFound` → `Executable not found in $PATH: "op"`, to match current Bun. Non-zero exit with empty stderr → `Command failed: op <args joined by space>`.
- maxBuffer: Rust `Command::output()` has no limit. To match, read stdout with a 50 MiB cap and fail with the message `stdout maxBuffer length exceeded`. Alternatively, treat this as a non-goal and document it.
- No error-specific mapping exists. "Not signed in" (and similar) shows `op`'s own stderr verbatim. `assertAuthenticated` ignores whoami stdout.
- Rust: synchronous `std::process::Command::new("op").args(..).stdin(Stdio::null()).output()`. No async runtime is needed because everything is sequential. On Windows, `Command` finds `op.exe` through `PATH`, which matches `execFile`.

### 5.2 Invocations (exact argv after `op`)

| Method | argv | Line |
|---|---|---|
| `assertAuthenticated()` | `whoami --format json` | `35` |
| `listVaults()` (static) | `vault list --format json` | `39` |
| `listItemsByTags(tags)` | `item list --vault <vault> --format json` + (`--tags <tags.join(",")>` only if `tags.length > 0`) | `58-61` |
| `readConfigItem(item)` / `readEnvItem(item)` | `item get <item> --vault <vault> --format json --reveal` | `88-97` |

Probed argv: `["item","list","--vault","V","--format","json","--tags","a,b"]`; `["item","get","My Title","--vault","V","--format","json","--reveal"]`.

Notes:
- `<item>` for env items is `item.id || item.title` (`72`). For config items it is the raw user or guided-mode string. **There is no `--` separator**, so an item name starting with `-` would be read as a flag (§9).
- A tag containing `,` would be split by `op` (templates allow commas; §9).
- Whether `op item list --tags a,b` uses AND or OR is **not verified** here (`op` was not called). It does not affect correctness, because classification enforces every common tag locally. *(Plan review 04 §12: not a blocking question; a nonexistent-tag probe could not distinguish AND from OR anyway.)*
- What `op item list` prints when there are no matches (`[]` or empty stdout) is **not verified**. Empty stdout would surface as `op item list output is not valid JSON: …`. Check this against a real `op` before declaring parity.

### 5.3 Output parsing

| Method | Parse rules | Messages |
|---|---|---|
| `listVaults` (`38-46`, `127-137`) | Must be a JSON array. Each element must be an object. `id`/`name` are strings, else `""`. Fails if both are `""`. Returns `name: name \|\| id`. Probed: `{id:"x"}` → `{id:"x",name:"x"}`. | `op vault list output is not valid JSON: ${e}`; `op vault list returned non-array JSON`; `op vault list result[${i}] must be an object`; `op vault list result[${i}] must include id or name` |
| `listItemsByTags` (`57-69`, `112-125`) | Must be an array. Elements must be objects. `id`/`title` are strings, else `""`. `tags` is an array filtered to string elements, else `[]` (probed `["a",1,null,"b"]` → `["a","b"]`; `"x"` → `[]`). Fails if both `id` and `title` are `""`. Other fields are ignored. **Order is preserved** (it drives merge order). | `op item list output is not valid JSON: ${e}`; `op item list returned non-array JSON`; `op item list result[${i}] must be an object`; `op item list result[${i}] must include id or title` |
| `getRawItem` (`87-105`, `139-147`) | Must be an object with an array `fields`. Each field must be an object. `label` and `value` are strings, else `""`. **Every other field property is ignored** (`id`, `type`, `purpose`, `section`, `reference`, `totp`, …). | `op item get ${item} output is not valid JSON: ${e}`; `op item get ${item} returned unexpected JSON`; `op item field[${i}] must be an object` |

`parseJson` runs `JSON.parse` on the whole stdout, so leading or trailing whitespace is fine. serde_json rejects lone-surrogate escapes such as `"\ud800"`, which JS accepts. This edge case is negligible; flag it only.

### 5.4 Fields → env vars (`readEnvItem`, `onepassword.ts:71-85`)

For each field in `fields` order:
1. `label = field.label.trim()` (JS trim, §1.4).
2. Skip if `label === ""` or `label === "notesPlain"`. The comparison uses the trimmed label and is case-sensitive.
3. Skip if it does not match `^[A-Za-z_][A-Za-z0-9_]*$` (`149-151`). For example `invalid-key` or `9KEY`.
4. `values[label] = field.value`. The value is **not trimmed**. A missing or non-string value becomes `""` (probed `{label:"NOVAL"}` → `NOVAL=""`; `value: 5` → `""`).

Consequences:
- **Duplicate labels: the last one wins**, including the same label in different sections. Sections are ignored.
- Field type does not matter. CONCEALED values are revealed because of `--reveal`. Login items' built-in `username` and `password` fields **do** become env vars (probed). An OTP field gives its `value` (the `otpauth://` URI), not the current code.
- **Labels are used, never field ids.**
- **A label of `__proto__` is silently dropped** (JS prototype setter; probed `Object.keys` excludes it). Rust would keep it unless this is replicated (§9).

`readConfigItem(item)` (`48-55`) picks the **first** field whose `label === "config"`, compared exactly and **untrimmed** (unlike env labels; probed: `" config"` is skipped). It fails if the field is missing, the value is not a string, or `value.trim() === ""` → `1Password config item ${item} must contain a non-empty field named config`. It returns the value **untrimmed** (probed `"  {\"a\":1}  "`).

### 5.5 Concurrency

None. Every `op` call is awaited one after another. The item cache prevents re-reads across targets within one run.

---

## 6. Dotenv writing (`dotenv.ts`)

### 6.1 `stringifyEnv(values)` (`dotenv.ts:7-16`)

- Keys are sorted with the default `sort()` (UTF-16 code units). That equals byte order for ASCII and differs from merge order.
- Keys are validated **during iteration in sorted order**, so the first invalid key in sorted order throws a plain `Error`: `invalid environment variable key: ${key}` (probed: `{"B-1","A-1"}` → reports `A-1`). Regex: `^[A-Za-z_][A-Za-z0-9_]*$`.
- Line format: `${key}=${formatValue(value)}`. Lines are joined with `\n` and a final `\n` is **always** appended. An empty map produces `"\n"` (one newline; probed, and the file gets written).
- `formatValue` (`35-46`):
  1. `""` → `` (empty, so `KEY=`).
  2. Contains `\n` or `\r` → `JSON.stringify(value)`.
  3. Matches JS `\s` (set in §1.4: includes U+FEFF and U+00A0, excludes U+0085) or any of `` # " ' ` $ \ `` → `JSON.stringify(value)`.
  4. Otherwise the raw value.
- `JSON.stringify` of a string: wrap in `"`; escape `"` → `\"`, `\` → `\\`, `\b \f \n \r \t`; other U+0000–U+001F → `\u00xx` (**lowercase** hex). Everything else is raw (non-ASCII, U+007F, U+2028/2029). `serde_json::to_string(&str)` produces identical output for valid UTF-8.
- **Control characters without whitespace are not quoted**: `"\u0001ctl"` → `K=\u0001ctl` (a raw control byte) and `"\u0000"` → a raw NUL (probed). `x\u000b` is quoted as `"x\u000b"` because `\v` ∈ `\s`. `=` is not quoted (`a=b` → `K=a=b`). A single quote alone triggers double-quoting with the `'` left unescaped (`"s'x"`).

Golden cases (probed, all `stringifyEnv({K: v})`):

| v | output |
|---|---|
| `""` | `K=\n` |
| `plain` | `K=plain\n` |
| `two words` | `K="two words"\n` |
| `a=b` | `K=a=b\n` |
| `q"x` | `K="q\"x"\n` |
| `s'x` | `K="s'x"\n` |
| `` b`x `` | ``K="b`x"\n`` |
| `$HOME` | `K="$HOME"\n` |
| `back\slash` | `K="back\\slash"\n` |
| `hash#` | `K="hash#"\n` |
| `tab<TAB>` | `K="tab\t"\n` |
| `nl<LF>x` | `K="nl\nx"\n` |
| `cr<CR>x` | `K="cr\rx"\n` |
| `<U+0001>ctl` | `K=<U+0001>ctl\n` |
| `nbsp<U+00A0>` | `K="nbsp<U+00A0>"\n` |
| `nel<U+0085>` | `K=nel<U+0085>\n` (unquoted) |
| `bom<U+FEFF>` | `K="bom<U+FEFF>"\n` |
| `ls<U+2028>` | `K="ls<U+2028>"\n` (raw inside quotes) |
| `é` | `K=é\n` |
| `<U+007F>` | `K=<U+007F>\n` |
| `x<U+000B>` | `K="x\u000b"\n` |
| `<U+0000>` | `K=<U+0000>\n` |

### 6.2 `writeEnvFile(path, values)` (`dotenv.ts:18-33`)

1. `content = stringifyEnv(values)` **first**. A serialization error happens before any filesystem change (test "does not replace…").
2. `dir = dirname(path)`; `tempPath = join(dir, ".${basename(path)}.${process.pid}.${randomUUID()}.tmp")`. Example: `.env` → `..env.1274.a40ec953-52da-410e-8d73-208f633a3a00.tmp` (probed shape; two leading dots for `.env`).
3. `mkdir(dir, {recursive: true, mode: 0o750})`. The mode is subject to umask and applies only to directories it creates; existing directories are untouched (probed: umask 077 → `700`, umask 000 → `750`). This step is **outside** the try block.
4. try:
   - `writeFile(tempPath, content, {encoding: "utf8", mode: 0o600})`. The flag is `w` (create + truncate, **no `O_EXCL`**), and the mode is subject to umask at creation.
   - `chmod(tempPath, 0o600)`.
   - `rename(tempPath, path)`. This is an atomic replace on POSIX. It does not follow a symlink at `path`; the link itself is replaced. If `path` is a directory, it fails with EISDIR (probed: the temp file is removed and the error is rethrown).
5. catch: `rm(tempPath, {force: true})`, then rethrow the original error (raw fs Error → exit 2).
6. `chmod(path, 0o600)`. This is **outside** the try; if it fails, the new file stays in place.
- **No fsync** of the file or the directory.
- The file content is UTF-8 with no BOM and `\n` line endings on every OS.

Rust:
- `fs::create_dir_all(dir)`, then on unix `DirBuilder::new().recursive(true).mode(0o750)`.
- Create the temp file with `OpenOptions::new().write(true).create(true).truncate(true).mode(0o600)` on unix. `create_new(true)` is stricter and harmless.
- Write, `set_permissions(0o600)`, `fs::rename`, cleanup on error, then `set_permissions(path, 0o600)`.
- Keep the temp name pattern (crates `uuid` v4 plus `std::process::id()`), so leftovers look the same.
- `tempfile::Builder::new().prefix(&format!(".{base}.{pid}.")).suffix(".tmp").tempfile_in(dir)` + `persist(path)` is an acceptable alternative (random suffix instead of a UUID, `O_EXCL`, 0600). The name differs only in its random part.

### 6.3 Windows behavior

- Node `fs.rename` uses `MoveFileExW(REPLACE_EXISTING)`, which replaces an existing file but fails with EPERM/EBUSY if the target is open elsewhere. Rust `fs::rename` also replaces existing files on Windows. Parity holds.
- `chmod` on Windows only toggles the read-only attribute; 0o600 has the write bit set, so it is effectively a no-op. In Rust, gate the mode calls behind `#[cfg(unix)]`.
- The dotenv test asserts `mode & 0o777 === 0o600`, which is POSIX-only. Gate that assertion with `#[cfg(unix)]`.

---

## 7. Progress events

`ProgressEvent = {status: "info"|"done"|"dry-run", message}` (`types.ts:66-71`). They are emitted only when a `log` callback is provided.

`switchEnvironment` order (`switcher.ts:74-108`; exact sequence asserted by the test "dry run logs progress…"):

| # | status | message | When |
|---|---|---|---|
| 1 | info | `validating project root ${root}` (`root` exactly as passed, not resolved) | start |
| 2 | info | `listing candidate 1Password items` | after selection and path validation |
| 3 | done | `found ${n} candidate item(s)` | n = raw list length |
| per target: |
| 4 | info | `processing ${name} -> ${outputPath}` | |
| 5* | info | `reading ${title \|\| id}` | per **cache miss**, in candidate (API) order, not sorted order |
| 6a | info | `writing ${outputPath}` | not dry-run (after the path re-check) |
| 7a | done | `wrote ${outputPath}` | not dry-run |
| 6b | dry-run | `would write ${outputPath}` | dry-run |

`inspectProject` emits nothing. `cli.ts` emits its own `reading config item …`/`loaded config item …` events (`cli.ts:436-438`); those are out of scope.

Rust: pass `Option<&mut dyn FnMut(ProgressEvent)>` (or `&dyn Fn`) with `enum Status { Info, Done, DryRun }`. Keep the strings identical.

---

## 8. Test inventory and Rust seams

How the TS tests fake things:
- **switcher tests** inject a duck-typed object `{listItemsByTags, readEnvItem}`, typed as `Pick<OnePasswordClient, …>` (`switcher.ts:31`), with an in-memory `items` list (16 fixtures, `switcher.test.ts:20-101`) and a `values` map (`103-120`).
- **onepassword tests** inject a `CommandRunner` (`StaticRunner`, `onepassword.test.ts:58-73`). It answers by argv shape: `vault list` → `responses["vault list"] ?? "[]"`; `item list` → `responses["item list"] ?? "[]"`; `item get X` → `responses["item get X"] ?? {"fields":[]}`; anything else → `"{}"`.
- The real `ExecCommandRunner` is tested once, by spawning the Bun executable (`process.execPath -e …`).
- File-system tests use `mkdtemp(tmpdir())` and clean up in `finally`.
- **No real `op` is ever called. There are no fixture files and no golden files**; every expectation is inline.

### 8.1 Proposed Rust seams

```rust
pub trait CommandRunner { fn run(&self, cmd: &str, args: &[String]) -> Result<String, Error>; }
pub struct ExecCommandRunner;                          // real
pub struct OnePasswordClient<R: CommandRunner> { vault: String, runner: R }
pub trait EnvSource {                                  // = Pick<OnePasswordClient, "listItemsByTags"|"readEnvItem">
    fn list_items_by_tags(&self, tags: &[String]) -> Result<Vec<ItemSummary>, Error>;
    fn read_env_item(&self, item: &ItemSummary) -> Result<EnvMap, Error>;
}
```

- `switch_environment<S: EnvSource>`; tests use `FakeEnvSource { items, values, read_counts: RefCell<HashMap<..>>, on_read: Option<Box<dyn Fn()>> }`. The `on_read` hook is needed for the symlink-swap test.
- Fixtures: put the 16 switcher items and values in `tests/fixtures/switcher_items.json` and `switcher_values.json`, or keep them inline in a `support` module.
- Temp directories: `tempfile::TempDir`.
- Symlinks: `std::os::unix::fs::symlink`; on Windows, a junction (crate `junction`) to mirror the TS test.
- Golden files: `tests/golden/web.env` (the merged output) and the §6.1 table.

### 8.2 Test → behavior → seam

| # | File / test name | Asserts | Fake used | Rust seam |
|---|---|---|---|---|
| 1 | config / parses a valid generic config | `project.id`, `targets[0].name`, `groups`, `tags.group` round-trip | pure | unit test on `parse_config(&str)` |
| 2 | config / rejects invalid target groups | `groups:["all apis"]` → contains `unsupported characters` | pure | unit; assert full message `target api group all apis contains unsupported characters` |
| 3 | config / rejects malformed config fields with specific messages | `"{"` → `config field is not valid JSON`; `[]` → `config JSON must be an object`; `clients:"alpha"` → `config.clients must be an array`; `id:"bad project"` → `project id contains unsupported characters` | pure | unit; JSON message checked by prefix only |
| 4 | config / rejects duplicate targets | contains `duplicate target` | pure | unit |
| 5 | config / resolves target output inside project root | `("/tmp/project","apps/api/.env")` → `/tmp/project/apps/api/.env` (the root need not exist) | pure plus lstat on nonexistent paths | unit, `#[cfg(unix)]`; on Windows use a temp root |
| 6 | config / rejects output paths that escape the project root | `../outside/.env` → `escapes`; `/tmp/outside/.env` → `relative` | pure | unit; add Windows cases `\foo`, `C:\x`, `\\srv\sh\x` under `cfg(windows)` |
| 7 | config / rejects output paths with symlinked parents | `root/linked → outside` → `symbolic-link parent`; nothing written outside | real FS, `mkdtemp`, `symlink` (`junction` on win32) | `TempDir` plus `symlink`/`junction` |
| 8 | config / selects all targets or one named target | `"all"` → 2; `"web"` → `["web"]`; `"worker"` → `not defined` | pure | unit |
| 9 | config / validates directory and package root markers | `{path:"services"}` + `{packageJsonName:"demo-suite"}` pass | real FS (dir plus `package.json`) | `TempDir`; **add** negative cases for each marker message (currently untested) |
| 10 | switcher / classifies only items that belong to the selected target | 18 classification assertions over the 16 fixture items (§4.2) | pure | table-driven unit test over the fixture items |
| 11 | switcher / recognizes unknown scopes with custom tag templates | custom `scope/client/{client}` etc.: foreign client/target/group tags on a `layer:shared` item → `undefined` | pure | unit (covers the regex-escape path) |
| 12 | switcher / dry run merges supported layers without writing files | api target: `keyCount 6`; layers `[shared, api-group, api-target, alpha, alpha-api]`; `overwrittenKeys ["COMMON"]`; no file; **no `console.error`** | inline fake op | `FakeEnvSource` plus `TempDir`; assert the library never prints (inject the logger) |
| 13 | switcher / dry run logs progress when a logger is provided | exact 10-event sequence (§7) | inline fake op | `FakeEnvSource`; collect into a `Vec<ProgressEvent>` |
| 14 | switcher / writes merged values in group-aware layer order | web target: layers `[shared, app-group, web-target, alpha, alpha-web]`; file content `APP_GROUP=1\nCLIENT_ONLY=1\nCOMMON=alpha-web\nSHARED=1\nWEB_CLIENT_ONLY=1\nWEB_ONLY=1\n` | inline fake op plus real FS | `FakeEnvSource` plus `TempDir` plus golden `web.env` |
| 15 | switcher / caches 1Password item reads across targets in one switch | `targetName:"all"` → results `[api, web]`; each of 8 classified items read exactly once | counting fake | `FakeEnvSource` with `RefCell<HashMap<String, usize>>` |
| 16 | switcher / rechecks output parents immediately before writing | `readEnvItem` swaps `apps/api` for a symlink; switch rejects with `symbolic-link parent`; nothing written outside | fake op with an FS side effect | `FakeEnvSource.on_read` closure; `TempDir`; symlink/junction |
| 17 | onepassword / lists vault summaries | `[{id,name}]` round-trip | `StaticRunner` | `StaticRunner: CommandRunner` (port the TS class one-to-one) |
| 18 | onepassword / rejects non-array item list output | `"{}"` → `op item list returned non-array JSON` | `StaticRunner` | same |
| 19 | onepassword / rejects invalid item JSON | `"not-json"` → `op item list output is not valid JSON` (prefix) | `StaticRunner` | same |
| 20 | onepassword / requires a non-empty config field | `config:""` → `must contain a non-empty field named config` | `StaticRunner` | same |
| 21 | onepassword / reads only valid env field labels | `VALID_KEY` kept; `invalid-key` and `notesPlain` dropped | `StaticRunner` | same; **add** cases for duplicates, trim, missing value, `__proto__` |
| 22 | onepassword / does not include failed command stdout in errors | the real runner with a child writing `SUPER_SECRET` to stdout and `safe failure` to stderr, exit 1 → the message contains stderr but not stdout | **real process** (`process.execPath -e`) | `ExecCommandRunner` with `sh -c 'printf SUPER_SECRET; printf "safe failure" >&2; exit 1'` (unix) / `cmd /C` (windows), or the self-re-exec pattern (the test binary checks an env var and acts as the child) |
| 23 | dotenv / stringifies dotenv output deterministically | `{B:"two words",A:"1",EMPTY:""}` → `A=1\nB="two words"\nEMPTY=\n` | pure | unit plus the §6.1 golden table |
| 24 | dotenv / writes env files atomically with private permissions | `nested/.env` created; content `NAME="demo app"\nTOKEN=secret\n`; mode `0600`; directory contains only `.env` (no temp leftover) | real FS | `TempDir`; mode assertion `#[cfg(unix)]` |
| 25 | dotenv / does not replace an existing env file when serialization fails | `{"BAD-KEY":…}` → `invalid environment variable key`; existing file unchanged; no temp leftover | real FS | `TempDir` |

Coverage gaps worth adding in Rust: tests for every marker failure message, `null` handling, `..env`, ENOTDIR, empty merged values (`"\n"`), `localeCompare` ordering, `__proto__`, `client` items with group tags, and a bare `{client}` template.

---

## 9. Recommended Rust layout, crates, and flagged behaviors

### 9.1 Modules

```
src/
  lib.rs          // re-exports; no printing
  error.rs        // Error enum (User vs Command/Io/InvalidEnvKey) + exit-code mapping
  jsutil.rs       // js_trim, js_is_space (\s set), identifier/env-key checks
  config/
    mod.rs        // parse_config (serde_json::Value → Config), validate_selection, select_targets
    model.rs      // Config, Project, RootMarker, Target, TagTemplates (+ defaults)
    paths.rs      // resolve_output_path, lexical normalize, reject_symlinked_parents
    markers.rs    // validate_project_root
    tags.rs       // render_template, tag_values, candidate_list_tags, compact_unique
  op/
    runner.rs     // CommandRunner trait, ExecCommandRunner
    client.rs     // OnePasswordClient (list_vaults, list_items_by_tags, read_env_item, read_config_item, assert_authenticated)
    parse.rs      // lenient item/vault/field parsing from Value
  switcher/
    matcher.rs    // TagMatcher, scope patterns, classify
    merge.rs      // merge_target, collation sort, overwritten keys
    mod.rs        // switch_environment, inspect_project, ProgressEvent
  dotenv.rs       // stringify_env, write_env_file
```

### 9.2 Crates

| Crate | Why |
|---|---|
| `serde_json` | Parses into `Value`, so the code can tell `null` from a missing key and produce the exact TS messages. Its string escaping equals `JSON.stringify`. |
| `serde` | Only for internal structs, if needed. Config parsing must stay manual. |
| `thiserror` | An error enum that keeps the user-vs-other split behind exit codes 1 and 2. |
| `regex` | Scope tag patterns (`^…$` with `.+`), via `regex::escape`. A hand-written prefix/suffix matcher is an alternative, but templates may hold several sentinels. |
| `icu_collator` (ICU4X) | Matches `String.prototype.localeCompare` for title tie-breaking. `feruca` (pure UCA) is a lighter alternative; check it against the golden list. |
| `uuid` (v4) | Reproduces the temp file name `.<base>.<pid>.<uuid>.tmp`. |
| `tempfile` (dev; optionally prod) | `TempDir` in tests; `Builder` for atomic temp files if the name parity is waived. |
| `junction` (dev, Windows) | Creates junctions in the symlink tests, like the TS tests do on win32. |
| none for async | All I/O is sequential. Use `std::process::Command`; skip tokio. |

### 9.3 Ambiguities and latent bugs (flagged, **not** fixed)

1. **`..`-prefixed names are rejected as escapes** (`config.ts:117`, `rel.startsWith("..")`). `..env` and `..foo/.env` fail with "escapes the project root". Decide: keep this behavior (safe) or fix it with a component check.
2. **Template prototype leak** (`config.ts:350`): `{constructor}`, `{toString}`, `{__proto__}`, etc. render JS internals. Recommend rendering `""` (the documented "unknown placeholder" behavior) and noting the divergence. No real config should depend on this.
3. **`{project}` falls back to `basename(process.cwd())`, not `root`** (`config.ts:156`). The two match in the CLI (`root = process.cwd()`, `cli.ts:467`), but the core API mixes the two sources.
4. **`project.id: ""` is accepted** (`config.ts:198, 211`). It skips validation and silently drops the project tag, so items from any project match.
5. **The `__proto__` env label is silently dropped** (`onepassword.ts:82`, `switcher.ts:257`). Replicate it by filtering out `__proto__` for parity.
6. **The `localeCompare` sort uses ICU collation and depends on the default locale** (`switcher.ts:246`). The output depends on the runtime and locale; Rust needs ICU collation to match.
7. **No id tie-break.** Equal titles in the same rank keep `op` API order, and that order is not guaranteed by `op`. Which value wins can then vary between runs.
8. **A `layer:client` item with `group:*` tags applies to all targets** (`switcher.ts:207`). This is consistent with the README, but it is a pitfall.
9. **A bare-placeholder scope template** (for example `client: "{client}"`) matches every tag, so no shared or target items ever apply (§3.5).
10. **The tag list is joined with `,`** (`onepassword.ts:60`). Templates may contain commas, and `op` would split them.
11. **No `--` before `<item>`** (`onepassword.ts:88-97`). A config item name starting with `-` would be read as an `op` flag.
12. **Runner errors are plain `Error`, so `op` failures exit with 2, not 1.** "op not installed" and "not signed in" get no friendly mapping. The messages are runtime-specific (Bun: `Executable not found in $PATH: "op"`; Node: `spawn op ENOENT`).
13. **ENOTDIR/EACCES during the lstat walk escape as raw fs errors** (exit 2) instead of a user error.
14. **No timeout on `op`.** A hung `op` (for example while waiting for biometric unlock) blocks forever. maxBuffer is 50 MiB.
15. **Writes are not transactional across targets** (`switcher.ts:87-119`). An earlier target's file stays written when a later target fails.
16. **No fsync** before or after the rename (`dotenv.ts:25-27`). After a power loss the file can be empty or old.
17. **The temp file opens without `O_EXCL`** (`dotenv.ts:25`). The UUID makes collisions practically impossible.
18. **An empty merge writes `"\n"`** (`dotenv.ts:15`). The file is created even when no layers matched.
19. **Unquoted control characters** (`dotenv.ts:42`): NUL, `\x01`–`\x08`, `\x0e`–`\x1f`, U+0085 and U+007F are written raw and unquoted.
20. **The marker `path` check reuses `resolveOutputPath`**, so its messages say "target output path…" for markers (`config.ts:326`).
21. **A target named `all` is unselectable** (`config.ts:87`). Parsing does not reserve the name.
22. **`clients`/`environments` entries are not identifier-validated** (`config.ts:39-40`). A non-identifier entry can never be selected.
23. **`trim()` semantics** (§1.4): use a JS-compatible whitespace set, not `str::trim`.
24. **Parse-error text is engine-specific** (`validation.ts:11`). Byte-for-byte parity of the suffix is impossible; tests should match the prefix only.
25. **`tags.base` "is required" branch is dead code** (`config.ts:300-302`).
26. **Unverified `op` facts:** the AND/OR semantics of `op item list --tags`, and whether an empty result prints `[]` or nothing. Verify both against a real `op` before sign-off. They were not tested here by instruction.
