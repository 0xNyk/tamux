#!/bin/sh
# install.sh -- Install tamux native binaries without npm
# Usage: curl -fsSL https://raw.githubusercontent.com/mkurman/tamux/main/scripts/install.sh | sh
#        TAMUX_VERSION=0.4.2 ./scripts/install.sh
#        TAMUX_INSTALL_DIR=/opt/tamux/bin ./scripts/install.sh
#        ./scripts/install.sh --dry-run
set -eu

INSTALL_DIR="${TAMUX_INSTALL_DIR:-$HOME/.local/bin}"
SKILLS_DIR="${TAMUX_SKILLS_DIR:-$HOME/.tamux/skills}"
GITHUB_OWNER="mkurman"
GITHUB_REPO="tamux"
GITHUB_API_URL="https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}"
DOWNLOAD_BASE_URL="https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases/download"
DIRECT_INSTALL_MARKER="${INSTALL_DIR}/.tamux-install-source"
BINARIES="tamux tamux-daemon tamux-tui tamux-gateway tamux-mcp"
DRY_RUN=false

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=true ;;
    *)
      echo "Error: Unsupported argument: $arg" >&2
      exit 1
      ;;
  esac
done

die() {
  echo "Error: $*" >&2
  exit 1
}

normalize_version() {
  printf '%s' "$1" | sed 's/^v//'
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    die "Required command not found: $1"
  fi
}

download_file() {
  curl -fsSL \
    -H "Accept: application/vnd.github+json" \
    -H "User-Agent: tamux-installer" \
    "$1" \
    -o "$2"
}

detect_platform() {
  os_name="$(uname -s)"
  arch_name="$(uname -m)"

  case "$os_name" in
    Linux) platform="linux" ;;
    Darwin) platform="darwin" ;;
    *) die "Unsupported OS: $os_name" ;;
  esac

  case "$platform:$arch_name" in
    linux:x86_64|linux:amd64)
      archive_platform="linux-x86_64"
      checksum_platform="linux-x86_64"
      ;;
    linux:aarch64|linux:arm64)
      archive_platform="linux-aarch64"
      checksum_platform="linux-aarch64"
      ;;
    darwin:x86_64|darwin:amd64)
      archive_platform="darwin-x86_64"
      checksum_platform="darwin-x86_64"
      ;;
    darwin:arm64|darwin:aarch64)
      archive_platform="darwin-arm64"
      checksum_platform="darwin-arm64"
      ;;
    *) die "Unsupported architecture for $platform: $arch_name" ;;
  esac

  archive_name="tamux-${archive_platform}.zip"
  checksum_name="SHA256SUMS-${checksum_platform}.txt"

  echo "Detected platform: ${archive_platform}"
}

resolve_version() {
  if [ -n "${TAMUX_VERSION:-}" ]; then
    VERSION="$(normalize_version "$TAMUX_VERSION")"
    echo "Using specified version: ${VERSION}"
    return
  fi

  metadata="$(curl -fsSL \
    -H "Accept: application/vnd.github+json" \
    -H "User-Agent: tamux-installer" \
    "${GITHUB_API_URL}/releases/latest")" || true

  VERSION="$(printf '%s' "$metadata" | tr -d '\n' | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)"

  if [ -z "${VERSION:-}" ]; then
    die "Could not determine latest version from GitHub. Set TAMUX_VERSION=x.y.z"
  fi

  echo "Latest version: ${VERSION}"
}

wait_for_previous_tamux() {
  if [ -z "${TAMUX_UPGRADE_WAIT_PID:-}" ]; then
    return
  fi

  echo "Waiting for tamux process ${TAMUX_UPGRADE_WAIT_PID} to exit..."
  while kill -0 "${TAMUX_UPGRADE_WAIT_PID}" 2>/dev/null; do
    sleep 1
  done
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
    return
  fi

  die "Need sha256sum or shasum to verify downloads"
}

lookup_checksum() {
  awk -v name="$1" '
    $NF == name || $NF == "*" name { print $1; exit }
  ' "$CHECKSUM_PATH"
}

verify_archive() {
  expected_hash="$(lookup_checksum "$archive_name")"

  if [ -z "$expected_hash" ]; then
    return 1
  fi

  actual_hash="$(sha256_file "$ARCHIVE_PATH")"
  if [ "$actual_hash" != "$expected_hash" ]; then
    die "SHA256 mismatch for ${archive_name}"
  fi

  return 0
}

extract_archive() {
  mkdir -p "$EXTRACT_DIR"

  if command -v unzip >/dev/null 2>&1; then
    unzip -oq "$ARCHIVE_PATH" -d "$EXTRACT_DIR"
    return
  fi

  if command -v bsdtar >/dev/null 2>&1; then
    bsdtar -xf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
    return
  fi

  die "Need unzip or bsdtar to extract ${archive_name}"
}

verify_binary() {
  binary_name="$1"
  expected_hash="$(lookup_checksum "$binary_name")"

  if [ -z "$expected_hash" ]; then
    die "Checksum not found for ${binary_name} in ${checksum_name}"
  fi

  actual_hash="$(sha256_file "$EXTRACT_DIR/$binary_name")"
  if [ "$actual_hash" != "$expected_hash" ]; then
    die "SHA256 mismatch for ${binary_name}"
  fi
}

install_binaries() {
  verify_extracted_binaries="$1"
  mkdir -p "$INSTALL_DIR"

  for binary_name in $BINARIES; do
    if [ ! -f "$EXTRACT_DIR/$binary_name" ]; then
      die "Release bundle is missing required binary ${binary_name}"
    fi

    if [ "$verify_extracted_binaries" = true ]; then
      verify_binary "$binary_name"
    fi
    cp "$EXTRACT_DIR/$binary_name" "$INSTALL_DIR/$binary_name"
    chmod 755 "$INSTALL_DIR/$binary_name"
  done

  {
    echo "source=direct"
    echo "install_dir=$INSTALL_DIR"
  } > "$DIRECT_INSTALL_MARKER"

  echo "Installed: ${BINARIES} -> ${INSTALL_DIR}"
}

install_skills() {
  if [ ! -d "$EXTRACT_DIR/skills" ]; then
    die "Release bundle is missing bundled skills"
  fi

  mkdir -p "$SKILLS_DIR"
  cp -R "$EXTRACT_DIR/skills/." "$SKILLS_DIR/"
  echo "Installed bundled skills -> ${SKILLS_DIR}"
}

print_path_hint() {
  case ":${PATH:-}:" in
    *":$INSTALL_DIR:"*)
      echo "tamux: run 'tamux --help' to confirm the install."
      ;;
    *)
      echo "tamux: add '${INSTALL_DIR}' to PATH if 'tamux' is not found."
      echo "tamux: example: export PATH=\"${INSTALL_DIR}:\$PATH\""
      echo "tamux: open a new shell after updating PATH, then run 'tamux --help'."
      ;;
  esac
}

cleanup() {
  if [ -n "${TMP_DIR:-}" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

detect_platform
resolve_version

ARCHIVE_URL="${DOWNLOAD_BASE_URL}/v${VERSION}/${archive_name}"
CHECKSUM_URL="${DOWNLOAD_BASE_URL}/v${VERSION}/${checksum_name}"

if [ "$DRY_RUN" = true ]; then
  echo ""
  echo "Platform: ${archive_platform}"
  echo "Version: ${VERSION}"
  echo "Download URL: ${ARCHIVE_URL}"
  echo "Checksum URL: ${CHECKSUM_URL}"
  echo "Install directory: ${INSTALL_DIR}"
  echo "Skills directory: ${SKILLS_DIR}"
  echo "Binaries: ${BINARIES}"
  echo "Dry run complete -- no files downloaded or modified."
  exit 0
fi

require_command curl
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t tamux-install)"
ARCHIVE_PATH="${TMP_DIR}/${archive_name}"
CHECKSUM_PATH="${TMP_DIR}/${checksum_name}"
EXTRACT_DIR="${TMP_DIR}/extract"
trap cleanup EXIT INT TERM HUP

wait_for_previous_tamux

echo "Downloading tamux v${VERSION} for ${archive_platform}..."
download_file "$CHECKSUM_URL" "$CHECKSUM_PATH"
download_file "$ARCHIVE_URL" "$ARCHIVE_PATH"

verify_extracted_binaries=true
if verify_archive; then
  verify_extracted_binaries=false
fi

echo "Extracting binaries and skills..."
extract_archive

echo "Verifying extracted binaries..."
install_binaries "$verify_extracted_binaries"
install_skills

echo ""
echo "tamux installed successfully."
print_path_hint
