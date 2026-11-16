#!/usr/bin/env bash
set -euo pipefail
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 BUN_RUNTIME_TRANSPILER_CACHE_PATH=
exec bun "$(dirname "$0")/generate.ts" "$@"
