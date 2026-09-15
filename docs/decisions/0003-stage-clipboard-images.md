# 0003: Stage clipboard images for the session

- Status: Accepted
- Date: 2026-09-11

## Context

Codex accepts local image input as a file path. The app-server can return from
`turn/start` before the Codex core reads the file. Bot must keep the file
private and available for this period. It must also remove files that the
session no longer needs.

## Decision

Bot reads clipboard files or image data through `arboard`. It decodes the image
and writes one PNG copy into a session-owned temporary directory. The directory
mode is `0700` on Unix. Each file mode is `0600` on Unix.

The interface shows only an image number and its dimensions. It does not show
the path or image bytes. Bot deletes an unsent image when the user removes it.
Bot deletes the complete temporary directory when the interface session closes.

## Consequences

- Submitted images remain available until the session closes.
- This avoids a race with the asynchronous Codex turn.
- Image conversion adds work during paste.
- Each provider gets a stable PNG input.
