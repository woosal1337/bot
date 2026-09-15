# 0004: Use Grok Build as the complete Bot foundation

- Status: Accepted
- Date: 2026-09-11
- Git history rule superseded by decision 0013.

## Context

The first Bot prototype proved the Codex app-server boundary. It did not match
the required Grok Build interface or feature set. A partial visual port would
repeat tested terminal, session, tool, command, and rendering work.

Grok Build is open source under Apache-2.0. The official source commit
`37949780c144e37df692e3d669051a21fec24f20` contains the complete terminal
application and its Git history.

## Decision

Use the full Grok Build source as the Bot foundation. Preserve its working
features while Bot changes the product identity and adds provider adapters.

Keep the upstream Git history, license, copyright, attribution, and third-party
notices. Keep upstream comments intact. Put a prominent change notice in each
imported file that Bot changes.

Treat the imported Grok runtime as the first provider. Do not replace it with a
smaller compatibility layer. Add Codex, Claude, and later providers behind
provider-owned authentication and supported protocols.

Do not rename all `grok` strings at one time. Separate these meanings first:

- Bot product and executable identity
- Grok provider identity
- Grok-owned account, storage, and protocol identity
- Internal upstream crate identity during the migration

## Required parity

The migration keeps the upstream TUI, slash commands, scrollback, composer,
sessions, tools, workflows, settings, plugins, media, keyboard, mouse, and
terminal behavior functional.

No migration slice can remove a working upstream feature without a tested Bot
replacement. Each slice must pass its focused tests and a live terminal check.

## Consequences

- Decision 0002 no longer controls the workspace layout.
- The old Bot crates stay outside the imported workspace until adapter work uses them.
- Internal `xai-*` crate names can remain during the first migration stage.
- Bot can ship the complete interface before it abstracts every provider boundary.
- Source changes need license checks, change notices, tests, and a focused commit.
