# aurorix-ffi-c

Public-safe transport projections for the stable facade. The checked-in
schema/ffi-v1.proto keeps the G3-05 versioned envelope and adds a bounded
Home snapshot/command value projection. Home values are carried through the
existing `ExtensionRequestV1` arm using namespace `aurorix.home` and payload
schema version `1`; this is a contract/value mapping, not Home business logic
or a new native ABI operation.

ffi-generation.toml records the pinned generator contract. Run
scripts/generate-ffi.ps1 from the repository root after installing the
required pinned tools. Descriptor and binding outputs belong under ignored
generated/; they are never hand-edited. The full reviewed schema may only be
provided as an explicit local maintainer input after publication review.

The G3-05 bootstrap ABI is declared in `include/aurorix_ffi.h` and exports
opaque client, operation, and subscription handles. Commands and queries accept
borrowed bounded envelopes and complete on a worker thread. Callback payloads
are borrowed until the callback returns; returned diagnostics are Rust-owned
and must be released with `aurorix_buffer_free_v1`.

`aurorix_subscription_cancel_v1` and client shutdown establish a no-more-callback
fence before resources are released. Operation cancellation is cooperative and
reports the cancellation-before-commit classification used by the managed
wrapper. Every exported entry point catches Rust panics before returning to C.

`transport::HomeSnapshot` carries stable card IDs, recent-played,
recently-added, favorites, continue-playback, quick entries, optional
discover/recommendations, source/status metadata, and the observed event
sequence. `transport::HomeCommandRequest` carries bounded route/query targets
for host-to-facade command routing without executing them. Snapshot payloads
are bounded by the 1 MiB message limit and 256 KiB event limit; nested values,
card counts, identities, unknown fields, unknown enum values, and unknown
oneof arms are fail-closed.

`src/transport.rs` contains the reviewed hand-written Rust mapping because
generated outputs are not checked in. `ffi-generation.toml` remains the pinned
tool contract; generated descriptors/bindings belong under ignored
`generated/` and are never hand-edited. This public slice does not claim that
the Core facade is connected to Home queries, Provider runtime, playback, or
WinUI.

The adapter's small raw-pointer implementation is isolated in this crate. The
workspace's `unsafe_code = "forbid"` policy remains unchanged for all domain,
runtime, storage, playback, and platform-neutral crates. This bootstrap does
not expose Rust domain structs, database rows, runtime handles, file
descriptors, credentials, or audio state.
