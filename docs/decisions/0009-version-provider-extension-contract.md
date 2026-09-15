# Version the provider extension contract

- Status: Accepted
- Date: 2026-09-13

## Context

The imported pager kept its extension capability matrix inside the TUI. This made the interface, instead of the provider boundary, decide which hooks, plugins, skills, workflows, and MCP actions a provider supports.

An unversioned capability list can also accept incompatible future data. Bot must reject unsupported contracts before it shows controls that the provider cannot run.

## Decision

Keep the extension contract in `bot-provider`. Version 1 declares provider-neutral capabilities for hooks, plugins, plugin updates, marketplaces, skills, workflows, MCP servers, MCP changes, and managed connectors.

Validate the contract version and capability dependencies when Bot reads a serialized contract. The TUI derives its extension tabs and actions from this contract. Provider adapters still own the operations and data behind each capability.

## Consequences

- The pager does not own a separate provider capability matrix.
- New providers must declare supported extension capabilities at the provider boundary.
- A plugin update requires plugin support.
- MCP changes and managed connectors require MCP server support.
- Bot rejects an unsupported contract version instead of guessing its meaning.
