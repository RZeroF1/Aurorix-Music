# Aurorix Architecture Overview

Aurorix is a local-first music platform for Windows and Android. Local cataloging
and local playback remain useful when no account, Provider, or Cloud server is
available. The product capability map is in
[`product-overview.md`](product-overview.md).

```text
Windows native client       Android native client
        |                            |
        +--- platform gateways/FFI ---+
                     |
              shared Rust Core
          /        |          \
   local SQLite  audio      extension ports
                     |
             optional Cloud sync
```

The diagram describes ownership and dependency direction. It does not claim
that every component is implemented or that every component runs in one
process.

## Component Ownership

| Component | Owns | Does not own |
|---|---|---|
| Rust Core | Domain values, local library, playback state, Sync state, and stable application commands | Native UI, OS lifecycle, Provider credentials, or hardware-specific APIs |
| Local SQLite | Device catalog, indexes, local projections, and device Sync outbox state | Cloud truth, raw Provider credentials, or open runtime resources |
| `aurorix-playback` | Queue policy, playback session, clock-facing snapshots, and the audio host port | Decoder internals, platform output, or a second UI queue |
| `aurorix-audio` | Decoder workers, bounded PCM buffers, realtime handoff, DSP/output boundaries | Account, Cloud, Provider authority, or UI state |
| Windows client | WinUI views, ViewModels, system integration, and Windows platform gateways | Domain reducers, playback queue, or Core persistence |
| Android client | Compose UI and service-owned MediaSession integration | A second CoreHost, playback engine, or authoritative queue |
| Provider adapters | External service capabilities through approved contracts | Core database, local IDs, raw credentials, or realtime audio access |
| Cloud server | Optional account-scoped replicated state and transport | Local files, device-local playback availability, or platform UI |
| Web control surface | Administrative/control views over approved APIs | A second domain implementation |

## Core Data Flows

Local playback follows this direction:

```text
Core command -> playback session -> source resolver -> decoder worker
             -> bounded PCM buffer -> realtime output boundary -> clock snapshot
```

The local library owns device-local assets and catalog projections. Replicated
user state uses portable identities and is resolved independently on each
device. Provider and Cloud availability may enrich or replicate user state but
must not be required for local cataloging or local playback.

## Audio Boundary

Audio control work, resource opening, decoding, buffering, seeking, graph
construction, and output changes run outside the realtime callback. The
realtime callback consumes prepared audio data and publishes a compact clock
sample; it does not perform file, network, database, Provider, UI, or blocking
work. Platform output format and latency are reported by the platform adapter
when those adapters are implemented.

`PresentationClock` is the shared source for playback progress, lyrics,
platform media state, and finalized play-history accounting. A discontinuity is
visible to consumers after seek, pause/resume restart, source transition,
underrun recovery, or output restart.

## Product Stages

| Stage | Public outcome |
|---|---|
| Gate 0 | Repository governance, Rust tooling, formatting, linting, and CI baseline |
| Gate 1 | Shared Rust model, local library/search/statistics, and local Sync foundations |
| Gate 2 | Offline local-file playback Core with deterministic PCM and realtime tests |
| Gate 3 | FFI and Windows local-only vertical slice |
| Gate 4 | Optional Cloud account, Sync transport, and recovery implementation |
| Gate 5 | Provider Host, SDK, package policy, and capability conformance |
| Gate 6 | Android service-owned playback host and native client |

## Public Design Principles

- Local media and device-local resources are not assumed to be globally
  addressable.
- User-visible UI state is driven by the shared Core rather than independent
  platform business implementations.
- The Core owns the queue, playback state, and presentation clock.
- Audio execution separates control work from realtime rendering work.
- External services are optional and can be unavailable without corrupting the
  local library, queue, or identities.
- Provider and extension permissions must correspond to enforceable host
  boundaries.
- Production security relies on authorization, key custody, input validation,
  and operational controls, never on hiding source code.

The public documentation intentionally omits private wire details, recovery
operations, deployment configuration, credentials, and security procedures.
