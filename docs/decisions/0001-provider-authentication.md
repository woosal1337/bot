# 0001: Use provider-owned authentication and supported protocols

- Status: Accepted
- Date: 2026-09-11

## Context

Bot needs one interface for several coding agents. Each provider has a
different login system, session model, tool protocol, and account policy.
Reading credential files or calling private endpoints would create security,
policy, and maintenance risks.

## Decision

Use Codex app-server for the first provider adapter. Use the Agent Client
Protocol for providers that expose a supported ACP server.

Start provider login through the provider protocol or provider executable.
Keep tokens and refresh behavior inside the provider process. Store only
non-secret profile metadata in Bot.

Reject internal-only authentication routes. Do not support a provider's
subscription in a third-party interface unless that provider permits it.

## Consequences

- Codex can use ChatGPT or API-key login through app-server.
- Grok and Gemini can use their documented ACP modes.
- Claude subscription support stays deferred under the current Anthropic policy.
- Multi-account support requires a tested isolation method for each provider.
- Provider capabilities control the interface. Brand names do not imply parity.

The detailed evidence is in `research/reference-design.md`.
