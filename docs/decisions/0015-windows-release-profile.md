# Decision 0015: Limit Windows release link work

## Context

A hosted Windows release build was still compiling after 92 minutes when its
runner was cancelled. The job log showed no Rust error. The binary and imported
workspace make the build expensive on the standard Windows runner.

## Decision

Keep the optimized `release-dist` profile on every target. Set its LTO value to
`off` only for the Windows package job. Linux and macOS keep thin LTO.

## Consequences

Windows no longer spends build time on cross-crate LTO. Its runtime performance
may differ slightly from the other packages. Windows remains a release gate:
the executable must build and run on the hosted runner before publication.
