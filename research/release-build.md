# Release build references

Bot uses hosted Linux, macOS, and Windows runners for tagged binary releases.
GitHub lists the current runner labels and their disk capacity in its
[hosted runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Build Linux archives on Ubuntu 22.04 to avoid linking against a newer host's
glibc when that is not required.

GitHub checks signed commit and tag identities against signing keys registered
with the account. The repository release gate follows the
[commit signature verification guide](https://docs.github.com/en/authentication/managing-commit-signature-verification/about-commit-signature-verification).

The release workflow attests each archive and the checksum list. People can
check an archive's origin with the
[artifact attestation verification guide](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).
They can check a downloaded file against a published release with the
[release integrity guide](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/verify-release-integrity).

GitHub exposes private security reports for public repositories through its
[vulnerability reporting setting](https://docs.github.com/en/rest/repos/repos#enable-private-vulnerability-reporting-for-a-repository).
Enable that setting when the public repository goes live.

GitHub build attestations do not replace native publisher signatures. Apple
documents [Developer ID signing and notarization](https://developer.apple.com/documentation/technologyoverviews/distribution)
for software distributed outside its store. Microsoft documents
[SmartScreen reputation for unsigned downloads](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).
