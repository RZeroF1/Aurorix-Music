# Aurorix.Platform.Windows

This project owns Windows-specific gateways used by the shared Core facade and
the WinUI presentation host. Planned gateway areas include file pickers,
Windows locators, WASAPI, SMTC, notifications, and per-user IPC.

## Ownership

- The project is a Windows-only x64 class library.
- `Aurorix.Windows` owns WinUI views, ViewModels, and presentation state.
- Rust Core owns domain behavior, persistence, playback policy, queues, Sync,
  and the authoritative playback clock.
- This project must not contain business reducers, queue state, Sync logic,
  database migrations, or a second audio clock.
- Native gateway implementations and FFI bindings are introduced by later
  Gate 3 slices after their contracts are reviewed.

The project intentionally has no UI or Core reference at this topology
checkpoint.
