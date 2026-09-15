# Provider-aware settings and extensions

Date: 2026-09-12

## Context

Bot uses the Grok Build interface foundation, but each provider owns different features and protocols.
Showing an inherited control without a supported provider operation causes errors such as `Method not found`.
It also makes a provider-specific option look like a Bot-wide option.

## Decision

Keep the shared interface small.
Show a provider feature only when the active adapter reports that feature and its supported actions.
Do not emulate a missing provider operation with a private endpoint or direct access to provider credentials.

Keep these shared controls:

- Appearance, input, mouse, transcript, tool display, and accessibility.
- Model and reasoning effort from the active provider catalog.
- Permission and plan modes with the active provider's native names and values.
- Plugins, skills, MCP servers, and hooks when the adapter supports them.

Remove these inherited Grok controls from shared settings:

- SpaceXAI coding data, retention, and training.
- Grok auto-update until Bot owns a provider-neutral update service.
- Grok speech language and voice service until Bot owns a provider-neutral voice service.

Use this Codex extension matrix:

| Area | Codex behavior |
| --- | --- |
| Plugins | Show native inventory and supported actions. |
| Skills | Show native inventory and enable or disable actions. |
| MCP servers | Show native inventory and supported actions. |
| Hooks | Show native inventory as provider-managed. Offer Reload only. |
| Workflows | Hide until the public protocol supplies supported operations. |
| Marketplace | Hide as a separate action panel until the adapter maps native plugin sources and actions. |

## Consequences

The extension modal can use one layout without promising identical provider functions.
Unsupported tabs and actions disappear instead of failing after selection.
New providers must declare capabilities before Bot shows their settings or extension actions.
Provider-owned authentication and configuration stay behind each adapter.
