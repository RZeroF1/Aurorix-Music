[CmdletBinding()]
param(
    [switch]$RequireOptionalTools
)

$ErrorActionPreference = "Stop"

function Write-Check {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [bool]$Passed,
        [Parameter(Mandatory)] [string]$Detail,
        [switch]$Optional
    )

    $status = if ($Passed) { "PASS" } elseif ($Optional -and -not $RequireOptionalTools) { "INFO" } else { "FAIL" }
    $color = switch ($status) { "PASS" { "Green" } "INFO" { "Yellow" } default { "Red" } }
    Write-Host ("[{0}] {1}: {2}" -f $status, $Name, $Detail) -ForegroundColor $color
    if ($status -eq "FAIL") { $script:failureCount++ }
}

function Get-CommandVersion {
    param([Parameter(Mandatory)] [string]$Command)

    $resolved = Get-Command $Command -ErrorAction SilentlyContinue
    if (-not $resolved) { return $null }

    try {
        $output = & $resolved.Source --version 2>&1 | Select-Object -First 1
        return [string]$output
    } catch {
        return "present (version query failed)"
    }
}

function Get-SdkVersions {
    param([Parameter(Mandatory)] [string]$Root)

    if (-not (Test-Path -LiteralPath $Root)) { return @() }
    return @(Get-ChildItem -LiteralPath $Root -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^10\.0\.\d+\.0$' } |
        Select-Object -ExpandProperty Name)
}

function Get-DisplayValue {
    param(
        [AllowNull()] [object]$Value,
        [Parameter(Mandatory)] [string]$Fallback
    )

    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        return $Fallback
    }

    return [string]$Value
}

$failureCount = 0
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot ".." )).Path
$windowsProject = Join-Path $repoRoot "apps\windows\Aurorix.Windows\Aurorix.Windows.csproj"
$windowsSolution = Join-Path $repoRoot "apps\windows\Aurorix.Windows.slnx"
$sdkRoot = "C:\Program Files (x86)\Windows Kits\10"

Write-Host "Aurorix Music Windows x64 toolchain verification" -ForegroundColor Cyan
Write-Host "Scope: x64 only; ARM64, x86, and AnyCPU are intentionally unsupported."
Write-Host ""

if (-not (Test-Path -LiteralPath $windowsProject)) {
    Write-Check "Windows project" $false "apps/windows/Aurorix.Windows/Aurorix.Windows.csproj was not found."
} else {
    $projectText = Get-Content -Raw -LiteralPath $windowsProject
    $projectXml = [xml]$projectText
    $declaredWindowsAppSdk = $projectXml.Project.ItemGroup.PackageReference |
        Where-Object { $_.Include -eq "Microsoft.WindowsAppSDK" } |
        Select-Object -ExpandProperty Version -First 1
    Write-Check "Windows project" $true "project found."
    Write-Check "x64 project declaration" ($projectText -match '<Platforms>x64</Platforms>') "Platforms contains x64."
    Write-Check "no unsupported project platforms" ($projectText -notmatch '<Platforms>[^<]*(AnyCPU|ARM64|x86)') "project platform list contains no AnyCPU, ARM64, or x86 entry."
    Write-Check "x64 platform target" ($projectText -match '<PlatformTarget>x64</PlatformTarget>') "PlatformTarget is x64."
    Write-Check "win-x64 runtime" ($projectText -match '<RuntimeIdentifiers>win-x64</RuntimeIdentifiers>') "RuntimeIdentifiers contains win-x64."
    Write-Check "unpackaged baseline" ($projectText -match '<WindowsPackageType>None</WindowsPackageType>') "WindowsPackageType is None."
}

if (-not (Test-Path -LiteralPath $windowsSolution)) {
    Write-Check "Windows solution" $false "apps/windows/Aurorix.Windows.slnx was not found."
} else {
    $solutionText = Get-Content -Raw -LiteralPath $windowsSolution
    Write-Check "Windows solution" $true "solution found."
    Write-Check "x64 solution configuration" ($solutionText -match '<Platform Name="x64"') "solution declares x64."
    Write-Check "no non-x64 solution configurations" ($solutionText -notmatch 'Any CPU|AnyCPU|ARM64|x86') "no unsupported solution platform is declared."
}

$dotnetVersion = Get-CommandVersion "dotnet"
Write-Check ".NET CLI" ($null -ne $dotnetVersion) (Get-DisplayValue $dotnetVersion "dotnet was not found.")
if ($dotnetVersion) {
    $sdkLines = @(dotnet --list-sdks 2>$null)
    $hasNet8 = $sdkLines | Where-Object { $_ -match '^8\.0\.' }
    $hasNet10 = $sdkLines | Where-Object { $_ -match '^10\.0\.' }
    $sdkMajors = (($sdkLines -replace '\s+\[.*$','' | ForEach-Object { ($_ -split '\.')[0..1] -join '.' } | Sort-Object -Unique) -join ', ')
    Write-Check ".NET SDK for project" ($hasNet8 -or $hasNet10) "project targets net8.0; available SDK major(s): $sdkMajors."
    Write-Host "  SDKs: $($sdkLines -join '; ')"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path -LiteralPath $vswhere) {
    $vsJson = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json 2>$null
    $vs = $vsJson | ConvertFrom-Json | Select-Object -First 1
    Write-Check "Visual Studio C++ x64 tools" ($null -ne $vs -and $vs.isComplete) $(if ($vs) { "$($vs.displayName) $($vs.installationVersion)" } else { "no complete installation with VC x64 tools was found." })
    $vcRoot = if ($vs) { Join-Path $vs.installationPath "VC\Tools\MSVC" } else { $null }
    $vcVersions = if ($vcRoot) { @(Get-ChildItem -LiteralPath $vcRoot -Directory -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name) } else { @() }
    Write-Check "MSVC toolset" ($vcVersions.Count -gt 0) $(if ($vcVersions.Count) { $vcVersions -join ', ' } else { "no MSVC toolset directory was found." })
} else {
    Write-Check "Visual Studio C++ x64 tools" $false "vswhere.exe was not found." 
}

$sdkIncludeVersions = Get-SdkVersions (Join-Path $sdkRoot "Include")
$sdkBinVersions = Get-SdkVersions (Join-Path $sdkRoot "bin")
$hasSdk = $sdkIncludeVersions.Count -gt 0 -and $sdkBinVersions.Count -gt 0
Write-Check "Windows SDK" $hasSdk $(if ($hasSdk) { "Include: $($sdkIncludeVersions -join ', '); bin: $($sdkBinVersions -join ', ')." } else { "Windows Kits 10 Include/bin version directories were not found." })
$sdkToolVersion = $sdkBinVersions | Sort-Object { [version]$_ } -Descending | Select-Object -First 1
$makeAppx = if ($sdkToolVersion) { Join-Path $sdkRoot "bin\$sdkToolVersion\x64\makeappx.exe" } else { $null }
$signTool = if ($sdkToolVersion) { Join-Path $sdkRoot "bin\$sdkToolVersion\x64\signtool.exe" } else { $null }
$hasMsixTools = $makeAppx -and $signTool -and (Test-Path -LiteralPath $makeAppx) -and (Test-Path -LiteralPath $signTool)
Write-Check "MSIX tooling" $hasMsixTools $(if ($sdkToolVersion) { "Windows SDK $sdkToolVersion x64 makeappx.exe and signtool.exe are available." } else { "no Windows SDK tool version is available." }) -Optional

$cargoVersion = Get-CommandVersion "cargo"
$rustcVersion = Get-CommandVersion "rustc"
$rustupCommand = Get-Command rustup -ErrorAction SilentlyContinue
$rustupTarget = if ($rustupCommand) { (& $rustupCommand.Source show active-toolchain 2>$null | Select-Object -First 1) } else { $null }
$hasRustMsvc = if ($rustupCommand) { (& $rustupCommand.Source target list --installed 2>$null | Select-String -SimpleMatch "x86_64-pc-windows-msvc") } else { $null }
Write-Check "Cargo" ($null -ne $cargoVersion) (Get-DisplayValue $cargoVersion "cargo was not found.")
Write-Check "Rust compiler" ($null -ne $rustcVersion) (Get-DisplayValue $rustcVersion "rustc was not found.")
Write-Check "Rust MSVC target" ($null -ne $hasRustMsvc) $(if ($rustupTarget) { "$rustupTarget" } else { "x86_64-pc-windows-msvc target was not found." })

$nugetGlobals = if ($dotnetVersion) { dotnet nuget locals global-packages --list 2>$null } else { @() }
$nugetGlobalPath = ($nugetGlobals | Select-String -Pattern '^global-packages:\s*(.+)$' | ForEach-Object { $_.Matches[0].Groups[1].Value.Trim() } | Select-Object -First 1)
$windowsPackage = if ($nugetGlobalPath) { Join-Path $nugetGlobalPath "microsoft.windowsappsdk" } else { $null }
$windowsPackageVersions = if (Test-Path -LiteralPath $windowsPackage) { @(Get-ChildItem -LiteralPath $windowsPackage -Directory | Select-Object -ExpandProperty Name) } else { @() }
$hasDeclaredWindowsAppSdk = $null -ne $declaredWindowsAppSdk -and $declaredWindowsAppSdk -ne ""
$hasRequestedWindowsAppSdk = $hasDeclaredWindowsAppSdk -and ($windowsPackageVersions -contains $declaredWindowsAppSdk)
Write-Check "Windows App SDK declaration" $hasDeclaredWindowsAppSdk $(if ($hasDeclaredWindowsAppSdk) { "project declares version $declaredWindowsAppSdk." } else { "Microsoft.WindowsAppSDK PackageReference was not found." })
Write-Check "Windows App SDK restore" $hasRequestedWindowsAppSdk $(if ($windowsPackageVersions.Count) { "declared: $declaredWindowsAppSdk; restored version(s): $($windowsPackageVersions -join ', ')." } else { "declared: $declaredWindowsAppSdk; package is not in the current NuGet global package cache; run dotnet restore first." })

foreach ($optionalTool in @("protoc", "cbindgen", "csbindgen", "iscc", "makensis")) {
    $toolVersion = Get-CommandVersion $optionalTool
    Write-Check "Optional tool $optionalTool" ($null -ne $toolVersion) (Get-DisplayValue $toolVersion "$optionalTool was not found on PATH.") -Optional
}

Write-Host ""
if ($failureCount -gt 0) {
    Write-Host "Verification failed with $failureCount required check(s)." -ForegroundColor Red
    exit 1
}

Write-Host "Required toolchain checks passed. Optional tools may remain pending for later Gate 3 slices." -ForegroundColor Green
