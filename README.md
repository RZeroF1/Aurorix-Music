# Aurorix Music

Aurorix is an open-source, local-first music platform with a shared Rust core,
native Windows and Android clients, and an optional self-hosted Cloud server.

## Status

Gate 0 is complete and Gate 1 local Core foundations are implemented. The
repository is not yet a usable end-user player or server: Gate 2 offline audio,
the Windows and Android hosts, platform audio adapters, FFI, Provider host, and
Cloud service are still pending.

Gate 1 currently includes typed domain models, application contracts, local
catalog/search/statistics persistence, and local Sync outbox/cursor primitives.
These capabilities have deterministic Rust tests, but they do not imply a
complete product UI, real hardware output, Cloud transport, or production
recovery environment.

Run the local Rust quality checks with:

```powershell
.\scripts\verify-rust.ps1
```

## Components

- `core/` - shared Rust Workspace for the domain and application Core.
- `apps/windows/` - planned WinUI 3 native client.
- `apps/android/` - planned Android and Compose native client.
- `server/` - planned ASP.NET Core Cloud server for optional account sync.
- `sdk/` - future public SDK surfaces.
- `web/` - planned administrative and control surfaces.
- `docs/` - intentionally publishable documentation only.

## Public Documentation

- `docs/README.md` - public documentation index.
- `docs/architecture-overview.md` - high-level architecture and boundaries.
- `docs/product-overview.md` - public product capability map and stage status.
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
