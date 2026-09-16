# Contributing

Issues and focused pull requests are welcome.

## Before you start

1. Search existing issues and pull requests.
2. Open an issue before a large design or provider change.
3. Use only public, supported provider protocols.
4. Do not include credentials, private endpoints, or captured user data.

## Checks

Run the commands in the README development section. Add protocol fixtures for a
provider change and snapshot tests for a visible TUI change.

## Update changes

Read `docs/architecture-and-updates.md` before an upstream, dependency,
generated-code, provider, merge, or rebase update.

Each update must preserve Bot's architecture boundaries and supported behavior.
Establish the current test result first. Port the smallest complete change and
preserve notices. Add protocol fixtures or interface snapshots when applicable.
Run all required checks and examine the final diff before you commit.

Keep changes small. Preserve upstream notices in imported files. Do not add a
dependency unless the change needs it now.

Contributions are licensed under Apache-2.0.
