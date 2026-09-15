# Codex plan mode

Date: 2026-09-12

## Decision

Map the shared ACP `plan` and `default` modes to Codex's native collaboration modes.
Each turn sends the selected mode, model, and reasoning effort through `turn/start`.
Send `developer_instructions: null` to keep Codex's built-in mode instructions.
This field needs the experimental API capability already used for terminal cleanup.

Keep the mode in each adapter session. New and resumed sessions start in Default mode.
Send Default explicitly on later turns so a prior Plan turn cannot remain active by accident.
Reject unknown modes and changes during an active turn.

The UI shows a pending message until the adapter confirms the change.
On failure, clear the pending state and show the error. Keep the last confirmed mode.
If `/plan <message>` cannot change mode, do not send the message in the old mode.
Render proposed plans as visible messages during streaming and history replay, not as collapsed thinking.

Plan mode sets the agent's work instructions. It is not a filesystem security boundary.
The provider's permission and sandbox policies still control tool access.

## Checks

Protocol tests check the exact mode payload and the null instruction field.
An adapter test sends Plan and Default turns through the shared Codex fixture.
UI tests check pending feedback, confirmed feedback, and failed mode changes.
A live terminal test left the test file unchanged in Plan mode, then wrote the exact six bytes in Default.
Codex's session record confirms both modes with the selected model, effort, and working directory.
A second terminal check showed the proposed plan during streaming and after resume, with thinking collapsed.

Contract: [Codex app-server](https://learn.chatgpt.com/docs/app-server), checked against the installed CLI's generated JSON schema.
