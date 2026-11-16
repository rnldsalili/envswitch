# Installer protocol tests

Run `bash test/install-sh.test.sh` on macOS/Linux and
`pwsh -NoProfile -File test/install-ps.Tests.ps1` on Windows/Linux.
Python 3 is a test-harness dependency only: the Bash installer uses Bash,
POSIX awk, curl/authenticated gh, tar, a SHA-256 tool, and standard Unix tools.
Set `INSTALL_TEST_PYTHON` to an existing test interpreter when necessary.
No test installs a runtime or contacts GitHub, Bun, or 1Password.

All suite roots are private directories beneath `test/install`. `harness.py`
checks resolved destinations before every Python fixture write and rejects
symlink escapes. Fake tools replace only suite-owned paths. The no-Python case
creates a fresh bin directory containing links to an explicit allowlist of tools;
it never writes through a PATH entry. Homes, XDG directories, profiles, tool
configuration, temporary files, and PATH are isolated. The suites set a dead
HTTPS proxy and loopback-only NO_PROXY; full install test mode requires an
explicit loopback API origin. Linux CI runs suites with `--network none`.

`fixtures.py` serves complete synthetic releases over loopback, including asset
IDs and redirect/retry cases. `fake-gh.py` reads only suite-owned JSON/assets.
Native Unix payloads are tiny shell scripts. Windows fixtures compile tiny real
.NET Framework executables using the installed Windows PowerShell compiler.
Bun and op are stubs. Persistent User PATH uses the required mock file under
USERPROFILE; registry type preservation is also checked with a memory-only key.
No suite reads or writes the real persistent User PATH.

`json-cases.tsv` exercises the single bundled awk grammar, including UTF-8,
escapes, surrogate pairs, duplicate decoded keys, numbers, and invalid syntax.
Run `sh test/install/test-json-awk.sh "$PWD" /usr/bin/awk` for BWK awk, or substitute
`gawk` / `busybox awk`. CI covers macOS BWK awk, gawk, and Alpine BusyBox awk.
Bash pipe/helper/full-install cases run on macOS Bash 3.2 and Linux Bash 5.
The Alpine job builds `Dockerfile.alpine` with test dependencies before running
the complete suite with networking disabled and BusyBox tar/awk. Hard-link
fixtures target only suite-owned secret files; both listing and post-extraction
inode checks are exercised before chmod. Controlled failures compare full stderr.

`archive-fixtures.json` contains minimal tar/zip fixtures for PowerShell archive
readers. Linux pwsh loads function declarations through the AST; the production
Windows OS gate is never bypassed. Windows integration tests run under pwsh 7,
Windows PowerShell 5.1 x64, and 5.1 x86, including `iex`, localized `chcp`, and
302 redirects, and one transaction-level confirmation. Those Windows process semantics cannot run on Linux pwsh.

For Docker on Apple Silicon, using an already available image:

```sh
docker run --rm --network none --platform linux/amd64 \
  -e DOTNET_EnableWriteXorExecute=0 -e COMPlus_EnableWriteXorExecute=0 \
  -e DOTNET_TieredCompilation=0 -e POWERSHELL_TELEMETRY_OPTOUT=1 \
  -v "$PWD:/work" mcr.microsoft.com/powershell:7.4-ubuntu-22.04 \
  pwsh -NoProfile -File /work/test/install-ps.Tests.ps1
```

The .NET settings work around QEMU crashes; they are not installer inputs.
PowerShell also checks child-scope `iex` hygiene and nonfatal notification failure.
Profile cases cover append-only writes, CRLF blocks, and dangling symlinks.
D2b covers installers. M1b owns old-installer migration/CLI transcript assertions;
D3 owns native updater invocation/result validation; D1 owns real target smoke
tests. Synthetic installer fixtures do not replace those publication gates.
