# Architecture and update policy

This policy applies to upstream imports, dependency upgrades, generated code, provider protocol changes, merges, and rebases.

## Architecture boundaries

- `bot-core` owns provider-neutral product types and behavior.
- `bot-provider` owns versioned provider contracts and capabilities.
- A provider adapter owns its authentication, protocol, model catalog, permission terms, session rules, and extension operations.
- The terminal interface renders shared application state. It must not own provider credentials, endpoints, or capability rules.
- Imported `xai-*` crates keep their upstream identity until an accepted decision moves behavior to a Bot-owned boundary.
- `docs/decisions/` owns design reasons and architecture changes.

Put shared behavior in a common component only when two active providers need the same behavior. Do not move provider details into a shared crate to prepare for possible future use.

## Required update workflow

1. Record the source repository and the exact version, tag, or commit.
2. Read the applicable decision records and identify the boundaries that the update can affect.
3. Run the focused tests before the update, or record each existing failure.
4. Port the smallest complete change. Do not replace a Bot-owned file or subtree with an upstream copy.
5. Keep provider-specific behavior behind its adapter. Extend a versioned contract before the interface uses a new provider capability.
6. Preserve licenses, notices, imported comments, and Bot change notices.
7. Add protocol fixtures for protocol changes and snapshots for visible interface changes.
8. Run the required repository checks after the update.
9. Examine the final diff for duplicate components, crossed boundaries, removed behavior, secrets, and unrelated changes.
10. Commit the update as a small focused change. Record a decision when the architecture changes.

## Required checks

Run these checks on Igris for each Rust update:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
BOT_PROVIDER=grok cargo test --workspace --all-features -- --test-threads=1
```

An existing unrelated failure does not make the update successful. Record the exact failure, run the affected focused tests, and do not add a new failure.

## Review result

An update is ready only when all of these statements are true:

- The update keeps the architecture boundaries above.
- Existing supported behavior still works, or an accepted decision records its removal.
- New provider behavior has adapter and protocol coverage.
- Visible behavior has a snapshot or live terminal check.
- The final diff contains only the focused update.
