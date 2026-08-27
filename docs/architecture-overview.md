# Aurorix Architecture Overview

Aurorix is a local-first music application. It is designed so local cataloging
and local playback remain useful when no account, Provider, or Cloud server is
available.

```text
Windows native client        Android native client
        |                            |
        +-------- shared Rust Core --+
                     |
          optional Cloud sync service
```

## Components

- **Rust Core** owns portable domain behavior, local library orchestration,
  playback state, synchronization behavior, and extension-facing semantics.
- **Windows client** will use native Windows UI and platform integrations.
- **Android client** will use native Android UI and a service-owned playback
  lifecycle for background media playback.
- **Cloud server** is optional. It will provide account-scoped synchronization
  and does not own a user's local files or local playback capability.
- **Provider integrations** are replaceable adapters for external music
  services. They are deliberately isolated from the Core's local database and
  realtime audio execution.

## Public Design Principles

- Local media and device-local resources are not assumed to be globally
  addressable.
- User-visible UI state is driven by the shared Core rather than independent
  platform business implementations.
- Audio execution separates control work from realtime rendering work.
- External services are optional and can be unavailable without corrupting the
  local library or queue.
- Production security relies on enforceable authorization, key custody, input
  validation, and operational controls, never on hiding this source code.

This overview deliberately omits unreleased wire formats, recovery procedures,
service configuration, and security operations. Those details are published
only when they have a stable public compatibility and disclosure policy.
