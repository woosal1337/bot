## Change

Describe the user-visible result and the smallest complete change.

## Update source

For an upstream, dependency, generated-code, provider, merge, or rebase update, link the source and give the exact revision.

## Architecture check

- [ ] I read `docs/architecture-and-updates.md`.
- [ ] Provider-specific behavior stays behind its adapter.
- [ ] This change does not replace a Bot-owned file or subtree with an upstream copy.
- [ ] The diff keeps imported notices and Bot change notices.
- [ ] An accepted decision records each architecture change or supported behavior removal.

## Tests

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] Focused tests for the changed behavior
- [ ] Protocol fixtures for a protocol change
- [ ] Snapshots or a live terminal check for a visible interface change

List each existing unrelated failure and show that this change adds no failure.
