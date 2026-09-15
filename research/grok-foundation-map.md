# Grok Build foundation map

## Source baseline

- Repository: `https://github.com/xai-org/grok-build.git`
- Imported commit: `37949780c144e37df692e3d669051a21fec24f20`
- Import commit: `e72a593a795b65cc264eb40709dfa17c586d190a`
- Branch: `foundation/grok-build`
- License: Apache-2.0
- Upstream copyright: `2023-2026 SpaceXAI`

The import keeps the upstream Git history. It also keeps `LICENSE`,
`THIRD-PARTY-NOTICES`, and the notices under `third_party/`.

## Composition map

| Responsibility | Foundation path |
| --- | --- |
| Product binary | `crates/codegen/xai-grok-pager-bin` |
| Full terminal application | `crates/codegen/xai-grok-pager` |
| Renderer and terminal capabilities | `crates/codegen/xai-grok-pager-render` |
| Minimal terminal mode | `crates/codegen/xai-grok-pager-minimal` |
| Text editor | `crates/codegen/xai-ratatui-textarea` |
| Shared runtime | `crates/codegen/xai-agent` |
| Shell and process tools | `crates/codegen/xai-shell` |
| Session state | `crates/codegen/xai-chat-state` |

## Interface map

| Feature | Foundation path |
| --- | --- |
| Slash registry and commands | `crates/codegen/xai-grok-pager/src/slash` |
| Slash palette | `crates/codegen/xai-grok-pager/src/views/slash_dropdown.rs` |
| Composer | `crates/codegen/xai-grok-pager/src/views/prompt_widget` |
| Conversation blocks | `crates/codegen/xai-grok-pager/src/scrollback/blocks` |
| Mouse routing | `crates/codegen/xai-grok-pager/src/app/mouse.rs` |
| Terminal input and output | `crates/codegen/xai-grok-pager-render/src` |

## Live baseline

The imported binary built on macOS arm64. A live terminal check opened the
upstream welcome screen from the Bot repository. A bare `/` opened the eight-row
command palette. The palette showed command descriptions, selection, and
keyboard hints.

The check used the binary at `target/debug/xai-grok-pager`.

## Migration boundary

Bot changes the composition root before it changes the provider runtime. The
first source slice created the `bot` executable and Bot-facing command name.
It kept Grok-specific account, storage, endpoint, and provider terms intact.
The PTY and shared test helpers now resolve the `bot` artifact.

Later slices isolate the Grok runtime behind the common provider contract. The
Codex adapter then connects through app-server. Other adapters must use their
provider's supported authentication and protocol.

## Parity gates

Each migration slice must keep these checks green:

1. The focused Rust build and tests.
2. The slash palette and command dispatch.
3. A live streamed turn with thinking and tool blocks.
4. Session, settings, workflow, plugin, and media smoke tests.
5. Keyboard, mouse, minimal, and full-screen terminal checks.
