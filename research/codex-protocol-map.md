# Codex app-server protocol map

## Source

This map uses the experimental JSON Schema from installed `codex-cli 0.154.0`.
The generated contract contains 81 server notifications and 11 server requests.

Regenerate it outside the repository:

```text
schema_dir=$(mktemp -d /tmp/bot-codex-schema.XXXXXX)
codex app-server generate-json-schema --experimental --out "$schema_dir"
```

Bot starts `codex app-server --listen stdio://`. Codex owns the account,
credentials, threads, turns, model list, and MCP configuration.

Primary source: [Codex app-server](https://learn.chatgpt.com/docs/app-server).

## Current interaction coverage

| Protocol area | App-server methods | Bot surface |
| --- | --- | --- |
| Turn lifecycle | `turn/started`, `turn/completed` | Existing Grok activity and completion states |
| Text and thinking | `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/plan/delta` | Existing transcript and folded thought blocks |
| Plans | `turn/plan/updated` | Existing Grok plan and todo component |
| Tools | All 19 current item types, command output, terminal input, file patches, MCP progress, dynamic client tools | Existing typed Grok tool cards and ACP client tool bridge |
| Manual compaction | `thread/compact/start` | Existing Grok compact command, status, and result rows |
| Session management | `thread/fork`, `thread/name/set`, `thread/delete`, `thread/turns/list`, `thread/revert` | Existing Grok fork, rename, delete, and rewind flows |
| Mid-turn steering | `turn/steer` | Existing Grok interjection flow with stable local echo |
| Account access | `account/read`, `account/login/start`, `account/login/cancel`, `account/logout`, `account/rateLimits/read` | Shared login screen, `/login`, `/login device`, `/logout`, and `/account` |
| Usage and messages | Token usage, warnings, structured errors, model reroutes | Existing status, warning, and error surfaces |

Bot does not render `item/reasoning/textDelta`. That event can contain hidden
reasoning. Bot renders the provider's reasoning summary instead.

Codex `fileChange` items and `item/fileChange/patchUpdated` notifications use
the imported Grok edit card. Bot converts each unified patch hunk to the edit
detail contract that the original diff renderer consumes. Multi-file changes
get one stable card per file. A nonstandard patch remains visible as a generic
tool card.

## Server requests

| Request group | State | Response path |
| --- | --- | --- |
| Command, file, and permission approvals | Connected | Existing Grok approval card; current and legacy response shapes |
| Agent questions | Connected | Existing Grok question card |
| MCP form, URL, and verification requests | Connected | Existing Grok MCP elicitation and OAuth cards |
| `currentTime/read` | Connected | Whole Unix seconds from the local clock |
| Dynamic client tools | Connected when the ACP client declares tools | Typed `item/tool/call` bridge and native content response |
| Token refresh and attestation | Not advertised | JSON-RPC method error if a server sends one |

MCP elicitation uses typed Codex `0.154.0` request and response structures.
Bot maps all five current modes to the shared MCP form or URL interaction and
passes request `_meta` through the shared component. Schema fixtures cover the
five modes. A full fixture covers the complete form response and URL
completion route through the app-server transport.

An ACP client declares Codex dynamic tools in new-session metadata under
`bot/codexDynamicTools`. Bot sends these definitions on `thread/start`. It
forwards each `item/tool/call` request to `bot/codex/dynamicToolCall` on the ACP
client and returns text, image, or audio content in the native response. A
client that does not own the declared tool gets a typed failed tool result.

Codex keeps token refresh because Bot does not supply ChatGPT tokens. Bot sets
no attestation capability. Those requests must not occur in the current
connection contract.

Agent questions use typed Codex `0.154.0` request and response structures. Bot
applies `isOther`, `isSecret`, and `isBlocking` to each shared question card.
Option-only questions do not show a freeform choice. Secret answers stay masked.
Non-blocking questions use Codex's 120-second auto-resolution window, which stops
after the first user interaction. Answers return under native question IDs, and
freeform notes use Codex's `user_note:` prefix.

## Account access and limits

Bot uses the public Codex app-server account protocol. A signed-out startup
selects browser login when the host has a local browser. It selects device-code
login on a remote or headless host. `/login` uses that automatic choice.
`/login browser` requests browser login, and `/login device` shows the provider
URL and device code. A remote browser login needs a forwarded callback port.
Cancellation calls `account/login/cancel`. `/logout` calls `account/logout` and
returns to the shared signed-out screen only after the provider accepts the
request.

`/account` calls `account/read` and `account/rateLimits/read`. The common account
document shows the account type, email, plan, limit windows, reset times, and
credits that Codex returns. A rate-limit read failure does not hide valid account
data.

Codex owns credential storage and token refresh. Bot does not read or copy the
ChatGPT access token. Source: [Codex app-server](https://learn.chatgpt.com/docs/app-server).

## Permissions, models, and effort

Codex uses Ask for approval, Approve for me, Full Access, and Read Only. Full Access sets the approval policy to `never` and sets the sandbox to `danger-full-access`. Approve for me uses `auto_review` and keeps the workspace sandbox.

Bot gets model IDs and effort keywords from `model/list`. It keeps a separate effort value for each model. A switch rejects an effort that the selected model does not advertise.

Sources:

- [Codex sandbox modes](https://learn.chatgpt.com/docs/sandboxing)
- [Codex permission profiles](https://learn.chatgpt.com/docs/permissions)
- [Grok permission modes](https://docs.x.ai/build/features/permissions)

## Forward compatibility

The transport preserves unknown notification and request payloads as JSON.
Unknown item lifecycle variants stay as opaque provider events. Unknown server
requests get a JSON-RPC `-32601` error. They do not get an empty result.
Provider control-plane notifications do not enter the active transcript unless
they affect the current thread or user interaction.

## Remaining project checks

| Check | Tracking task |
| --- | --- |
| Verify every imported Grok state and input route | Open |
| Add isolated account profiles and more provider adapters | Open |

## Live verification

The core six-test live suite passed with installed `codex-cli 0.154.0` on
13 September 2026. It covered the account and model catalog, text, images,
command approval, MCP discovery, and MCP elicitation. Codex loaded enabled stdio
and OAuth-backed MCP servers through its own configuration.

An authenticated, read-only account check passed on 14 September 2026. It read
the current Codex account, model catalog, and rate limits without printing or
moving credentials. PTY tests also completed browser login, device-code login,
logout, and `/account` against the protocol fixture.

An additional ephemeral live test passed for `thread/compact/start` on
14 September 2026. The adapter test verifies that Bot's `/compact` request reaches
the native method with the active Codex thread ID.

An authenticated live test listed two turns, reverted the thread before the
second turn, and confirmed that only the first turn remained. The adapter fixture
verifies the shared rewind picker and execute routes. Codex rewind changes
conversation history only and keeps workspace file changes.

A live test also renamed, forked, and deleted Codex threads through
`thread/name/set`, `thread/fork`, and `thread/delete` on 14 September 2026. The
adapter fixture verifies the exact thread IDs, child load, working directory,
and trimmed title. Codex rejects empty thread names, so Bot reports that
`/rename --auto` is not available for Codex. Codex assigns fork IDs, so Bot
rejects client-selected IDs for Codex forks.

The same live test writes a unique message and finds the thread through
`thread/search`. Bot maps the native content snippet to the shared session
picker result. It applies the optional working-directory filter after the
provider search because the Codex search request has no working-directory
field.

A full-adapter fixture sends an interjection during an active Codex turn. It
verifies the native thread ID, expected turn ID, content, client message ID,
response, and shared echo notification.

Source: [Codex thread-search tests](https://github.com/openai/codex/blob/6f39a47bb3b04de4c804187bfbf55edc56939aab/codex-rs/app-server/tests/suite/v2/thread_list.rs#L693).

The elicitation test adds a temporary stdio MCP server through process-only
app-server configuration. It does not change the user Codex configuration or
read credentials. Codex requests native MCP tool approval and forwards the
server form. The test accepts both requests and returns a value to the server.
The model then completes the turn.

A separate full-adapter test confirms that the shared Bot card carries the
request and response through the complete Codex adapter.

An authenticated dynamic-tool test passed on Linux on 15 September 2026. Codex
accepted a tool definition on `thread/start`, requested `item/tool/call`, read
Bot's typed result, emitted the completed item, and completed the model turn.
Fixtures cover every item type in the installed `0.154.0` schema. Resume replay
also keeps hook context, function outputs, dynamic tool arguments, and results.
