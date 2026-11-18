# envswitch Rust cutover — release checklist

Run these steps **in order**. Every step that pushes, merges, dispatches a workflow, or publishes needs explicit approval from the repo owner. The release-state table in [INSTALLER-PROTOCOL.md](INSTALLER-PROTOCOL.md) §8 explains why the order matters.

Branches (all local until step 1):

| Branch | Base | Contents |
|---|---|---|
| `bridge/m1a-checksummed-source` | `main` | Reviewed release pipeline; the next **source** release also publishes `SHA256SUMS`, `install.sh` and `install.ps1`. Old installers unchanged. |
| `bridge/m1b-dual-mode` | M1a | Final TypeScript bridge `update` (pinned tag, receipt, private installer copy), dual-mode installers (D2b), Light-mode README recovery copy. |
| `rust-port` | `main` (+ everything) | Native Rust binary, parity harness, native release pipeline, native README. |

## 0. CI on Windows (before anything reaches `main`)

1. Push `rust-port` (no publication possible: pushes never publish, and only `main` can publish).
2. Wait for `rust-check`, `installers`, and `parity` to run.
3. The Windows `parity` job uploads the `windows-parity-goldens` artifact and **fails by design** until those goldens are committed. Download it, review the diff against the macOS/Linux goldens (only path and OS-specific differences are expected), commit it under `parity/goldens/windows/`, and push again.
4. Required green on all three OSes: `rust-check`, `installers` (incl. PowerShell 5.1 x64/x86 and pwsh), `parity` (TS self-check **and** the Rust lane).
5. Push `bridge/m1b-dual-mode` and confirm `installers` and `bridge` (Windows bridge runner and migration tests) are green.

## 1. M1a — checksummed source release

1. Merge `bridge/m1a-checksummed-source` → `main`. The push builds and verifies a **source** release; it does not publish.
2. Dispatch `release.yml` on `main` with `kind=source`, `publish=true`.
3. Verify the new latest release (`v0.0.<run>`) has exactly `envswitch_<ver>_source.tar.gz`, `install.sh`, `install.ps1`, `SHA256SUMS`, and that `sha256sum -c SHA256SUMS` passes on the downloaded assets.
4. Do not continue until this checksummed source release is the published **latest**. The dual-mode installers in M1b refuse a release without `SHA256SUMS`, so merging M1b first would break the README one-liner.

## 2. M1b — TypeScript bridge release

1. Merge `bridge/m1b-dual-mode` → `main`. From this moment the README one-liners serve the dual-mode installers. The latest release is the checksummed M1a source release from step 1, so fresh installs keep working.
2. Dispatch `release.yml` with `kind=source`, `publish=true`. That publishes the bridge as latest.
3. Live checks in a throwaway account or VM, not a real profile:
   - Fresh install with the one-liner (macOS, Linux, Windows) installs the bridge source release.
   - A pre-bridge install (`v0.0.5` or older) runs `envswitch update` and lands on the bridge.

## 3. Native release

1. Merge `rust-port` → `main`. The push selects `kind=native` automatically and builds, tests, and verifies all five targets without publishing.
2. Dispatch `release.yml` with `kind=native`, `publish=true`. Required before publication: native builds, installer suites, and oracle parity on all three OSes. Assets are uploaded to a draft, the complete set is verified (including archive contents and the bundled-script bytes), and only then is the release promoted to latest. Attestations are created for the published archives.
3. Live checks:
   - Fresh install with the one-liner on macOS arm64 and x64, glibc Linux, Alpine, WSL, and Windows (PowerShell 5.1 and 7) → `envswitch --version` prints `envswitch <ver> <target>`.
   - Bridge install → `envswitch update` → native. On Windows, run this from the old `envswitch.cmd` launcher: the installer replaces that `.cmd` while cmd.exe is still executing it, so confirm the session ends cleanly and the next `envswitch --version` is native.
   - Native → `envswitch update` → `envswitch is already up to date (v<ver>)`.
   - Pre-bridge install that skipped the bridge → the old updater fails, and the README recovery one-liner fixes it (Light mode, INSTALLER-PROTOCOL §8.2).
   - Optional: `gh attestation verify <archive> --repo rnldsalili/envswitch`.
4. Real 1Password smoke on a real machine: `envswitch doctor …`, `list …`, `switch … --dry-run`, one real write in a scratch project, guided mode, Ctrl-C. Compare against the TS output from before the cutover.

## 4. After the first native release

1. ✅ Done: pushes to `main` publish again (`scripts/release/release.py` `plan`), keeping main-only publication, the required suites, and the latest-order guard (see the `release.yml` header).
2. ✅ Done: the TypeScript app is removed from `main` (`src/`, its Bun tests, `package.json`, `bun.lock`, `tsconfig.json`, the bridge workflow). The parity oracle stays reachable by SHA `53de029`: `parity/oracle/materialize.*` exports it with `git archive`. Source releases can no longer be built; v0.0.10 remains the last (bridge) source release.
3. Follow-up behavior fixes listed in [PLAN.md](PLAN.md) §9, each in its own PR.
