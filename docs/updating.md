# Update Bot

## Installed releases

Run this command to check the latest stable release without changing files:

```sh
bot update --check
```

Run this command to install the latest stable release:

```sh
bot update
```

Bot 1.0.2 and earlier do not include these commands. Run the installer once to
move those copies to a release that includes the updater.

## Safety model

The updater gets the latest stable release from the public GitHub Releases API.
It selects one archive for the current operating system and CPU architecture.
It then applies these checks:

1. Reject a draft or prerelease.
2. Limit the download size and require the exact reported byte count.
3. Match the archive with the SHA-256 digest in GitHub release metadata.
4. Match the same digest with the release `SHA256SUMS` file.
5. Extract only the expected `bot` entry from the archive.
6. Run the staged binary and match its version with the release tag.

The updater uses a file lock to reject concurrent updates. On macOS and Linux,
it stages the binary in the install directory and uses an atomic rename. On
Windows, it starts a helper that waits for the update command to exit. The
helper keeps the old binary until the staged file is in place.

The command does not stop active Bot sessions. Those sessions keep their old
version until they restart. Bot does not check or install updates during normal
startup.

## Source builds and forks

Source builds can use `bot update --check`, but they cannot install an update.
Build the source again when you need to update a development copy.

Set `BOT_REPOSITORY=owner/repository` to use a compatible fork that publishes
the same release assets. The updater never uses provider credentials.

## Recovery

If an update fails before replacement, Bot leaves the installed binary intact.
Run the installer again if the update command cannot write to its directory.

Check the installed version after an update:

```sh
bot --version
```

Each release also includes build provenance. If GitHub CLI is installed, check
a downloaded archive with this command:

```sh
gh attestation verify bot-*.tar.gz --repo woosal1337/bot
```
