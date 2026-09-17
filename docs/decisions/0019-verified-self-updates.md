# Decision 0019: Use an explicit verified self-update command

## Status

Accepted on 2026-09-17.

## Context

Bot removed the inherited Grok updater because it used provider-specific
release behavior. Bot 1.0.2 uses verified release installers, but installed
copies must run an installer again for each update.

## Decision

Bot owns update behavior in the `bot-update` crate. `bot update --check` reads
the latest stable GitHub release without changing files. `bot update` installs
that release only from a production build.

The updater selects an exact platform archive. It validates the GitHub asset
digest and the matching `SHA256SUMS` entry. It also checks the byte count and
the staged binary version. A file lock prevents two update processes from
changing the same installation.

Unix installations use a same-directory atomic replacement. Windows uses a
separate helper after the update command exits because the running executable
cannot replace itself. Bot does not silently check or install updates during
startup.

## Consequences

Bot 1.0.2 and earlier need one final installer update. Later releases can use
the update command. Active sessions keep their current version until restart.

GitHub Releases remains the distribution trust boundary. Release artifacts
keep their checksums and GitHub build provenance. Users can verify provenance
with GitHub CLI when they need that additional check.
