# AGENTS.md

## Product

- Product name: Bot.
- Repository and directory: `bot`.
- Executable: `bot`.

## Integration boundary

- Use a provider's public and supported protocol.
- Let each provider own authentication and secret storage.
- Do not read, copy, print, or migrate provider tokens.
- Do not call private vendor endpoints.
- Do not identify Bot as an official vendor client.
- Keep provider-specific data behind an adapter.
- Preserve unknown provider data as opaque diagnostic metadata.

## Rust

- Use stable Rust and the 2024 edition.
- Deny unsafe code in Bot-owned crates.
- Use typed errors and explicit domain types.
- Do not add `unwrap` or `expect` outside tests in Bot-owned source.
- Keep imported upstream comments and notices intact.
- Add a file notice when Bot changes an imported file.
- Do not add explanatory comments or commented-out code to Bot-owned source.
- Put design reasons in `docs/decisions/`.
- Keep modules small and keep public APIs narrow.
- Put shared behavior in one common component.
- Add a dependency only for a current requirement.

## Interface

- Keep one reading order: header, transcript, composer, status bar.
- Give every pointer action an equivalent keyboard action.
- Keep focus visible and restore focus after a modal closes.
- Do not use color as the only status cue.
- Keep streamed text stable. Do not move the reader's scroll position.
- Fold detailed reasoning and tool output by default.
- Use sentence case and verb-first action labels.
- Make small terminals useful before adding detail to large terminals.

## Quality

- Run `cargo fmt --all --check`.
- Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- Run `BOT_PROVIDER=grok cargo test --workspace --all-features -- --test-threads=1`
  while imported Grok tests still assume that provider.
- Add snapshot tests for visible TUI changes.
- Add protocol fixtures before a provider adapter ships.
- Keep commits small, focused, and imperative.
- Do not commit unrelated work.
- Do not add co-authors.
