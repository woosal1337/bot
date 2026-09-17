#!/bin/sh
set -eu

test_directory="$(mktemp -d)"
trap 'rm -rf "$test_directory"' EXIT HUP INT TERM
release_directory="${test_directory}/release"
package_directory="${test_directory}/package/bot-x86_64-unknown-linux-gnu"
darwin_package_directory="${test_directory}/darwin-package/bot-aarch64-apple-darwin"
install_directory="${test_directory}/install"
darwin_install_directory="${test_directory}/darwin-install"
command_directory="${test_directory}/commands"
mkdir -p "$release_directory" "$package_directory" "$darwin_package_directory" "$command_directory"

printf '%s\n' '#!/bin/sh' 'printf "%s\n" "bot-test"' > "${package_directory}/bot"
chmod 0755 "${package_directory}/bot"
tar -C "${test_directory}/package" -czf "${release_directory}/bot-x86_64-unknown-linux-gnu.tar.gz" "bot-x86_64-unknown-linux-gnu"
printf '%s\n' '#!/bin/sh' 'printf "%s\n" "bot-darwin"' > "${darwin_package_directory}/bot"
chmod 0755 "${darwin_package_directory}/bot"
tar -C "${test_directory}/darwin-package" -czf "${release_directory}/bot-aarch64-apple-darwin.tar.gz" "bot-aarch64-apple-darwin"
(
    cd "$release_directory"
    sha256sum bot-x86_64-unknown-linux-gnu.tar.gz bot-aarch64-apple-darwin.tar.gz > SHA256SUMS
)

cat > "${command_directory}/curl" <<'EOF'
#!/bin/sh
set -eu
output=""
url=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o) output="$2"; shift 2 ;;
        http*) url="$1"; shift ;;
        *) shift ;;
    esac
done
cp "${BOT_TEST_RELEASE_DIRECTORY}/${url##*/}" "$output"
EOF
chmod 0755 "${command_directory}/curl"

cat > "${command_directory}/uname" <<'EOF'
#!/bin/sh
set -eu
case "$1" in
    -s) printf '%s\n' "${BOT_TEST_UNAME_S:-Linux}" ;;
    -m) printf '%s\n' "${BOT_TEST_UNAME_M:-x86_64}" ;;
    *) exit 1 ;;
esac
EOF
chmod 0755 "${command_directory}/uname"

cat > "${command_directory}/shasum" <<'EOF'
#!/bin/sh
set -eu
test "$1" = "-a"
test "$2" = "256"
/usr/bin/sha256sum "$3"
EOF
chmod 0755 "${command_directory}/shasum"

BOT_INSTALL_DIR="$install_directory" \
BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
PATH="${command_directory}:/usr/bin:/bin" \
sh "$(dirname "$0")/../install.sh" > "${test_directory}/output"

test -x "${install_directory}/bot"
test "$("${install_directory}/bot")" = "bot-test"
grep -F "Installed Bot in ${install_directory}/bot" "${test_directory}/output" >/dev/null

BOT_INSTALL_DIR="$darwin_install_directory" \
BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
BOT_TEST_UNAME_S=Darwin \
BOT_TEST_UNAME_M=arm64 \
PATH="${command_directory}:/usr/bin:/bin" \
sh "$(dirname "$0")/../install.sh" > "${test_directory}/darwin-output"
test -x "${darwin_install_directory}/bot"
test "$("${darwin_install_directory}/bot")" = "bot-darwin"

printf '%064d  %s\n' 0 bot-x86_64-unknown-linux-gnu.tar.gz > "${release_directory}/SHA256SUMS"
if BOT_INSTALL_DIR="$install_directory" \
    BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
    PATH="${command_directory}:/usr/bin:/bin" \
    sh "$(dirname "$0")/../install.sh" >/dev/null 2>&1; then
    printf '%s\n' "The installer accepted an invalid checksum." >&2
    exit 1
fi
test "$("${install_directory}/bot")" = "bot-test"

printf '%s\n' '#!/bin/sh' 'printf "%s\n" "bot-next"' > "${package_directory}/bot"
chmod 0755 "${package_directory}/bot"
tar -C "${test_directory}/package" -czf "${release_directory}/bot-x86_64-unknown-linux-gnu.tar.gz" "bot-x86_64-unknown-linux-gnu"
(
    cd "$release_directory"
    sha256sum bot-x86_64-unknown-linux-gnu.tar.gz bot-aarch64-apple-darwin.tar.gz > SHA256SUMS
)
BOT_INSTALL_DIR="$install_directory" \
BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
PATH="${command_directory}:/usr/bin:/bin" \
sh "$(dirname "$0")/../install.sh" > "${test_directory}/update-output"
test "$("${install_directory}/bot")" = "bot-next"
if find "$install_directory" -name '.bot-update.*' -print | grep . >/dev/null; then
    printf '%s\n' "The installer left a staged binary behind." >&2
    exit 1
fi

printf '%s\n' "Installer tests passed."
