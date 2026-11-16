# Installer protocol tests

Run `bash test/install-sh.test.sh` on macOS/Linux (existing Python 3 required),
and `pwsh -NoProfile -File test/install-ps.Tests.ps1` on Windows/Linux.
No test installs a runtime or contacts GitHub, Bun, or 1Password.

`fixtures.py` creates complete synthetic releases and a loopback HTTP server.
`fake-gh.py` reads only suite-owned JSON/assets. Native Unix payloads are tiny
Bash programs. Windows tests compile tiny real .NET Framework executables with
the installed Windows PowerShell compiler. Bun and op are stubs. Every test has
isolated homes, tool configuration, temporary files, and PATH. Windows persistent
User PATH reads and writes are redirected to a file under isolated USERPROFILE;
the real registry-backed User PATH is never read or written by the suite.

`archive-fixtures.json` contains base64-encoded minimal tar/zip files for the
PowerShell archive-reader assertions, including links, FIFOs, traversal, case
collisions, and Unicode PAX metadata. Linux PowerShell loads function declarations
through the AST; only the directory ACL helper is replaced for archive tests.
The production Windows OS/CPU gate is never bypassed. Windows integration tests
run separately in CI under pwsh, 64-bit Windows PowerShell 5.1, and 32-bit Windows
PowerShell 5.1 on x64 Windows.

For Docker on Apple Silicon, the tested command is:

```sh
docker run --rm --platform linux/amd64 \
  -e DOTNET_EnableWriteXorExecute=0 -e COMPlus_EnableWriteXorExecute=0 \
  -e DOTNET_TieredCompilation=0 -e POWERSHELL_TELEMETRY_OPTOUT=1 \
  -v "$PWD:/work:ro" mcr.microsoft.com/powershell:7.4-ubuntu-22.04 \
  pwsh -NoProfile -File /work/test/install-ps.Tests.ps1
```

The .NET settings work around QEMU crashes on this host; they are not installer
inputs. No network is used by the tests inside the container. The image must
already be available when running without network access.

D2b covers installers. The untouched-old-installer migration/CLI transcript
assertions belong to M1b; Rust receipt lookup, copied-binary guidance and child
result validation belong to D3; real packaged target smoke tests belong to D1.
These suites use synthetic payloads and do not replace those separate gates.
