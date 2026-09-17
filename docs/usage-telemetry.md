# Usage telemetry

Bot shows only values that the active provider returns through a supported protocol. A missing value stays unavailable. Bot does not estimate an account balance from local messages.

| Provider | Account allowance | Account tokens | Conversation context | Session tokens |
| --- | --- | --- | --- | --- |
| Codex | Primary and secondary limit windows, percent used, percent left, and reset time | Lifetime tokens | Live tokens used and model context limit | Included in the live context total |
| Grok | Unavailable | Unavailable | Live tokens used and model context limit | Native session token and cost totals in `/usage` |
| Claude | Unavailable | Unavailable | Available when a future supported adapter sends ACP usage updates | Unavailable |

Codex gets account data from `account/rateLimits/read`, `account/rateLimits/updated`, and `account/usage/read`. It gets conversation data from `thread/tokenUsage/updated`. Bot refreshes account data when a session starts, a session resumes, a turn finishes, or an in-app sign-in completes. It clears account data after logout.

The Codex resume response does not include a current context snapshot. A resumed session shows context data after Codex sends the next token-usage update. Bot does not display zero while it waits.

The imported Grok runtime supplies live conversation context and native session totals. Its account-wide allowance uses a private product-service route in the upstream client. Bot does not call that route, so account allowance and account tokens stay unavailable.

Provider-specific fields can cross the shared telemetry contract as opaque extension data. The common interface ignores fields that it does not understand.
