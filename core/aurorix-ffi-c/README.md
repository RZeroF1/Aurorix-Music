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

The C ABI handles, callbacks, asynchronous operations, and ownership wrappers
remain the G3-05 boundary. This crate does not expose Rust domain structs,
database rows, runtime handles, file descriptors, credentials, or audio state.
