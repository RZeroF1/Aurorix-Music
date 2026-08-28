# aurorix-runtime

Platform-neutral Core composition root and process-scoped `CoreHost` lifetime.
This crate wires existing storage and playback boundaries; it does not own
reducers, database migration definitions, identity, Sync semantics, realtime
audio rules, FFI, or platform output.
