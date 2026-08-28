[CmdletBinding()]
param(
    [string]$ProtoPath,
    [string]$OutputRoot,
    [switch]$AllowLocalSchema
)

$ErrorActionPreference = "Stop"

function Get-ConfigValue {
    param(
        [Parameter(Mandatory)] [string]$Text,
        [Parameter(Mandatory)] [string]$Name
    )

    $pattern = '(?m)^\s*' + [regex]::Escape($Name) + '\s*=\s*"([^"]+)"\s*$'
    $match = [regex]::Match($Text, $pattern)
    if (-not $match.Success) {
        throw "ffi-generation.toml is missing '$Name'."
    }
    return $match.Groups[1].Value
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ffiRoot = Join-Path $repoRoot "core\aurorix-ffi-c"
$configPath = Join-Path $ffiRoot "ffi-generation.toml"
$configText = Get-Content -Raw -LiteralPath $configPath

if ([string]::IsNullOrWhiteSpace($ProtoPath)) {
    $ProtoPath = Join-Path $ffiRoot (Get-ConfigValue -Text $configText -Name "proto")
}

$resolvedProto = (Resolve-Path -LiteralPath $ProtoPath).Path
if (-not $AllowLocalSchema -and $resolvedProto -match "(^|[\\/])private([\\/]|$)") {
    throw "A private schema requires -AllowLocalSchema and must never be committed with generated outputs."
}
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $ffiRoot "generated"
}

$resolvedOutput = [System.IO.Path]::GetFullPath($OutputRoot)
$descriptorName = Split-Path -Leaf (Get-ConfigValue -Text $configText -Name "descriptor")
$hashName = Split-Path -Leaf (Get-ConfigValue -Text $configText -Name "descriptor_sha256")
$descriptorPath = Join-Path $resolvedOutput $descriptorName
$hashPath = Join-Path $resolvedOutput $hashName
$protoc = Get-Command protoc -ErrorAction SilentlyContinue
if (-not $protoc) {
    throw "protoc was not found on PATH. Install the pinned version before running FFI generation."
}

$expectedProtoc = Get-ConfigValue -Text $configText -Name "protoc_version"
$actualProtoc = (& $protoc.Source --version 2>&1 | Select-Object -First 1).ToString().Trim()
if ($actualProtoc -ne "libprotoc $expectedProtoc") {
    throw "protoc version mismatch: expected libprotoc $expectedProtoc, found '$actualProtoc'."
}

New-Item -ItemType Directory -Path $resolvedOutput -Force | Out-Null
$protoDirectory = Split-Path -Parent $resolvedProto
$protoName = Split-Path -Leaf $resolvedProto
$protocArguments = @(
    "--proto_path=$protoDirectory"
    "--descriptor_set_out=$descriptorPath"
    "--include_imports"
    "--include_source_info"
    $protoName
)
& $protoc.Source @protocArguments
if ($LASTEXITCODE -ne 0) {
    throw "protoc failed with exit code $LASTEXITCODE."
}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $descriptorPath).Hash.ToLowerInvariant()
Set-Content -LiteralPath $hashPath -Value $hash -Encoding ascii

Write-Host "Generated descriptor: $descriptorPath"
Write-Host "SHA-256: $hash"
Write-Host "Rust DTO, C header, and C# NativeMethods remain deferred until the reviewed facade schema and G3-05 ABI are approved."
