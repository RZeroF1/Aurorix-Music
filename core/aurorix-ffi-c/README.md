# aurorix-ffi-c

Public-safe transport bootstrap for the stable facade. The checked-in
schema/ffi-v1.proto is intentionally a small, versioned envelope used to
prove descriptor generation, protobuf round-tripping, closed oneof handling,
and size limits. It is not the complete Home, playback, library, or provider
facade.

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

The adapter's small raw-pointer implementation is isolated in this crate. The
workspace's `unsafe_code = "forbid"` policy remains unchanged for all domain,
runtime, storage, playback, and platform-neutral crates. This bootstrap does
not expose Rust domain structs, database rows, runtime handles, file
descriptors, credentials, or audio state, and it is not the complete facade.
