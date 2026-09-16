# 0018 — Report Bot lifecycle state to Herdr

Status: Accepted

Date: 2026-09-17

## Decision

Bot reports its session identity and lifecycle state when it runs in a Herdr pane. The integration is active only when `HERDR_ENV=1` and `HERDR_PANE_ID` is present. Bot uses `HERDR_BIN_PATH` when Herdr provides it.

Bot owns the `herdr:bot` source and the `bot` agent label. It reports `idle`, `working`, and `blocked`. Herdr derives `done` from a working-to-idle transition in an unfocused pane. Bot releases its report when the application exits.

The Bot session ID is the restore identity. Herdr can resume it with `bot --resume <session-id>` after native Bot support is available in Herdr.

## Reason

Screen matching alone cannot identify every lifecycle change. Bot has direct access to authentication, permission, turn, and session state, so it can report those facts without parsing its own rendered output.

The Herdr pane commands are the supported integration boundary. Bot does not read Herdr state files, use private sockets, or expose provider credentials. The integration does nothing outside Herdr.

## Result

Herdr can show Bot as idle, working, blocked, or done and can wait for a Bot turn to finish. A bundled Herdr screen manifest can provide a fallback if a lifecycle report is late or missing.

Herdr must register the `bot` agent before it accepts the reports. The upstream request is tracked in [Herdr Discussion #4260](https://github.com/herdrdev/herdr/discussions/4260).
