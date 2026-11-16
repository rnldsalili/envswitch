# Rust port plan review

What this change does: The plan replaces the Bun CLI with native Rust executables while retaining its command behavior and migrating installed copies. This review checks whether the proposed tasks, interfaces, tests, and publication sequence can deliver that result.

Reviewed against commit `53de029a25a717c8ac824ca8553035e2b0fec991` and the current, uncommitted planning documents. Assumed use: a local developer CLI, including existing customized installations and Windows users. Severity: **Blocker** prevents the proposed rollout; **Major** creates a concrete regression or requires substantial implementation replanning; **Minor** is a bounded planning correction.

## Must fix

### 1. Blocker — The cutover requires native migration before a native release exists

**Evidence:** [PLAN.md:179](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:179) (D1), [PLAN.md:194–209](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:194) (Phases 5–6); [03-distribution.md:470–488](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:470); [release.yml:3](/Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:3), [release.yml:48–70](/Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:48).

Phase 6 step 1 publishes a full TS bridge; step 2 requires an untouched pre-bridge updater to reach native through M2; step 3 publishes the first native release. D1 explicitly publishes nothing beforehand. M2's bootstrap requires native assets from its pinned release, so step 2 cannot pass against the bridge release described here. A pre-bridge updater first installs the full TS bridge, not the later M2 archive.

There is also an exposed transition window: the first merge changes the README's mutable `main/scripts/install.*` URLs immediately, while the existing source-only release remains latest until CI finishes. D2's native installer cannot consume that release unless M1's source/native selection and legacy integrity policy have already been integrated. The current main-push workflow publishes automatically after only TS tests/typecheck; it does not wait for the separate `rust-check` workflow or an additional publication approval.

**Fix:** Write an explicit release-state table covering the old source latest, bridge latest, and native latest, including what fresh installers, bridge updaters, and untouched old updaters do in each state. Publish the bridge through a small, separately verified change using installers that support both release kinds. Test old → bridge while the bridge is latest; test old → compatibility archive → native against a complete, locally simulated native release; require the live native check only after native promotion. Gate the publishing job before the first main merge, and verify the exact bridge/native artifact sets before authorizing their publication. Do not wait for an impossible native handoff as the prerequisite for publishing its payload.

**If skipped:** Cutover stalls at step 2, or the gate is bypassed and fresh installs/old updaters encounter an unsupported release layout.

### 2. Blocker — Two workflow counters do not form one release sequence

**Evidence:** [PLAN.md:179](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:179), [PLAN.md:203–207](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:203), [PLAN.md:283](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:283); [release.yml:52](/Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:52), [release.yml:69](/Users/rnldsalili/Documents/career/tools/envswitch/.github/workflows/release.yml:69).

D1 names both `release.yml` and a new `release-rust.yml`, but never identifies which workflow owns the authoritative run number across the bridge and native pipelines. GitHub's `GITHUB_RUN_NUMBER` counts runs of one particular workflow, starting at 1; it is not a repository-wide sequence. A new Rust workflow can attempt a tag already used by the TS workflow, or generate a version lower than the bridge and be rejected by its own latest-order guard. Dry-run dispatches advance that workflow's counter without establishing continuity. [GitHub variable reference](https://docs.github.com/en/actions/reference/workflows-and-actions/variables#default-environment-variables).

**Fix:** Keep the existing publisher workflow as the single version authority and pass its planned version/SHA to any reusable Rust build workflow. Keep both release modes under that authority and serialize the final publication/promotion across modes. If a new publisher is required, explicitly choose and test an offset/allocation rule that exceeds all published versions; add it to Decision D5. Reject tag/SHA conflicts instead of treating an unrelated existing tag as a rerun.

**If skipped:** The first native publication can collide with an old tag, fail ordering protection, or promote a numerical downgrade.

### 3. Major — The interface freeze does not freeze the interfaces the workers actually share

**Evidence:** [PLAN.md:83–100](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:83), [PLAN.md:159–173](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:159); [src/types.ts:1](/Users/rnldsalili/Documents/career/tools/envswitch/src/types.ts:1), [src/switcher.ts:44–67](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:44), [src/cli.ts:43–83](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:43).

The sketch leaves `Deps` fields untyped and refers to undefined `Vault`, `ItemSummary`, `Output`, question, and install-plan types. S1 is not explicitly assigned the config/merge/progress model definitions that C3/C4 are promised, while C1 owns `config/model.rs`. No signatures are frozen for config/tag/path helpers, dotenv writing, `switch_environment`, `inspect_project`, their result types, or logger callbacks. Thus agreeing on the three traits shown does not let the four workers independently agree on their shared API.

Guided discovery adds a concrete Rust constraint: clients must cross worker-thread boundaries and survive selection, and shared factories/loggers need appropriate thread-safety or a main-thread event-collection design. Neither `Send`/`Sync` nor ownership/lifetimes are specified. The suggested `RefCell` test fakes from contract 02 cannot simply become shared thread-safe clients. `Prompter::select -> String` also needs a defined choice/default representation for the boolean Mode question; the existing contract records boolean values/defaults, not arbitrary strings. Finally, injected `root` is not necessarily the process CWD used by `{project}`, and `is_interactive` does not supply the separate stderr-TTY decision used by color formatting.

**Fix:** Make S1 own and compile the shared model definitions, exact helper/function signatures, factory/client lifetime and thread rules, prompt choice/default schema, and typed dependency fields before freezing. Specify actual CWD separately from an overridden project root, and specify environment/TTY access and logging sinks. Keep these definitions separate from C1's implementation ownership, or explicitly hand off the definition files after S1. Use caller-side choice indices if that avoids generic prompt traits. Do not add a clock abstraction merely for symmetry; the existing CLI contract does not require one.

**If skipped:** Workers need incompatible signatures or cannot compile the guided scan, forcing widespread changes after the advertised freeze.

### 4. Major — C3/C4 cannot meet their completion gates independently of C1/C2

**Evidence:** [PLAN.md:135–140](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:135), [PLAN.md:164–173](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:164), [PLAN.md:215–224](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:215); [src/switcher.ts:3–13](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:3), [src/switcher.ts:74–105](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:74), [src/cli.ts:274–323](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:274).

C3 calls config selection, marker/path validation, tag rendering, and the dotenv writer directly. C4 calls config parsing, the switcher, and project inspection directly. Only the `op` and prompt boundaries are faked in the existing tests. Empty S1 modules and model definitions cannot make C3's merge/write/path-swap fixtures or C4's guided/doctor/switch fixtures pass. C2 also needs C1's `jsutil` behavior. The plan allows coding to overlap but incorrectly promises independently green, complete workers before integration.

The schedule also places I1, D3, M1, and M2 in the same wave although I1's merge order includes D3, D3 waits for D2's protocol, and migration consumes D2's final installers and D1's packaging. Phase numbering puts whole-port review before migration, whereas the wave table puts migration before review. These are different acceptance orders. D2 → D3 and migration/package verification are mandatory release dependencies, regardless of whether their estimated durations eventually put them on the longest path.

**Fix:** Publish a dependency graph with separate “may start coding” and “may pass/merge” gates. The smaller implementation sequence is shared definitions/helpers → C1/C2 → C3 → C4 command integration; parsing/output work can start earlier. Freeze D2's protocol before D3/M1/M2, merge the final migration/package changes before I2/R1/R2, and run dependent completion checks after rebasing onto their actual dependencies. Recalculate the critical path from that graph instead of asserting that six complete workers are independent.

**If skipped:** Agents report blocked or misleadingly green work, integration absorbs unfinished testing, and the stated schedule understates the release dependencies.

### 5. Major — The Windows fake `op.cmd` cannot exercise the production spawn path

**Evidence:** [PLAN.md:58–62](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:58), [PLAN.md:160](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:160), [PLAN.md:188](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:188); [src/onepassword.ts:13–18](/Users/rnldsalili/Documents/career/tools/envswitch/src/onepassword.ts:13), [02-core-contract.md:357–370](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/02-core-contract.md:357).

The production Rust runner launches `op` directly. Rust's Windows lookup can infer `.exe`, but does not discover `op.cmd` from the bare name `op`. The TS runner also uses `execFile` without a shell; Node documents that batch scripts require a shell. A PATH containing the proposed `.cmd` shim therefore does not provide the fake executable required by the Windows differential tests. Adding shell execution to production to accommodate the fixture would change argument handling and the security boundary. [Rust Command documentation](https://doc.rust-lang.org/std/process/struct.Command.html#method.new), [Node batch-file documentation](https://nodejs.org/api/child_process.html#spawning-bat-and-cmd-files-on-windows).

**Fix:** Have F1/S1 supply a small native test helper named `op.exe` on Windows, driven by the same fixtures and recording literal argv. Build it only as test infrastructure, and keep production execution shell-free. Make a fake-helper handshake mandatory before running scenarios, and ensure a missing helper cannot fall through to an installed real `op`.

**If skipped:** Windows parity fails before reaching the behavior under test, or accidentally invokes the user's real 1Password CLI.

### 6. Major — F1 needs an oracle/adapter protocol, not just CLI stdout comparisons

**Evidence:** [PLAN.md:160](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:160), [PLAN.md:188](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:188), [PLAN.md:209](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:209); [src/cli.ts:143](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:143), [src/cli.ts:225–232](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:225), [src/cli.ts:334–349](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:334), [src/switcher.ts:217](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:217).

The exported CLI functions are exactly `main`, `parseArgs`, and `formatProgressEvent`; switcher exports are `switchEnvironment`, `inspectProject`, and `classifyItemForTarget`. `mergeTarget`, `buildTagMatcher`, `scopeTagPattern`, `renderTemplate`, `printHelp`, and update helpers are private. F1 can generate merge/tag/help fixtures through the public functions, but cannot directly import those private helpers. No source-export change is necessary if those adapters are specified.

The promised black-box invocation `bun src/cli.ts` cannot cover all CLI rows: captured stdin/stdout makes no-argument guided mode reject non-TTY input, and `update` always sees the repository's `.git` and takes the source-checkout guard. MainOptions fakes used by the real tests are not available through that invocation. Comparing only stdout/stderr/status/files also misses changed `op` argv and duplicate reads; an implementation that rereads every item for every target can produce identical compared outputs.

The oracle also changes under the plan: M1 rewrites `src/cli.ts`, and Phase 6 deletes it. No baseline commit/runtime or durable source materialization is assigned to F1. A runner importing the working tree after these changes either compares against the changed bridge behavior or stops working. OS-specific absolute paths and random temporary roots also cannot be shared as literal macOS-generated goldens on Windows.

**Fix:** Freeze the TS oracle commit and Bun version, record ICU/locale metadata, and retain a way to materialize that baseline in an isolated CI directory after TS removal. Add an in-process TS scenario adapter under `parity/` using exported `main` with fake prompt/client/update dependencies, alongside black-box scenarios for real spawning. Capture structured prompt calls/defaults, progress, `op` argv/read counts, bytes, and relevant file-mode/cleanup assertions. Use platform-specific path expectations or replace only the scenario's known temporary root; preserve literal backslashes and meaningful trailing newlines. Assign Rust adapter/test entry points too.

**If skipped:** The “full differential suite” cannot exercise guided/update parity, can miss changed 1Password behavior, and becomes unusable at cutover.

### 7. Major — ICU root is not an established equivalent of Bun's default collation

**Evidence:** [PLAN.md:75](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:75), [PLAN.md:232](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:232); [src/switcher.ts:242–259](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:242), [02-core-contract.md:313–319](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/02-core-contract.md:313).

The source passes no locale to `localeCompare`; the plan hard-codes root and asserts equivalence. This machine's Bun 1.4.0 reports `en-US`, ICU 78.1, Unicode 17.0. Those facts describe this oracle, not all supported hosts. The ECMAScript default locale is host-defined, while ICU root is an explicit locale; collation also depends on data/version. The short golden list cannot establish general equivalence or justify substituting another collation implementation. [ECMA-402 DefaultLocale](https://tc39.es/ecma402/#sec-defaultlocale), [ICU locale handling and versioning](https://unicode-org.github.io/icu/userguide/collation/architecture.html).

For a concrete consequence, Bun orders `ä` before `z` under English and after `z` under Swedish. Two same-rank items with those titles and different `COMMON` values therefore select different winners. This probe does not claim an observed root-versus-en-US mismatch for the current sample; it establishes why fixed-root equivalence to every host default is an unsupported contract.

**Fix:** Add a Phase 0 decision: preserve the supported host-default behavior, or deliberately adopt one fixed, documented ordering. Freeze the chosen locale/options/data baseline before C3. Expand fixtures to include locale-sensitive letters, composed/decomposed equal strings, punctuation, non-Latin text, and stable equal-title ties, asserting the winning env value as well as order. Approve any remaining ordering divergence explicitly; do not accept `feruca` solely because it passes the 19-title example.

**If skipped:** The same-rank override winner can change for real item titles while all current goldens pass.

### 8. Major — Two contract algorithms disagree with the actual TS source

**Evidence:** [01-cli-contract.md:37–49](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/01-cli-contract.md:37), [src/cli.ts:212–218](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:212); [02-core-contract.md:246–248](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/02-core-contract.md:246), [src/switcher.ts:282–285](/Users/rnldsalili/Documents/career/tools/envswitch/src/switcher.ts:282).

First, the CLI contract says a next token is consumed unless missing or prefixed with `--`. The source uses `!next`, so an empty next argument is not consumed: `parseArgs(["switch", "--help", ""])` returns `help: true` and an empty positional, and `main` exits 0 with help. Following the written algorithm instead makes help a string and causes a validation error. Similarly, `switch --dry-run ""` reaches `--vault is required` in TS rather than a boolean-value error. These were reproduced with exported functions and no I/O dependencies.

Second, the core contract recommends replacing the JS scope wildcard with Rust `.+`. JS dot excludes CR, LF, U+2028, and U+2029; Rust regex dot normally excludes LF. For a shared item containing `env:local`, `layer:shared`, and `client:foreign\r`, TS classifies it as shared because the final tag is not recognized as a client scope. The proposed Rust pattern recognizes that scope and excludes the item. The same TS result was reproduced for U+2028/U+2029. [Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax).

**Fix:** Correct the contracts before freezing fixtures. Assign the empty-next-token cases to F1/C4. Assign CR/LF/U+2028/U+2029 scope cases to F1/C3, and replace each scope sentinel with a nonempty class excluding exactly those four JS line terminators, preserving escaped literal template content. Do not fix these by changing the TS oracle to agree with the proposed Rust implementation.

**If skipped:** Agents faithfully port the research text and ship behavior that differs from the current CLI/classifier.

### 9. Major — Install/release verification depends on a flag explicitly deferred until after cutover

**Evidence:** [PLAN.md:23](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:23), [PLAN.md:270](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:270); [03-distribution.md:374](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:374), [03-distribution.md:451](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:451), [03-distribution.md:557](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:557); [src/cli.ts:146–176](/Users/rnldsalili/Documents/career/tools/envswitch/src/cli.ts:146).

Current `envswitch --version` is an unknown command, exit 1. Section 8 defers adding it, and C4 is told to preserve that behavior. Yet D1/D2 consume distribution contracts requiring the staged binary's `--version` to validate the stamped version before launcher replacement. A parity-correct C4 cannot pass those installer acceptance checks. Conversely, silently adding the flag violates the current D1 parity decision and the stated non-goals.

**Fix:** Resolve this in Phase 0. The smaller useful choice is to approve `--version` as a narrow port divergence, define exact text/newline/leading-`v` behavior, assign it to C4, and update the CLI fixtures and distribution verification together. Otherwise replace every `--version` acceptance requirement with another explicit version-verification mechanism. Also define how the installer verifies the target identity; a version string by itself proves no target triple.

**If skipped:** Either verified installation fails or the port adds an unapproved public behavior.

### 10. Major — The parity decision excludes behavior changes already required elsewhere

**Evidence:** [PLAN.md:23–30](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:23), [PLAN.md:181](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:181), [PLAN.md:235](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:235), [PLAN.md:279–281](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:279); [01-cli-contract.md:346–352](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/01-cli-contract.md:346), [01-cli-contract.md:367–368](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/01-cli-contract.md:367); [02-core-contract.md:205](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/02-core-contract.md:205).

The two divergence categories in §1 cover JS internals and platform-impossible behavior. They do not authorize prompt render-stream/navigation changes, Esc cancellation, or rejecting drive-relative `C:x` paths that Node accepts on the current drive. The risk register accepts broader prompt deltas than Decision D3's Esc-only question. C1's referenced Windows approach expressly recommends such path tightening.

Native update also changes more than internals: compiled versions replace an unknown `.version`, an embedded installer removes the old missing-bundled-script precondition, and a receipt requirement can reject existing manual/copied installations. Contract 01 rows 16–22 cannot all remain literal parity tests for that design. Finally, raw filesystem diagnostics are user-visible: Node's operation/path-bearing `ENOTDIR: ... lstat ...` message is not reproduced by simply displaying Rust's `io::Error`; neither source currently renders a shared cross-language format.

**Fix:** Before S1/F1, approve a concrete divergence table with example input, TS outcome, Rust outcome, owner, and replacement assertion. Include native update/receipt behavior and installer output separately from ordinary command parity. Keep Windows drive-relative parity unless tightening is explicitly approved. Make prompt stream/interaction deltas explicit rather than using “document it later” as authorization. For filesystem errors either preserve operation/path-aware messages or approve specific stable matching rules. Assign D3 replacement tests for all changed update rows.

**If skipped:** Workers cannot distinguish a regression from an accepted change, and “all contract rows ported” becomes either impossible or a blanket waiver.

### 11. Major — Migration and distribution test ownership is incomplete

**Evidence:** [PLAN.md:119–123](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:119), [PLAN.md:157–181](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:157), [PLAN.md:194–201](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:194); [03-distribution.md:470–494](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:470), [03-distribution.md:550–566](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/03-distribution.md:550).

Workers must write only their exclusive file lists, but M1/M2 have no `Owns` column. M1 must change TS update callers/tests and installers that D2 owns. M2 must add compatibility packaging to release files that D1 owns. S1 retains Cargo manifests/lockfiles even though later workers need crate features/dev dependencies; the approved list also omits the junction helper recommended by C1's Windows test contract. Newly shared Rust integration-test support, baseline-retention plumbing, and `.gitignore` changes have no explicit assignment. This is an ownership/hand-off gap, not a reason to let every agent edit `lib.rs`, module indexes, and Cargo manifests.

The distribution acceptance matrix has an additional hole: only optional M2 explicitly owns 03 §B9 migration tests. Selecting “light” removes M2, although M1's bridge still needs verification. No task explicitly owns wiring the full installer/bridge suites into the publication dependency graph. `cargo test` does not execute Bash/PowerShell suites automatically, and the current package test script runs only Bun tests and one Bash profile-helper test. The plan's temporary-environment list also omits `USERPROFILE` and Windows persistent User PATH isolation required by 03 §B9; changing `HOME`/PATH alone does not sandbox those writes.

**Fix:** Add exact M1/M2 file lists and sequential handoffs for D2 → M1 scripts and D1 → M2 packaging. Name a single manifest/lockfile writer after S1 and assign test-support/index exports explicitly. Map every 03 §B9 row to D1, D2, D3, M1, or M2 and a CI command/job; keep bridge/install tests mandatory for both migration-depth choices. Mock persistent Windows PATH mutation, isolate all platform home/profile roots, and make publishing depend on those jobs. Retain the forwarding stub/recovery behavior from 03 §B6 for undiscoverable custom launchers; do not promise to recover settings the old installation never stored.

**If skipped:** Workers either break the ownership rule or leave required migration changes/tests unwritten, and a release can pass Cargo checks without exercising its installers.

## Should fix

### 12. Minor — The proposed nonexistent-tag probe cannot establish AND versus OR

**Evidence:** [PLAN.md:152](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:152), [PLAN.md:241](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:241); [02-core-contract.md:386–387](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/research/02-core-contract.md:386).

An all-nonexistent-tag query is empty under both AND and OR, so P0.2 as described cannot answer both questions. The source already enforces all common tags locally, making remote AND/OR semantics unnecessary to reproduce the current invocation.

**Fix:** Keep the empty-result-format probe if approved, and remove AND/OR from C2's blocking prerequisites. If investigating it remains useful, combine a tag known to match a controlled item with a guaranteed-nonexistent tag; request the needed read-only authorization at implementation time. No `op` command was run for this review.

### 13. Minor — Integration/release edits need an explicit review head and re-check rule

**Evidence:** [PLAN.md:115](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:115), [PLAN.md:133–140](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:133), [PLAN.md:187–190](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:187), [PLAN.md:203–209](/Users/rnldsalili/Documents/career/tools/envswitch/docs/rust-port/PLAN.md:203).

Per-PR cross-model reviews are specified, but orchestrator glue/review fixes and the later workflow switch/TS deletion can change the reviewed result. R1/R2 have no required reviewed SHA, and the text does not say that their clean result expires after those changes. R2's parallel timing also means it should receive an independent scope, not depend on reading R1's conclusions to discover its scope.

**Fix:** Give both whole-port reviewers the same frozen candidate SHA and independent briefs containing requirements, actual source, and test evidence. Require focused cross-model re-review plus relevant checks for subsequent integration/publication edits. Prepare the workflow switch and source-removal changes for review before publishing them. The model labels alone do not substantiate a stronger claim that any individual worker is underpowered; the interface/dependency corrections above are the actionable problems.

## Contract-test coverage audit

No existing core test-table row is inherently without a module owner; the gaps are execution seams, changed expectations, ownership of shared test files, and CI enforcement. The following mapping should be made explicit in the revised task briefs.

| Contract rows | Required owner | Necessary clarification |
| --- | --- | --- |
| 01 §9, 1–6 | C4; F1 oracle fixtures | Pure formatting/parser tests; include empty-next-token behavior from finding 8. |
| 01 §9, 7–15 | C4; F1 in-process adapters | Scripted choices/defaults, factory reuse, concurrent discovery, cancellation, non-TTY guard; freeze thread-safe fakes first. |
| 01 §9, 16–22 | D3; D2 installer-protocol tests | Explicit parity/replacement assertions for compiled versions, receipts, embedded installers, skip/repair, failed resolution, and nonzero status. |
| 01 §9, 23 | C4 dispatch/flag validation, exercised with D3 | Unknown update flags must fail before receipt/network/installer work. |
| 02 §8.2, 1–9 | C1 | Config, selection, paths, markers; Windows junction support and negative marker cases need usable test infrastructure. |
| 02 §8.2, 10–11 | C3 | Classification/scope fixtures, including the JS line-terminator behavior. |
| 02 §8.2, 12–16 | C3, integrated with C1/C2 | Merge bytes/events, no library printing, read cache, symlink-swap-before-write; cannot all pass against empty dependencies. |
| 02 §8.2, 17–22 | C2 | Lenient parsing, argv, field mapping, real child-process stdout secrecy; Windows process helper must be executable. |
| 02 §8.2, 23–25 | C2 | All 22 serialization cases, serialization-before-mutation, atomic replacement/cleanup, Unix-only modes. |
| 03 §B9 | D1/D2/D3/M1/M2, explicitly split | Packaged-target, integrity, detection, rollback, PATH, migration, Windows-lock, and publication tests; retain applicable tests when M2 is omitted. |

## Smaller correction to the plan

Use one publisher/version authority, one installer protocol shared by native update and the bridge, and fewer complete implementation waves following the real config → switcher → CLI dependencies. Generate fixtures through existing exported TS entry points instead of exporting private functions or introducing additional production abstraction layers solely to sustain the six-worker schedule. Let the user choose light migration only if manual recovery for skipped/custom installations is acceptable; otherwise retain the bounded compatibility archive/stub described in 03 §B6.

Verdict: Fix findings 1–11 before approving implementation. Resolve 12–13 in the revised probe/review instructions.

Not checked: No real `op`, installers, builds, complete test suite, CI runs, pushes, tags, or releases were executed. Verification was source/test inspection, read-only Bun probes, and primary documentation; actual Windows behavior, cross-platform collation parity, and packaged migration remain unverified.
