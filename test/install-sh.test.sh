#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
tmpdir="$(mktemp -d)"

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
