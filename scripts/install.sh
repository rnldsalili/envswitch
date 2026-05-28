#!/usr/bin/env bash

set -euo pipefail

REPO="${REPO:-rnldsalili/envswitch}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
APP_ROOT="${APP_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/envswitch}"
APP_DIR="${APP_ROOT}/current"
COMMAND_NAME="envswitch"

log_info() {
  printf 'info: %s\n' "$1"
}

log_success() {
  printf 'success: %s\n' "$1"
}

log_error() {
  printf 'error: %s\n' "$1" >&2
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

validate_repo() {
  local repo="$1"

  if [[ ! "$repo" =~ ^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$ ]]; then
    log_error "invalid repository format: $repo"
    log_error "expected owner/repo"
    exit 1
  fi
}

validate_absolute_dir() {
  local name="$1"
  local dir="$2"

  if [[ "$dir" != /* ]] || [[ "$dir" == *..* ]]; then
    log_error "invalid ${name}: $dir"
    log_error "${name} must be an absolute path without '..'"
    exit 1
  fi
}

detect_platform() {
  local os

  os="$(uname | tr '[:upper:]' '[:lower:]')"
  case "$os" in
    darwin)
      log_info "detected macOS"
      ;;
    linux)
      if [[ -r /proc/version ]] && grep -qi microsoft /proc/version; then
        log_info "detected WSL/Linux"
      else
        log_info "detected Linux"
      fi
      ;;
    *)
      log_error "unsupported OS for this installer: $os"
      log_error "use scripts/install.ps1 on Windows"
      exit 1
      ;;
  esac
}

bun_path() {
  if command_exists bun; then
    command -v bun
    return 0
  fi

  if [[ -x "${HOME}/.bun/bin/bun" ]]; then
    printf '%s\n' "${HOME}/.bun/bin/bun"
    return 0
  fi

  return 1
}

install_bun() {
  if ! command_exists curl; then
    log_error "curl is required to install Bun automatically"
    log_error "install Bun manually from https://bun.com/docs/installation, then rerun this installer"
    exit 1
  fi

  log_info "Bun is not installed; installing Bun"
  curl -fsSL https://bun.com/install | bash

  export BUN_INSTALL="${BUN_INSTALL:-$HOME/.bun}"
  export PATH="${BUN_INSTALL}/bin:${PATH}"
}

ensure_bun() {
  local bun_bin

  if ! bun_bin="$(bun_path)"; then
    install_bun
    bun_bin="$(bun_path)" || {
      log_error "Bun installation finished, but bun was not found on PATH"
      exit 1
    }
  fi

  log_info "using Bun at ${bun_bin}"
  "$bun_bin" --version >/dev/null
  BUN_BIN="$bun_bin"
}

get_latest_release() {
  local latest api_response

  if command_exists gh && gh auth status >/dev/null 2>&1; then
    latest="$(gh api "repos/${REPO}/releases/latest" --jq '.tag_name' 2>/dev/null || true)"
  else
    if ! command_exists curl; then
      log_error "curl or an authenticated GitHub CLI is required to download envswitch"
      exit 1
    fi

    local curl_opts=(-fsSL)
    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
      curl_opts+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
    fi

    api_response="$(curl "${curl_opts[@]}" "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)"
    latest="$(printf '%s' "$api_response" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  fi

  if [[ ! "$latest" =~ ^v?[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]]; then
    latest=""
  fi

  if [[ -z "$latest" ]]; then
    log_error "no GitHub Release found for ${REPO}, or access is denied"
    log_error "publish a release with an envswitch_<version>_source.tar.gz asset first"
    exit 1
  fi

  printf '%s\n' "$latest"
}

download_archive() {
  local latest="$1"
  local archive="$2"
  local tmpdir="$3"
  local url

  url="https://github.com/${REPO}/releases/download/${latest}/${archive}"

  if command_exists gh && gh auth status >/dev/null 2>&1; then
    log_info "downloading ${archive} with GitHub CLI"
    if gh release download "$latest" --repo "$REPO" --pattern "$archive" --dir "$tmpdir" >/dev/null 2>&1; then
      return 0
    fi

    log_info "GitHub CLI download did not find ${archive}; trying direct download"
  else
    log_info "downloading ${archive}"
  fi

  local curl_opts=(-fsSL -H "Accept: application/octet-stream")
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl_opts+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
  fi

  if ! curl "${curl_opts[@]}" "$url" -o "$tmpdir/$archive"; then
    log_error "release asset not found or unavailable: ${archive}"
    log_error "expected asset URL: ${url}"
    exit 1
  fi
}

install_source() {
  local archive_path="$1"
  local tmpdir="$2"
  local staging source_dir

  staging="${tmpdir}/staging"
  mkdir -p "$staging" "$APP_ROOT"

  log_info "extracting source archive"
  tar -xzf "$archive_path" -C "$staging"

  source_dir="${staging}/envswitch"
  if [[ ! -f "${source_dir}/package.json" ]] || [[ ! -f "${source_dir}/src/cli.ts" ]]; then
    log_error "archive did not contain an envswitch source package"
    exit 1
  fi

  log_info "installing production dependencies"
  (cd "$source_dir" && "$BUN_BIN" install --production --frozen-lockfile)

  rm -rf "$APP_DIR"
  mv "$source_dir" "$APP_DIR"
}

write_launcher() {
  local tmpdir="$1"
  local launcher_tmp target_launcher

  launcher_tmp="${tmpdir}/${COMMAND_NAME}"
  target_launcher="${INSTALL_DIR}/${COMMAND_NAME}"

  cat >"$launcher_tmp" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if command -v bun >/dev/null 2>&1; then
  BUN_BIN="\$(command -v bun)"
elif [[ -x "\${HOME}/.bun/bin/bun" ]]; then
  BUN_BIN="\${HOME}/.bun/bin/bun"
else
  printf 'envswitch: bun is not installed or not on PATH\n' >&2
  exit 127
fi

exec "\$BUN_BIN" "${APP_DIR}/src/cli.ts" "\$@"
EOF

  chmod 0755 "$launcher_tmp"

  if [[ ! -d "$INSTALL_DIR" ]]; then
    mkdir -p "$INSTALL_DIR"
  fi

  log_info "installing launcher to ${target_launcher}"
  install -m 0755 "$launcher_tmp" "$target_launcher"
}

main() {
  local latest version archive tmpdir

  validate_repo "$REPO"
  validate_absolute_dir "INSTALL_DIR" "$INSTALL_DIR"
  validate_absolute_dir "APP_ROOT" "$APP_ROOT"
  detect_platform
  ensure_bun

  latest="$(get_latest_release)"
  version="${latest#v}"
  archive="envswitch_${version}_source.tar.gz"

  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  download_archive "$latest" "$archive" "$tmpdir"
  install_source "$tmpdir/$archive" "$tmpdir"
  write_launcher "$tmpdir"

  log_success "installed envswitch ${latest}"
  log_info "source: ${APP_DIR}"
  log_info "launcher: ${INSTALL_DIR}/${COMMAND_NAME}"
  if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
    log_info "add ${INSTALL_DIR} to your PATH if '${COMMAND_NAME}' is not found"
  fi
  log_info "run '${COMMAND_NAME} --help' to verify the installation"
}

main "$@"
