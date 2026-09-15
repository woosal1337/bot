#!/bin/sh
set -eu

test_directory="$(mktemp -d)"
trap 'rm -rf "$test_directory"' EXIT HUP INT TERM
release_directory="${test_directory}/release"
package_directory="${test_directory}/package/bot-x86_64-unknown-linux-gnu"
install_directory="${test_directory}/install"
command_directory="${test_directory}/commands"
mkdir -p "$release_directory" "$package_directory" "$command_directory"

printf '%s\n' '#!/bin/sh' 'printf "%s\n" "bot-test"' > "${package_directory}/bot"
chmod 0755 "${package_directory}/bot"
tar -C "${test_directory}/package" -czf "${release_directory}/bot-x86_64-unknown-linux-gnu.tar.gz" "bot-x86_64-unknown-linux-gnu"
(
    cd "$release_directory"
    sha256sum bot-x86_64-unknown-linux-gnu.tar.gz > SHA256SUMS
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

BOT_INSTALL_DIR="$install_directory" \
BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
PATH="${command_directory}:/usr/bin:/bin" \
sh "$(dirname "$0")/../install.sh" > "${test_directory}/output"

test -x "${install_directory}/bot"
test "$("${install_directory}/bot")" = "bot-test"
grep -F "Installed Bot in ${install_directory}/bot" "${test_directory}/output" >/dev/null

printf '%064d  %s\n' 0 bot-x86_64-unknown-linux-gnu.tar.gz > "${release_directory}/SHA256SUMS"
if BOT_INSTALL_DIR="$install_directory" \
    BOT_TEST_RELEASE_DIRECTORY="$release_directory" \
    PATH="${command_directory}:/usr/bin:/bin" \
    sh "$(dirname "$0")/../install.sh" >/dev/null 2>&1; then
    printf '%s\n' "The installer accepted an invalid checksum." >&2
    exit 1
fi

printf '%s\n' "Installer tests passed."
