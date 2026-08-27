# Contributing to Aurorix

Thank you for helping build Aurorix. The project is at repository-bootstrap
stage; the public source tree is intentionally a scaffold rather than a working
player or server.

## Before You Start

Read `README.md`, `docs/architecture-overview.md`, `docs/publication-policy.md`,
and `SECURITY.md`. Do not submit secrets, live service information, proprietary
media, real account data, or copied third-party source code.

## Contributions Welcome Now

- Documentation that is explicitly safe for public publication.
- Reproducible tooling, formatting, licensing, and repository hygiene.
- Small, scoped implementation work after a maintainer has opened or approved
  the matching task.
- Tests with synthetic or redistributable fixtures.

## Design-Sensitive Changes

Do not independently implement a new identity conversion, Sync conflict rule,
account security policy, Provider permission, FFI lifetime rule, audio realtime
behavior, database migration policy, or public wire format. Open a proposal
first. Maintainers will decide whether the work needs a public contract, a
private design review, or both.

## Pull Requests

- Keep each pull request focused on one concern.
- State the user-visible behavior and test evidence.
- Update public documentation only when the interface is intentionally public
  and stable enough to support.
- Do not expose private planning material in a PR description, screenshot,
  commit message, test fixture, or generated artifact.
- Never add a generated file unless its generator, input, and regeneration
  command are also defined.

## Licensing and Third-Party Work

Contributed code must be compatible with this repository's `LICENSE`. Do not
copy code from a third-party project merely because it is publicly visible;
confirm its license and preserve required notices.
