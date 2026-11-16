#!/usr/bin/env bash
# Shared by the unprivileged test job and the archive-producing job.
set -euo pipefail

git -c core.autocrlf=false -c core.eol=lf checkout-index --all --force
test "$(git rev-parse HEAD)" = "$SOURCE_SHA"
test "$TAG" = "v$VERSION"

if [[ "$RUNNER_OS" == Linux ]]; then
  sudo apt-get update
  sudo apt-get install --yes musl-tools binutils file
fi

toolchain=$("$PYTHON" -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml", "rb"))["toolchain"]["channel"])')
rustup toolchain install "$toolchain" --profile minimal
rustup target add "$TARGET"
rustc --version
"$PYTHON" scripts/release/release.py stamp . "$VERSION"
# cargo test --locked in the test job validates the reconciled lockfile.
