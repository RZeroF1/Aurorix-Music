# Aurorix Music

Aurorix is an open-source, local-first music platform with a shared Rust core,
native Windows and Android clients, and an optional self-hosted Cloud server.

## Status

This is the public repository scaffold. It is not a usable player or server
yet: the Rust Core, Windows application, Android application, audio backend,
Provider host, and Cloud service have not been implemented.

## Components

- `core/` - shared Rust domain and application Core.
- `apps/windows/` - planned WinUI 3 native client.
- `apps/android/` - planned Android and Compose native client.
- `server/` - planned ASP.NET Core Cloud server for optional account sync.
- `sdk/` - future public SDK surfaces.
- `docs/` - intentionally publishable documentation only.

## Public Documentation

- `docs/README.md` - public documentation index.
- `docs/architecture-overview.md` - high-level architecture and boundaries.
- `docs/publication-policy.md` - what is appropriate to publish here.
- `CONTRIBUTING.md` - contribution and design-change policy.
- `SECURITY.md` - vulnerability reporting and security expectations.

Detailed working specifications, risk reviews, recovery procedures, test
operations, and other maintainer material are deliberately not tracked in this
public repository. Their absence is intentional; it does not mean a feature is
implemented or approved.

## Security

The Server may be open source. Its production security must not depend on
source secrecy: deployment secrets, private network configuration, tokens, and
real operational data are never committed. See `SECURITY.md` and
`docs/publication-policy.md`.
