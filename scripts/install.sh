#!/bin/sh
set -eu

repository="${BOT_REPOSITORY:-woosal1337/bot}"
version="${BOT_VERSION:-latest}"
install_directory="${BOT_INSTALL_DIR:-${HOME}/.local/bin}"

case "$(uname -s)" in
    Linux) system="unknown-linux-gnu" ;;
    Darwin) system="apple-darwin" ;;
    *) printf '%s\n' "Bot does not provide a binary for this operating system." >&2; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64|amd64) architecture="x86_64" ;;
    arm64|aarch64) architecture="aarch64" ;;
    *) printf '%s\n' "Bot does not provide a binary for this architecture." >&2; exit 1 ;;
esac

target="${architecture}-${system}"
archive_name="bot-${target}.tar.gz"

if [ "$version" = "latest" ]; then
    release_path="latest/download"
else
    version="${version#v}"
    release_path="download/v${version}"
fi

base_url="https://github.com/${repository}/releases/${release_path}"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM
archive_path="${temporary_directory}/${archive_name}"
checksums_path="${temporary_directory}/SHA256SUMS"

curl --proto '=https' --tlsv1.2 -fsSL "${base_url}/${archive_name}" -o "$archive_path"
curl --proto '=https' --tlsv1.2 -fsSL "${base_url}/SHA256SUMS" -o "$checksums_path"

expected="$(awk -v name="$archive_name" '$2 == name { print $1 }' "$checksums_path")"
if [ -z "$expected" ]; then
    printf '%s\n' "Bot could not find the release checksum for ${archive_name}." >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$expected" "$archive_path" | sha256sum --check --status
elif command -v shasum >/dev/null 2>&1; then
    printf '%s  %s\n' "$expected" "$archive_path" | shasum -a 256 --check --status
else
    printf '%s\n' "Bot needs sha256sum or shasum to verify the download." >&2
    exit 1
fi

tar -xzf "$archive_path" -C "$temporary_directory"
mkdir -p "$install_directory"
install -m 0755 "${temporary_directory}/bot-${target}/bot" "${install_directory}/bot"

printf 'Installed Bot in %s\n' "${install_directory}/bot"
if ! command -v bot >/dev/null 2>&1; then
    printf 'Add %s to PATH, then run bot.\n' "$install_directory"
fi
