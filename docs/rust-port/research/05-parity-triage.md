# 05 — Integrated parity run and triage

Integration candidate: `854a2ed` (`Merge F1: parity harness`). This report covers
that product with F1's worker-transport fix; no product source, shared test
support, scenario input, golden, or approved divergence was changed for this run.

## Environments and reproduction

- macOS arm64: Rust 1.100.0, Bun 1.4.0, en-US; macOS goldens.
- Linux arm64: Debian bookworm, Rust 1.100.0, Bun 1.4.0 copied from
  `oven/bun:1.4.0`, en-US; committed Linux goldens. Nonroot container execution
  preserves the EACCES fixture. Native binaries are built inside Linux, with a
  separate container-local target directory.
- `LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8` on both systems.
- No real `op`, actual installer, real HOME/profile write, push or release.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --locked --workspace --bins
LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 \
  cargo test --locked --test parity -- --ignored --nocapture
```

The full command correctly exits **101** on both systems because the integration
parity test remains red. The separately ignored scaffold-version transcript smoke
passes; it is not counted as an additional scenario in the matrix below.

## Results after the harness fix

Each platform runs 474 scenarios. Both have **459 passed, 15 failed**, consisting
of **one product mismatch and 14 pending D3 scenarios**. There are no remaining
harness errors. Windows-only scenarios were not executed.

| Kind | macOS pass | macOS fail | Linux pass | Linux fail |
| --- | ---: | ---: | ---: | ---: |
| blackbox | 15 | 0 | 15 | 0 |
| classify | 66 | 0 | 66 | 0 |
| cli | 109 | 14 | 109 | 14 |
| config | 89 | 0 | 89 | 0 |
| dotenv | 40 | 0 | 40 | 0 |
| formatProgressEvent | 6 | 0 | 6 | 0 |
| inspect | 14 | 0 | 14 | 0 |
| op | 31 | 1 | 31 | 1 |
| parseArgs | 25 | 0 | 25 | 0 |
| path | 27 | 0 | 27 | 0 |
| switch | 28 | 0 | 28 | 0 |
| tags | 9 | 0 | 9 | 0 |
| **Total** | **459** | **15** | **459** | **15** |

## (a) Harness defect — fixed by F1

`worker()` wrote a successful partial transcript with `partial.json()`, an ordinary
JSON object. `bridge.ts finish` expects the tagged ordered form
`["object", [[key, encodedValue], ...]]`, which is produced by `partial.encode()`.
Passing an ordinary object to `plain()` returned undefined, so the bridge failed
at `partial.stdout` before any product comparison.

The earlier smoke wrote its partial with `encode()` directly, and the scaffold's
panicking production stubs never reached the successful-worker branch. That is
why those checks did not expose the defect.

The real worker now uses a tested `serialize_worker_result()` function that emits
`partial.encode().to_string()` for successful results. Panic/error envelopes remain
ordinary objects and are consumed by the parent before bridge decoding. A new
regression calls the actual worker serializer, checks the root `"object"` tag,
and round-trips nested integer-like key order and raw byte values. It failed with
the original producer and passes with the fix. All six formatter scenarios now
pass through the real subprocess/bridge route.

Diagnostics now show the actual exit code, stdout, stderr and result when a
replacement assertion fails, and ordinary comparisons emit all differing fields
instead of truncating after twelve. No divergence assertion was relaxed.

## (b) Product mismatch — C2 (`op`)

### `op-lone-surrogate-value` — kind `op`, both operating systems

Owner: **C2 op**, at
[`parse_item_fields`](../../../crates/envswitch/src/op/parse.rs#L86).
The generic helper it calls is
[`jsutil::parse_json`](../../../crates/envswitch/src/jsutil.rs#L59), owned by C1.

Reproduction:

```sh
LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 \
ENVSWITCH_PARITY_FILTER=op-lone-surrogate-value \
  cargo test --locked --test parity scenario_parity -- --ignored --nocapture
```

Exact synthetic `op` stdout (the backslash escape is literal JSON):

```json
{"fields":[{"label":"A","value":"\ud800"}]}
```

The TS oracle accepts the string and returns `{ "A": "\ud800" }`, exit 0.
That original TS transcript is correct and must remain unchanged. Approved
[DIVERGENCES a7](../DIVERGENCES.md) requires the Rust replacement below.

The following are the complete mismatching fields against that **approved Rust
replacement**, not a direct comparison against the successful TS baseline.
Strings are UTF-8 bytes displayed with an explicit final LF escape.

```diff
-exitCode: 2
+exitCode: 1

-stderr must start: "envswitch: op item get output is not valid JSON: "
+stderr is:         "envswitch: op item get alpha output is not valid JSON: unexpected end of hex escape at line 1 column 40\n"
```

Expected stderr must have a nonempty serde diagnostic after the approved prefix
and a final `\n`; its engine-specific suffix is not otherwise pinned. Actual
stdout is empty and actual result is null, as required for the rejection.

Cause: `parse_item_fields()` calls the generic parser with
`"op item get alpha output"`; that parser returns `Error::User`, whose exit code
is 1. Thus **both the error category/exit code and the a7 prefix differ**. This is
an op-specific approved exception; globally changing the C1 helper would also
change ordinary malformed/config JSON paths that currently pass.

F1 did not change product code or weaken a7 to accommodate this mismatch.
No other C1, C2 dotenv, C3, or C4 mismatch was found in the executed corpus.

## (b) Pending D3 — kind `cli`, both operating systems

These are **not additional implemented-product mismatches**. All fourteen reach
[`cli/update.rs:16`](../../../crates/envswitch/src/cli/update.rs#L16), which is still
`todo!("D3: 01 §6")`, and are reported as caught per-scenario panics. A normal
transcript cannot be compared until D3 is implemented.

Common complete observed outcome for every row:

```text
PANIC: not yet implemented: D3: 01 §6
```

The replacement expectation is the native update contract, with e1/e2/e3 and e6
where marked: compiled current version, embedded installer, receipt settings,
no separate latest-release lookup or version pin, and checked status/summary.
The harness provides synthetic managed receipt/payload/launcher fixtures; its
fake installer performs only the declared test writes.

| Scenario ID | Expected implemented behavior (in addition to the common native assertions) |
| --- | --- |
| `cli-update-empty-current` | Use compiled version despite blank `.version`; invoke installer |
| `cli-update-extra-positional` | Ignore the extra positional; invoke installer |
| `cli-update-failed-installer` | Preserve failure diagnostic and forward installer status 1 |
| `cli-update-fetch-throws` | Never invoke the TS-only lookup seam; installer resolves latest |
| `cli-update-install` | Pass receipt locations and compiled current version, without a pin; validate success |
| `cli-update-installer-sigterm` | Preserve failure diagnostic and forward status 143 (e6) |
| `cli-update-installer-status-4` | Preserve failure diagnostic and forward status 4 (e6) |
| `cli-update-installer-throws` | Controlled launch failure, exit 1, no success summary |
| `cli-update-missing-installer` | Missing bundled script does not prevent embedded installer execution |
| `cli-update-no-reread-fallback` | Native fake installer commits canonical receipt/version metadata; validate success |
| `cli-update-same` | Invoke installer for health/repair even when TS would short-circuit |
| `cli-update-source-checkout` | Source-checkout guard, exit 1, no installer/lookup call |
| `cli-update-unknown-current` | Use compiled version despite absent `.version`; invoke installer |
| `cli-update-unknown-latest` | Delegate latest resolution to installer, without a separate lookup |

## (c) Golden/contract errors

**None identified.** The a7 source golden legitimately succeeds; the mismatch is
with its explicitly approved Rust replacement. The D3 expectations deliberately
differ from legacy TS update behavior under the approved e-rows. No golden was
regenerated, edited, skipped or waived to make the integrated product pass.

## Verification and limits

The worker regression, normal locked Cargo tests (integration parity remains
ignored by default), format/clippy checks, harness/schema self-checks, TS oracle
reproduction, and the explicit full macOS/Linux runs are recorded with this fix.
The full parity command remains red until C2 addresses a7 and D3 is merged.
Windows execution, real 1Password, real installers and live CLI PTY/signal smoke
are outside this run (the normal Cargo suite includes existing component prompt
probes); the per-kind totals apply only to the committed corpus on the
two platforms above.
