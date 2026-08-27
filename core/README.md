# Rust Core

The Cargo Workspace contains coarse-grained crates for model, application,
storage, library, playback, audio, synchronization, extensions, and FFI. Gate 1
implemented the model, application contracts, local library/storage, local
statistics, and local Sync foundations. Playback/audio runtime behavior,
platform hosts, extensions, and FFI remain incomplete until their later gates.

The shared validation command is `./scripts/verify-rust.ps1` from the
repository root. The public architecture overview describes the dependency and
ownership boundaries that future implementation must preserve.
