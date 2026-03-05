#!/bin/sh
# Installer for aptos-move-tools
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/gregnazario/aptos-move-tools/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version v0.2.0
#   curl -fsSL .../install.sh | sh -s -- --target x86_64-unknown-linux-musl
#   curl -fsSL .../install.sh | sh -s -- --install-dir /usr/local/bin

set -eu

REPO="gregnazario/aptos-move-tools"
BINARIES="move-suggest move-bounds-checker move1-to-move2"

# Defaults
VERSION=""
TARGET=""
INSTALL_DIR="${HOME}/.local/bin"

usage() {
    cat <<EOF
Install aptos-move-tools binaries.

Usage:
    install.sh [OPTIONS]

Options:
    --version VERSION       Install a specific version (e.g. v0.2.0)
    --target TARGET         Override the target triple (e.g. x86_64-unknown-linux-musl)
    --install-dir DIR       Installation directory (default: ~/.local/bin)
    -h, --help              Show this help message
EOF
}

say() {
    printf 'install: %s\n' "$1"
}

err() {
    printf 'install: error: %s\n' "$1" >&2
    exit 1
}

# ---------- argument parsing ----------

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            VERSION="$2"
            shift 2
            ;;
        --target)
            TARGET="$2"
            shift 2
            ;;
        --install-dir)
            INSTALL_DIR="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "unknown option: $1"
            ;;
    esac
done

# ---------- dependency check ----------

check_dependencies() {
    missing=""

    # Need curl or wget (curl is almost certainly present since this script is fetched via curl)
    if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
        missing="$missing curl"
    fi

    # Need tar for extraction
    if ! command -v tar >/dev/null 2>&1; then
        missing="$missing tar"
    fi

    if [ -n "$missing" ]; then
        say "missing required tools:$missing"
        say ""

        # Suggest install command based on OS/package manager
        if command -v apt-get >/dev/null 2>&1; then
            say "install with:  sudo apt-get install$missing"
        elif command -v dnf >/dev/null 2>&1; then
            say "install with:  sudo dnf install$missing"
        elif command -v yum >/dev/null 2>&1; then
            say "install with:  sudo yum install$missing"
        elif command -v pacman >/dev/null 2>&1; then
            say "install with:  sudo pacman -S$missing"
        elif command -v apk >/dev/null 2>&1; then
            say "install with:  apk add$missing"
        elif command -v brew >/dev/null 2>&1; then
            say "install with:  brew install$missing"
        else
            say "please install the missing tools using your system package manager"
        fi

        exit 1
    fi
}

# ---------- HTTP helper ----------

download() {
    url="$1"
    output="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$output" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$output" "$url"
    else
        err "need curl or wget to download files"
    fi
}

download_to_stdout() {
    url="$1"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O- "$url"
    else
        err "need curl or wget to download files"
    fi
}

# ---------- platform detection ----------

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os_part="unknown-linux" ;;
        Darwin) os_part="apple-darwin" ;;
        *)      err "unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch_part="x86_64" ;;
        aarch64|arm64)  arch_part="aarch64" ;;
        *)              err "unsupported architecture: $arch" ;;
    esac

    # For Linux, detect libc variant
    if [ "$os" = "Linux" ]; then
        libc="gnu"
        # Check for musl
        if [ -f /etc/alpine-release ]; then
            libc="musl"
        elif command -v ldd >/dev/null 2>&1; then
            case "$(ldd --version 2>&1 || true)" in
                *musl*) libc="musl" ;;
            esac
        fi
        # Also check for musl loader
        if ls /lib/ld-musl-* >/dev/null 2>&1; then
            libc="musl"
        fi
        TARGET="${arch_part}-${os_part}-${libc}"
    else
        TARGET="${arch_part}-${os_part}"
    fi
}

# ---------- version resolution ----------

resolve_version() {
    if [ -n "$VERSION" ]; then
        return
    fi

    say "fetching latest release version..."
    # Use the GitHub API redirect to get the latest tag
    VERSION="$(download_to_stdout "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"

    if [ -z "$VERSION" ]; then
        err "could not determine latest release version"
    fi
}

# ---------- checksum verification ----------

verify_checksum() {
    archive_path="$1"
    archive_name="$2"
    checksums_url="https://github.com/${REPO}/releases/download/${VERSION}/checksums-sha256.txt"

    say "downloading checksums..."
    checksums_file="${TMP_DIR}/checksums-sha256.txt"
    download "$checksums_url" "$checksums_file"

    expected="$(grep "  ${archive_name}\$" "$checksums_file" | cut -d' ' -f1 || true)"
    if [ -z "$expected" ]; then
        # Try alternate format (two spaces or single space)
        expected="$(grep "${archive_name}" "$checksums_file" | awk '{print $1}' || true)"
    fi

    if [ -z "$expected" ]; then
        err "checksum for ${archive_name} not found in checksums-sha256.txt"
    fi

    say "verifying SHA-256 checksum..."
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$archive_path" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$archive_path" | cut -d' ' -f1)"
    else
        say "warning: no sha256sum or shasum found, skipping checksum verification"
        return
    fi

    if [ "$actual" != "$expected" ]; then
        err "checksum mismatch!
  expected: ${expected}
  actual:   ${actual}
This could indicate a corrupted download or a tampered file."
    fi

    say "checksum verified OK"
}

# ---------- main ----------

main() {
    check_dependencies

    if [ -z "$TARGET" ]; then
        detect_target
    fi

    resolve_version

    say "installing aptos-move-tools ${VERSION} for ${TARGET}"

    archive_name="aptos-move-tools-${VERSION}-${TARGET}.tar.gz"
    archive_url="https://github.com/${REPO}/releases/download/${VERSION}/${archive_name}"

    # Create temp dir and set up cleanup
    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    archive_path="${TMP_DIR}/${archive_name}"

    say "downloading ${archive_url}..."
    download "$archive_url" "$archive_path"

    verify_checksum "$archive_path" "$archive_name"

    say "extracting..."
    tar xzf "$archive_path" -C "$TMP_DIR"

    # The archive contains a directory named aptos-move-tools-{VERSION}-{TARGET}
    extracted_dir="${TMP_DIR}/aptos-move-tools-${VERSION}-${TARGET}"
    if [ ! -d "$extracted_dir" ]; then
        err "expected directory ${extracted_dir} not found in archive"
    fi

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    # Copy binaries
    for bin in $BINARIES; do
        if [ ! -f "${extracted_dir}/${bin}" ]; then
            err "binary ${bin} not found in archive"
        fi
        cp "${extracted_dir}/${bin}" "${INSTALL_DIR}/"
        chmod +x "${INSTALL_DIR}/${bin}"
        say "installed ${INSTALL_DIR}/${bin}"
    done

    # Print SHA-256 checksums of installed binaries
    say ""
    say "installed binary checksums (SHA-256):"
    for bin in $BINARIES; do
        if command -v sha256sum >/dev/null 2>&1; then
            checksum="$(sha256sum "${INSTALL_DIR}/${bin}" | cut -d' ' -f1)"
        elif command -v shasum >/dev/null 2>&1; then
            checksum="$(shasum -a 256 "${INSTALL_DIR}/${bin}" | cut -d' ' -f1)"
        else
            checksum="(unavailable)"
        fi
        say "  ${bin}: ${checksum}"
    done

    # Check if install dir is in PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*)
            say ""
            say "done! All binaries are ready to use."
            ;;
        *)
            say ""
            say "done! To add the binaries to your PATH, add this to your shell config:"
            say ""

            # Determine the right shell config file
            shell_name="$(basename "${SHELL:-/bin/sh}")"
            case "$shell_name" in
                zsh)    rc_file="~/.zshrc" ;;
                bash)   rc_file="~/.bashrc" ;;
                fish)
                    say "  set -Ux fish_user_paths ${INSTALL_DIR} \$fish_user_paths"
                    say ""
                    say "Or add it to ~/.config/fish/config.fish"
                    return
                    ;;
                *)      rc_file="~/.profile" ;;
            esac

            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            say ""
            say "Then reload your shell or run:"
            say "  source ${rc_file}"
            ;;
    esac
}

main
