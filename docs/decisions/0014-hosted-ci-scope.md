# Decision 0014: Keep hosted CI within its runner limits

## Context

The imported Grok Build workspace has large test binaries. A full workspace
test run filled the disk on a standard GitHub Linux runner before the tests
could start. Ubuntu's default Protobuf compiler also failed imported fixtures
that pass with Protobuf 29.3.

## Decision

Hosted CI checks formatting, secrets, the installer, and full workspace Clippy.
It tests Bot's provider crates and the Protobuf builder with a pinned Protobuf
29.3 setup. Run full workspace tests on a development machine before a release.
The five-target release workflow, not every CI push, builds production binaries.

## Consequences

Hosted CI reports useful results without filling its runner disk. It does not
replace full workspace tests. A release still needs those tests and the complete
platform build matrix before publication.
