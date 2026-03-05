# Installer for aptos-move-tools (Windows)
# Usage:
#   irm https://raw.githubusercontent.com/gregnazario/aptos-move-tools/main/install.ps1 | iex
#   .\install.ps1 -Version v0.2.0
#   .\install.ps1 -Target aarch64-pc-windows-msvc -InstallDir C:\tools

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Target = "",
    [string]$InstallDir = "$HOME\.local\bin"
)

$ErrorActionPreference = "Stop"

$Repo = "gregnazario/aptos-move-tools"
$Binaries = @("move-suggest", "move-bounds-checker", "move1-to-move2")

function Say($msg) {
    Write-Host "install: $msg"
}

function Err($msg) {
    Write-Host "install: error: $msg" -ForegroundColor Red
    exit 1
}

# ---------- platform detection ----------

function Detect-Target {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch) {
        "X64"   { $script:Target = "x86_64-pc-windows-msvc" }
        "Arm64" { $script:Target = "aarch64-pc-windows-msvc" }
        default { Err "unsupported architecture: $arch" }
    }
}

# ---------- version resolution ----------

function Resolve-Version {
    if ($script:Version) { return }

    Say "fetching latest release version..."
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "aptos-move-tools-installer" }
        $script:Version = $release.tag_name
    } catch {
        Err "could not determine latest release version: $_"
    }

    if (-not $script:Version) {
        Err "could not determine latest release version"
    }
}

# ---------- checksum verification ----------

function Verify-Checksum($archivePath, $archiveName) {
    $checksumsUrl = "https://github.com/$Repo/releases/download/$Version/checksums-sha256.txt"

    Say "downloading checksums..."
    try {
        $checksums = Invoke-RestMethod -Uri $checksumsUrl -Headers @{ "User-Agent" = "aptos-move-tools-installer" }
    } catch {
        Err "could not download checksums: $_"
    }

    # Find the matching checksum line
    $expected = ""
    foreach ($line in $checksums -split "`n") {
        $line = $line.Trim()
        if ($line -match "^([a-f0-9]{64})\s+.*$([regex]::Escape($archiveName))$") {
            $expected = $Matches[1]
            break
        }
    }

    if (-not $expected) {
        Err "checksum for $archiveName not found in checksums-sha256.txt"
    }

    Say "verifying SHA-256 checksum..."
    $actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLower()

    if ($actual -ne $expected) {
        Err "checksum mismatch!`n  expected: $expected`n  actual:   $actual`nThis could indicate a corrupted download or a tampered file."
    }

    Say "checksum verified OK"
}

# ---------- main ----------

function Main {
    if (-not $Target) {
        Detect-Target
    }

    Resolve-Version

    Say "installing aptos-move-tools $Version for $Target"

    $archiveName = "aptos-move-tools-$Version-$Target.zip"
    $archiveUrl = "https://github.com/$Repo/releases/download/$Version/$archiveName"

    # Create temp dir
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "aptos-move-tools-install-$([System.Guid]::NewGuid().ToString('N').Substring(0,8))"
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        $archivePath = Join-Path $tmpDir $archiveName

        Say "downloading $archiveUrl..."
        try {
            Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath -UseBasicParsing
        } catch {
            Err "download failed: $_"
        }

        Verify-Checksum $archivePath $archiveName

        Say "extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force

        # The archive contains a directory named aptos-move-tools-{VERSION}-{TARGET}
        $extractedDir = Join-Path $tmpDir "aptos-move-tools-$Version-$Target"
        if (-not (Test-Path $extractedDir)) {
            Err "expected directory $extractedDir not found in archive"
        }

        # Create install directory
        if (-not (Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
            Say "created $InstallDir"
        }

        # Copy binaries
        foreach ($bin in $Binaries) {
            $src = Join-Path $extractedDir "$bin.exe"
            if (-not (Test-Path $src)) {
                Err "binary $bin.exe not found in archive"
            }
            Copy-Item -Path $src -Destination (Join-Path $InstallDir "$bin.exe") -Force
            Say "installed $(Join-Path $InstallDir "$bin.exe")"
        }

        # Print SHA-256 checksums of installed binaries
        Say ""
        Say "installed binary checksums (SHA-256):"
        foreach ($bin in $Binaries) {
            $binPath = Join-Path $InstallDir "$bin.exe"
            $hash = (Get-FileHash -Path $binPath -Algorithm SHA256).Hash.ToLower()
            Say "  ${bin}.exe: $hash"
        }

        # Check if install dir is in PATH
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $inPath = $false
        if ($userPath) {
            foreach ($p in $userPath -split ";") {
                if ($p.TrimEnd("\") -eq $InstallDir.TrimEnd("\")) {
                    $inPath = $true
                    break
                }
            }
        }

        Say ""
        if ($inPath) {
            Say "done! All binaries are ready to use."
        } else {
            Say "done! To add the binaries to your PATH, run:"
            Say ""
            Say "  `$currentPath = [Environment]::GetEnvironmentVariable('Path', 'User')"
            Say "  [Environment]::SetEnvironmentVariable('Path', `"`$currentPath;$InstallDir`", 'User')"
            Say ""
            Say "Then restart your terminal for the change to take effect."
        }
    } finally {
        # Clean up temp dir
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Main
