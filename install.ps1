# install.ps1 - download and install the `prima` binary for Windows.
#
# The binary is fetched from the GitHub Releases of this repository and verified
# against the published SHA-256 checksum before installation.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/TickPoints/prima-language/main/install.ps1 | iex
#   .\install.ps1                       # install latest release to ~\.local\bin
#   .\install.ps1 -Version v0.3.0  # pin a version
#   .\install.ps1 -Dir $HOME\bin        # override the install directory
#
# Overridable via environment variables:
#   PRIMA_VERSION      release tag to install (default: latest)
#   PRIMA_TARGET       target triple, e.g. x86_64-pc-windows-msvc (default: detected)
#   PRIMA_INSTALL_DIR  install directory (default: $HOME\.local\bin)
#   PRIMA_REPO         "owner/repo" (default: TickPoints/prima-language)

[CmdletBinding()]
param(
    [string]$Version = $env:PRIMA_VERSION,
    [string]$Target = $env:PRIMA_TARGET,
    [string]$Dir = $env:PRIMA_INSTALL_DIR
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = if ($env:PRIMA_REPO) { $env:PRIMA_REPO } else { "TickPoints/prima-language" }
$InstallDir = if ($Dir) { $Dir } else { Join-Path $HOME ".local\bin" }
$Ext = ".exe"

# --- Architecture detection ---------------------------------------------------

function Get-PrimaArch {
    $arch = $env:PROCESSOR_ARCHITECTURE
    switch ($arch) {
        "AMD64" { "x86_64" }
        "ARM64" { "aarch64" }
        "x86"   { "x86" }
        default { throw "unsupported architecture: $arch" }
    }
}

if (-not $Target) {
    $Arch = Get-PrimaArch
    switch ($Arch) {
        "x86_64"  { $Target = "x86_64-pc-windows-msvc" }
        "aarch64" { $Target = "aarch64-pc-windows-msvc" }
        default   { throw "unsupported architecture: $Arch" }
    }
}

# --- Resolve the version ------------------------------------------------------

if (-not $Version) {
    Write-Host "==> resolving the latest release of $Repo"
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "prima-installer" }
    $Version = $release.tag_name
    if (-not $Version) {
        throw "could not determine the latest release of $Repo"
    }
}

$Artifact = "prima-${Version}-${Target}${Ext}"
$Url = "https://github.com/$Repo/releases/download/$Version/$Artifact"
$ShaUrl = "$Url.sha256"

# --- Download and verify ------------------------------------------------------

$Work = Join-Path ([IO.Path]::GetTempPath()) ("prima-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Work | Out-Null

try {
    Write-Host "==> downloading $Artifact"
    $BinaryPath = Join-Path $Work "prima$Ext"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $BinaryPath -UseBasicParsing
    }
    catch {
        throw "download failed - check that release '$Version' provides a '$Target' asset:`n  $Url"
    }

    $ShaPath = Join-Path $Work "prima.sha256"
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaPath -UseBasicParsing

    Write-Host "==> verifying SHA-256"
    $ExpectedLine = (Get-Content $ShaPath).Trim()
    $Expected = ($ExpectedLine -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 -Path $BinaryPath).Hash.ToLowerInvariant()
    if ($Expected -ne $Actual) {
        throw "checksum mismatch: expected $Expected, got $Actual"
    }

    # --- Install ---------------------------------------------------------------

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path $BinaryPath -Destination (Join-Path $InstallDir "prima$Ext") -Force
    Write-Host "==> installed to $(Join-Path $InstallDir "prima$Ext")"

    if ($env:PATH -notlike "*$InstallDir*") {
        Write-Host "==> note: $InstallDir is not on your PATH"
        Write-Host "    add it permanently, e.g.:"
        Write-Host "    setx PATH `"$env:PATH;$InstallDir`""
        Write-Host "    or for this session: `$env:PATH = `"$InstallDir;`$env:PATH`""
    }

    Write-Host "==> run \`prima --help\` to get started"
}
finally {
    Remove-Item -Path $Work -Recurse -Force -ErrorAction SilentlyContinue
}
