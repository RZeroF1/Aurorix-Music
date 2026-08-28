# Scripts

Repository and CI helpers live here. Scripts must be safe to run against
disposable test data and must not contain secrets.

- `verify-rust.ps1` runs the Rust Workspace formatter check, Clippy with
  warnings denied, and all workspace tests.
- `verify-windows-toolchain.ps1` checks the public x64 Windows build contract
  and reports optional FFI and installer tools without installing software.
- `generate-ffi.ps1` generates the public bootstrap protobuf descriptor from
  the pinned schema and tool version. `verify-ffi-generation.ps1` runs it twice
  and compares SHA-256 output. Both scripts fail closed when `protoc` is absent
  or the version is not the pinned one.
