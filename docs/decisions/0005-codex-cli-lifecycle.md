# Codex CLI lifecycle

Date: 2026-09-12

## Decision

The TUI and single-turn CLI use one Codex ACP adapter. The imported Grok runtime remains the Grok adapter.
The CLI uses the selected provider's session IDs. Unsupported flags fail before the turn starts.
Grok-only subcommands require an explicit Grok selection.

Keep the event receiver created before `turn/start`. Replacing it can lose text or completion events that arrive before the response.
After Codex accepts the turn, send a client prompt acknowledgement through the shared queue contract.

Headless Ctrl+C interrupts the turn and requests cleanup of that thread's background terminals.
Codex exposes terminal cleanup through an experimental app-server method, so Bot opts into experimental API fields during initialization.
The adapter does not change stored provider settings or credentials.

Close the app-server input pipe before process teardown. Allow five seconds for shutdown before the existing process-drop fallback.
Use a separate managed process group for Codex so terminal Ctrl+C reaches Bot without killing the provider first.
Do not treat a cancelled model turn as proof that its shell processes stopped. Check the process lifecycle separately.

## Evidence

The CLI test fixture emits text and completion before the turn-start response.
Separate tests cover plain output, JSON resume, provider errors, unsupported flags, cancellation cleanup, and the Grok-only login guard.
Live tests use a temporary directory and the installed Codex account without Grok credentials.

Contract: [Codex app-server](https://learn.chatgpt.com/docs/app-server), checked against the installed CLI's generated JSON schema.
