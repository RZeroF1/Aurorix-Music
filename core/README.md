# Rust Core

The Cargo Workspace contains coarse-grained crates for model, application,
storage, library, playback, audio, synchronization, extensions, and FFI. They
are buildable placeholders until implementation begins; no domain behavior or
runtime capability is present.

The shared validation command is `./scripts/verify-rust.ps1` from the
repository root. The public architecture overview describes the dependency and
ownership boundaries that future implementation must preserve.
