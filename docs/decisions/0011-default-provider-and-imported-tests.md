# 0011 — Select Codex by default

## Decision

The installed `bot` executable selects Codex when no provider is specified.
`--provider` and `BOT_PROVIDER` can select another provider.

The imported Grok Build integration tests still assume Grok for commands that
do not name a provider. The workspace test step sets `BOT_PROVIDER=grok` for
those inherited tests. Tests that clear the environment or check Grok-only
behavior name Grok explicitly. The direct executable check and a parser test
check the production Codex default outside that test setting.

The isolated Grok test environment also selects Grok. It clears the parent
environment, so the test-step setting alone cannot reach its child processes.

## Reason

The distribution binary must keep the Codex-first launch behavior that the
older local launcher provided. Imported Grok tests must still test the Grok
runtime instead of starting an unrelated provider.
