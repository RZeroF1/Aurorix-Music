# Aurorix.Platform.Windows

This project owns Windows-specific gateways used by the shared Core facade and
the WinUI presentation host. Gateway areas include file pickers, Windows
locators, WASAPI, SMTC, notifications, and per-user IPC.

## Ownership

- The project is a Windows-only x64 class library.
- `Aurorix.Windows` owns WinUI views, ViewModels, and presentation state.
- Rust Core owns domain behavior, persistence, playback policy, queues, Sync,
  and the authoritative playback clock.
- This project must not contain business reducers, queue state, Sync logic,
  database migrations, or a second audio clock.
- Native gateway implementations and FFI bindings are introduced by later
  Gate 3 slices after their contracts are reviewed.

The project intentionally has no UI or Core reference. The file-picker and
library-gateway implementation keeps that topology: a WinUI host provides an
`IWindowsPickerAdapter`, while this project validates the transient selection,
normalizes it, and returns typed platform values.

## Local locator gateway

`WindowsMediaLocator` is the persistent Windows-side locator value. Its path is
normalized with `WindowsPathNormalizer` and may carry an observed
`WindowsFileIdentity` containing only a volume serial and file index. Native
file handles are opened and closed inside `WindowsFileIdentityProvider`; no
handle or lease is represented by a public type.

`WindowsLibraryGateway.StartScan` requires a non-empty
`WindowsScanRequest.Roots` collection. Roots come from an explicit user picker
selection, and the scanner never chooses a drive or performs a default
full-disk scan. Enumeration runs on a worker task, emits deterministic,
sequence-numbered events, and supports cancellation through
`WindowsScanSession.Cancel` or `TryCancelScan`. Permission denied, missing,
reparse-point, and other I/O outcomes are typed in `WindowsScanIssue` rather
than exposed as exception text.

Relinking is deliberately conservative. `WindowsRelinkMatcher` checks file
identity first. If identity is unavailable, a size plus the configured quick
hash may produce candidates; multiple candidates remain ambiguous for Core or
the user to resolve. A title, artist, or path string is never used to merge a
catalog identity.

The locator and scan types have no JSON/protobuf/Sync serializers here. They
must not be copied into FFI DTOs, Sync operations, or durable playback intent.
Core remains responsible for catalog identity, scan state persistence, and
preserving recordings, favorites, playlists, and play facts when an asset is
missing.

The current implementation is x64-only (`win-x64`). A native Windows picker
adapter and Core/FFI projection are separate slices and are intentionally not
implemented in this project.

## G3-09 Windows system media control boundary

`WindowsMediaControlContracts.cs` defines the public-safe typed boundary for a
real System Media Transport Controls (SMTC) host:

- `WindowsPlaybackSnapshot` mirrors the Core playback state, opaque item ID,
  state/buffer revisions, and the Core-owned `WindowsPresentationClock`.
- `WindowsMediaMetadata` carries display metadata and optional duration only;
  it never carries a path, URL, credential, provider handle, or native lease.
- `WindowsSmTcProjection` contains playback status, metadata, timeline,
  seek range, playback rate, clock epoch, and enabled system actions.
- `WindowsMediaControlAction` maps Play, Pause, Stop, Previous, Next, and Seek
  to one `WindowsCorePlaybackCommand` through
  `IWindowsCorePlaybackCommandSink`; Play becomes Core `Resume` when the
  latest snapshot is paused and Core `Play` otherwise.
- `WindowsMediaControlHostEvent` carries ordered host/device lifecycle events
  through `IWindowsMediaControlLifecycleSink`.

`WindowsSmTcProjectionGateway` is a bounded latest-value gate. A state or
metadata change is applied immediately. Progress samples are ordered using the
sampling source's sequence and Core clock epoch, merged into the one latest
snapshot, and applied only when the host explicitly calls `FlushProgress`.
There is no platform queue, background worker, wall clock, local position
counter, playback reducer, or UI timer-owned state. Request IDs, clock values,
sample ordering, and lifecycle timestamps come from the Core-facing host or
native adapter.

The project intentionally does not reference Windows Runtime SMTC types. A
later native host can translate `WindowsSmTcProjection` into
`SystemMediaTransportControls`, `SystemMediaTransportControlsTimelineProperties`,
and `MediaPlaybackItem` metadata. The included `FakeWindowsMediaControlSink`
records projections, Core commands, and lifecycle events for deterministic
contract probes.

No standalone test project existed in this directory when G3-09 started. Per
the scope restriction, this slice does not create a project outside the
existing solution or alter the solution file. Verification is therefore
limited to the project build and source-level contract probes; native SMTC
registration, callback delivery, packaged WinUI integration, and Windows
Runtime behavior remain unverified until a host adapter is introduced.
