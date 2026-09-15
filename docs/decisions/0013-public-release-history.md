# 0013: Start the public repository from one release commit

- Status: Accepted
- Date: 2026-09-16

## Context

Bot started from the open-source Grok Build tree. Decision 0004 kept the
imported Git history during development. That history also holds old Bot work
and CI logs that are not needed to install or review the release.

## Decision

Keep the full development history in a private, archived repository. Start a
new public repository from the checked Bot 1.0.0 source tree with one signed
root commit and one signed release tag.

Keep the upstream source commit ID, license, copyright, attribution, and
third-party notices in the public tree. Do not copy old branches, tags, Git
commits, or CI logs into the public repository.

## Consequences

The public repository starts at the release source tree. The private archive
keeps the full audit trail. Later public changes use normal focused commits.
