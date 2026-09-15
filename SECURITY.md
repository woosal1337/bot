# Security policy

## Report a vulnerability

Use the repository's **Security** tab and select **Report a vulnerability**.
Do not include credentials, tokens, private prompts, or private source code in a
public issue.

The maintainers will acknowledge a complete report within seven days. They will
coordinate a fix and disclosure when the report is valid.

## Supported versions

Security fixes apply to the latest release.

## Credential boundary

Codex keeps its login and credential storage in the official Codex client. The
experimental imported Grok runtime keeps its existing Grok credential store.
Bot does not transfer credentials between providers. A report that concerns a
provider account or service can also need disclosure to that provider.
