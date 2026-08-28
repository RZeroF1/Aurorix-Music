# Windows Toolchain

This document records the public, reproducible build contract for the Gate 3
Windows client. The supported architecture for this phase is x64 only.
ARM64, x86, and AnyCPU are intentionally outside the build and release scope.

## Verification

From the repository root, run:

```powershell
.\scripts\verify-windows-toolchain.ps1
```

The checker validates the x64 project and solution declarations, the .NET CLI,
Visual Studio C++ x64 tools, MSVC, Windows SDK, Rust MSVC target, and the
restored Windows App SDK package. It also reports optional tools used by later
slices: `protoc`, `cbindgen`, and `csbindgen`. Missing optional tools are reported as `INFO` until a slice needs
them. Use `-RequireOptionalTools` when a local task is ready to require them.

The checker's output deliberately reports versions and capability states rather
than user names, credentials, machine names, or private paths. It does not
modify the repository or install software.

## Reproducible x64 commands

The startup baseline can be built with:

```powershell
dotnet restore apps/windows/Aurorix.Windows.slnx
dotnet build apps/windows/Aurorix.Windows.slnx --no-restore -c Debug -p:Platform=x64
dotnet build apps/windows/Aurorix.Windows/Aurorix.Windows.csproj `
  --no-restore -c Release -p:Platform=x64 -p:RuntimeIdentifier=win-x64
```

The Windows project is packaged x64 MSIX. The release pipeline produces both a
standalone MSIX package and an MSIX Bundle; signing certificates and private
keys are managed outside the repository.

Rust checks remain independent of the Windows host:

```powershell
.\scripts\verify-rust.ps1
```

## Toolchain contract

- Target framework: `net8.0-windows10.0.19041.0`.
- Minimum Windows target: `10.0.17763.0`, subject to the Windows App SDK
  support matrix used by the project.
- Rust toolchain: the version pinned by the repository's `rust-toolchain.toml`
  with `x86_64-pc-windows-msvc` installed.
- Native compilation: Visual Studio C++ x64 tools and a Windows 10 SDK that
  provides the project headers/libraries and x64 packaging tools.
- Windows App SDK: the version declared by the Windows project and restored by
  NuGet. The checker reports the actual local cache; it does not silently choose
  a different SDK version.
- FFI generation: `protoc`, `cbindgen`, and `csbindgen` are required only when
  the corresponding FFI slice starts. Their versions must be pinned in that
  slice before generated artifacts are committed.
- Packaging: MSIX and MSIX Bundle are the only Windows distribution outputs for
  this phase. The packaging slice must record signing and clean-machine install
  evidence before release claims are made.

## Evidence and limits

The verification script is a local preflight, not proof of a clean-machine
install, Visual Studio F5 launch, real WASAPI output, MSIX upgrade, or package
signing. Those require dedicated tests in the relevant Gate 3 slices.
The script also does not alter `AnyCPU` settings or `.gitignore`; x64-only
configuration remains owned by the Windows project and solution.

## Verified baseline

The following versions were observed on the maintainer Windows x64 machine on
2026-08-28. Re-run the checker after tool updates; this list is evidence of the
baseline, not a requirement to publish machine-specific paths.

- Visual Studio Community 2026 `18.9.12120.119`, with the C++ x64 workload.
- MSVC toolset `14.51.36231`.
- Windows SDK `10.0.26100.0`, including x64 `makeappx.exe` and `signtool.exe`.
- .NET CLI/SDK `10.0.400`; the application target remains `net8.0`.
- Windows App SDK NuGet package `2.4.0`.
- Rust `1.97.1`, Cargo `1.97.1`, target `x86_64-pc-windows-msvc`.

`protoc`, `cbindgen`, and `csbindgen` were not installed on that machine at this
checkpoint. They are intentionally pending until the FFI slice selects and pins
their versions.
