# Version command ownership

- Status: Accepted
- Date: 2026-09-13

## Context

The imported workflow catalog used `workflowSource` as both type and owner. Codex skills used separate flat provider and owner fields. The TUI could not validate one common command identity before it treated provider data as a skill or workflow.

## Decision

Add a versioned command ownership record in `bot-provider`. Each owned command declares its provider, owner, and kind. Version 1 supports skills and workflows.

Codex skill commands declare their provider and scope or plugin owner. Grok workflow commands declare Grok as the provider and their built-in, project, or user source as the owner. Keep existing provider fields during migration, but require the versioned record before the workflow UI accepts a command.

Reject unknown ownership versions, empty identities, owner mismatches, and kind mismatches. Provider adapters still execute the command through their supported protocol.

## Consequences

- A provider skill cannot become a workflow by adding one legacy field.
- A workflow cannot claim a built-in run unless its validated owner is `builtin`.
- Codex plugin skill collisions keep their plugin owner after name qualification.
- Future command kinds require a contract version change.
