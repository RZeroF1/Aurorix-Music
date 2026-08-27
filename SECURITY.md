# Security Policy

Aurorix is developed in public, but production safety must never rely on source
code being private. Real credentials, production configuration, user data, and
unpatched vulnerability details do not belong in this repository.

## Reporting a Vulnerability

Do not open a public issue for a suspected vulnerability, secret exposure, or
reproducible exploit. When this repository is hosted on GitHub, use its Private
Vulnerability Reporting feature after maintainers enable it in repository
settings. Until a private reporting channel is configured, open only a minimal
public issue requesting a security contact; do not include exploit steps,
target details, logs, credentials, or proof-of-concept code.

## Scope

Report issues affecting the Rust Core, Windows client, Android client, Cloud
server, Provider runtime, build pipeline, or published SDKs. Include affected
version or commit, impact, safe reproduction conditions, and suggested
mitigation when available.

## Maintainer Requirements Before Public Release

- Enable private vulnerability reporting and define a response owner.
- Enable secret scanning and push protection on the repository host.
- Keep production deployment credentials outside Git and outside CI logs.
- Restrict production deployment credentials to trusted protected branches.
- Publish fixes, advisories, and acknowledgements only after coordinated review.
