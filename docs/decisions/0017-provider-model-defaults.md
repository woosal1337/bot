# 0017 — Keep model defaults with their provider

Status: Accepted

Date: 2026-09-16

## Decision

Bot saves model defaults by provider. A provider adapter reads only its own choice when it starts a new conversation. It checks that the model still exists in the provider's catalog. If the model is gone or the preference cannot be read, the adapter uses the provider's default. A resumed conversation keeps its own model.

## Reason

The inherited Grok model setting does not control Codex threads. Saving a Codex model there could also change a Grok conversation. Provider-specific storage keeps those choices apart without changing the provider's account or thread records.

## Result

The Settings model picker still offers a live model switch. Saving its new-thread default is a separate step. Bot reports a save error if that step fails. Clearing the choice restores the provider's default for later conversations.
