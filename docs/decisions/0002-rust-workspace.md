# 0002: Build one typed Rust workspace

- Status: Superseded by 0004
- Date: 2026-09-11

## Context

The application needs a responsive event loop, terminal control, concurrent
provider processes, a normalized event model, and repeatable terminal tests.
Adding Perl would split these responsibilities across two runtimes without a
current product need.

## Decision

Build Bot with stable Rust and the 2024 edition. Use Ratatui for terminal
rendering, Crossterm for terminal input and lifecycle, and Tokio for concurrent
process and event work.

Start with four crates: `bot-core`, `bot-provider`, `bot-ui`, and `bot-cli`.
Add a provider crate only when its vertical slice starts.

Use typed identifiers, typed capabilities, and one shared `Action` type. Route
keyboard, mouse, palette, and help actions through the same dispatcher.

## Consequences

- One type system covers the terminal, application state, and providers.
- Provider dependencies stay outside the core domain.
- The project does not include Perl without a new accepted decision.
- Bot-owned source contains no explanatory comments. Decision records hold design reasons.
- Snapshot, protocol, and pseudo-terminal tests can share common fixtures.
