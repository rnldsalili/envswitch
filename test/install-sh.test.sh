#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
tmpdir="$(mktemp -d)"
tmpdir="$(cd "$tmpdir" && pwd -P)"
export HOME="$tmpdir/home" USERPROFILE="$tmpdir/home" LOCALAPPDATA="$tmpdir/local" APPDATA="$tmpdir/appdata"
export XDG_DATA_HOME="$tmpdir/data" XDG_CONFIG_HOME="$tmpdir/config" XDG_CACHE_HOME="$tmpdir/cache" XDG_STATE_HOME="$tmpdir/state"
export TMPDIR="$tmpdir/tmp" TEMP="$tmpdir/tmp" TMP="$tmpdir/tmp" BUN_INSTALL="$tmpdir/bun" GH_CONFIG_DIR="$tmpdir/gh"
export SHELL=/bin/bash
unset GITHUB_TOKEN GH_TOKEN ENVSWITCH_INSTALL_TEST_MODE
mkdir -p "$HOME" "$TMPDIR"
python_bin="$(command -v python3)"
mkdir "$tmpdir/tools"
ln -s "$python_bin" "$tmpdir/tools/python3"
export PATH="$tmpdir/tools:/usr/bin:/bin:/usr/sbin:/sbin"

cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

profile="${tmpdir}/.zshrc"
install_dir="${tmpdir}/bin"

ENVS_SWITCH_INSTALL_TEST_MODE=ensure-profile-path \
  bash "${REPO_ROOT}/scripts/install.sh" "$install_dir" "$profile" >/dev/null

if ! grep -Fq "# >>> envswitch PATH >>>" "$profile"; then
  echo "missing envswitch PATH block start" >&2
  exit 1
fi

if ! grep -Fq "export PATH=\"${install_dir}:\$PATH\"" "$profile"; then
  echo "missing envswitch PATH export" >&2
  exit 1
fi

ENVS_SWITCH_INSTALL_TEST_MODE=ensure-profile-path \
  bash "${REPO_ROOT}/scripts/install.sh" "$install_dir" "$profile" >/dev/null

block_count="$(grep -Fc "# >>> envswitch PATH >>>" "$profile")"
if [[ "$block_count" != "1" ]]; then
  echo "expected one envswitch PATH block, found ${block_count}" >&2
  exit 1
fi

echo "install.sh PATH profile helper ok"

python3 "$SCRIPT_DIR/install/test_install.py"
