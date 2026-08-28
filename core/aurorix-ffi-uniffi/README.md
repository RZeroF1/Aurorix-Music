# aurorix-ffi-uniffi

UniFFI mapping remains deferred until the reviewed facade schema is public and
the C transport contract is complete. The crate intentionally has no generated
bindings yet; Kotlin records must be generated from the same reviewed
descriptor and may not become an independent DTO source of truth.

Cancellation, callback, and CoreHost lifetime semantics belong to G3-05 and
must continue to follow the shared facade contract.
