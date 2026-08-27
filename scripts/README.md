# Scripts

Repository and CI helpers live here. Scripts must be safe to run against
disposable test data and must not contain secrets.

- `verify-rust.ps1` runs the Rust Workspace formatter check, Clippy with
  warnings denied, and all workspace tests.
