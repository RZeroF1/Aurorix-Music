[CmdletBinding()]
param(
    [string]$ProtoPath,
    [switch]$AllowLocalSchema
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("aurorix-ffi-generation-" + [guid]::NewGuid().ToString("N"))
$firstOutput = Join-Path $tempRoot "first"
$secondOutput = Join-Path $tempRoot "second"

try {
    New-Item -ItemType Directory -Path $firstOutput, $secondOutput -Force | Out-Null
    $generator = Join-Path $repoRoot "scripts\generate-ffi.ps1"
    $commonArguments = @{}
    if (-not [string]::IsNullOrWhiteSpace($ProtoPath)) {
        $commonArguments["ProtoPath"] = $ProtoPath
    }
    if ($AllowLocalSchema) {
        $commonArguments["AllowLocalSchema"] = $true
    }

    & $generator @commonArguments -OutputRoot $firstOutput
    if ($LASTEXITCODE -ne 0) {
        throw "first descriptor generation failed with exit code $LASTEXITCODE."
    }
    & $generator @commonArguments -OutputRoot $secondOutput
    if ($LASTEXITCODE -ne 0) {
        throw "second descriptor generation failed with exit code $LASTEXITCODE."
    }

    $firstDescriptor = Get-ChildItem -LiteralPath $firstOutput -Filter "*.descriptor.pb" -File | Select-Object -First 1
    $secondDescriptor = Get-ChildItem -LiteralPath $secondOutput -Filter "*.descriptor.pb" -File | Select-Object -First 1
    if (-not $firstDescriptor -or -not $secondDescriptor) {
        throw "descriptor generation did not produce a descriptor file."
    }
    $firstHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $firstDescriptor.FullName).Hash
    $secondHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $secondDescriptor.FullName).Hash
    if ($firstHash -ne $secondHash) {
        throw "descriptor generation is not reproducible: $firstHash != $secondHash."
    }

    Write-Host "[PASS] FFI descriptor generation is reproducible."
    Write-Host "SHA-256: $($firstHash.ToLowerInvariant())"
}
finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
