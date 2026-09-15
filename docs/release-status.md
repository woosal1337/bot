# Release status

## Bot 1.0.0 scope

Bot 1.0.0 is a Codex-first release. The full terminal interface, provider-owned
authentication, model and effort selection, permissions, images, streaming,
reasoning, tools, MCP servers, skills, hooks, usage, and conversation actions are
connected to the supported Codex app-server protocol.

The release pipeline must pass on every supported target before it publishes a
GitHub release. Release archives include checksums, license notices, and build
provenance. The install scripts download these archives and do not compile Rust.

## Remaining work

1. Finish isolated multi-account profiles. Provider and session switching work,
   but one provider account per provider process is the current safe boundary.
2. Add the Claude provider through its supported local interface.
3. Replace the experimental Grok runtime boundary with a provider adapter that
   uses only supported public interfaces.
4. Add native code signing for Windows and Developer ID signing and notarization
   for macOS. The first archives have checksums and build attestations, but they
   do not carry operating-system publisher signatures.

## Release rule

Publish Bot 1.0.0 only after the five-target Linux, macOS, and Windows matrix
passes, and the signed release commit and tag are verified.

Do not describe an unfinished provider as supported. Keep an unavailable action
hidden or show a clear provider limit. Do not emulate a missing operation through
a private endpoint or by reading provider credentials.
