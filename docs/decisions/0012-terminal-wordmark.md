# 0012 — Use the terminal wordmark in the README

## Decision

Bot uses a custom, seven-row BOT wordmark on the full welcome screen. Narrow screens use a five-row version.

The README uses an SVG built from the seven-row grid. A square cursor follows the T as a terminal cue.

The SVG uses the welcome panel's #141414 background, #333333 border, and #6D6D6D letters.

## Reason

A single-letter badge did not name the product. Separate abstract icons did not express the terminal interface.

The wordmark lets people identify Bot in both places without a vendor mark. Matching colors make the README image look like the launch screen.

## Consequence

Changes to either wordmark tier need a welcome snapshot. The SVG geometry must match the full text grid.
