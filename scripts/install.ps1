[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$Repo = "rnldsalili/envswitch",
    [string]$InstallDir,
    [string]$AppRoot
)

$ErrorActionPreference = "Stop"
$CommandName = "envswitch.cmd"

function Write-Info {
    param([string]$Message)
    Write-Host "info: $Message"
}

function Write-Success {
    param([string]$Message)
    Write-Host "success: $Message"
}

function Write-Failure {
    param([string]$Message)
    throw $Message
}

function Test-Command {
    param([string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Test-GhAuthenticated {
    return (Test-Command "gh") -and $(
        & gh auth status *> $null
        $LASTEXITCODE -eq 0
    )
}

function Assert-Windows {
    $isWindows = [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [Runtime.InteropServices.OSPlatform]::Windows
    )

    if (-not $isWindows) {
        Write-Failure "unsupported OS for this installer. Use scripts/install.sh on macOS, Linux, or WSL."
    }
}

function Assert-Repo {
    param([string]$Value)

    if ($Value -notmatch "^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$") {
        Write-Failure "invalid repository format: $Value. Expected owner/repo."
    }
}

function Get-GitHubHeaders {
    param([string]$Accept = "application/vnd.github+json")

    $headers = @{
        Accept = $Accept
    }

    if ($env:GITHUB_TOKEN) {
        $headers.Authorization = "Bearer $env:GITHUB_TOKEN"
    }

    return $headers
}

function Get-LatestRelease {
    param([string]$Repository)

    if (Test-GhAuthenticated) {
        $json = & gh api "repos/$Repository/releases/latest" 2> $null
        if ($LASTEXITCODE -eq 0 -and $json) {
            return ($json | ConvertFrom-Json)
        }
    }

    try {
        return Invoke-RestMethod `
            -Uri "https://api.github.com/repos/$Repository/releases/latest" `
            -Headers (Get-GitHubHeaders)
    }
    catch {
        Write-Failure "no GitHub Release found for $Repository, or access is denied"
    }
}

function Get-BunPath {
    $command = Get-Command "bun" -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $fallback = Join-Path $env:USERPROFILE ".bun\bin\bun.exe"
    if (Test-Path $fallback) {
        return $fallback
    }

    return $null
}

function Install-Bun {
    Write-Info "Bun is not installed; installing Bun"
    powershell -NoProfile -ExecutionPolicy Bypass -Command "irm bun.sh/install.ps1 | iex"

    $bunBin = Join-Path $env:USERPROFILE ".bun\bin"
    if (Test-Path $bunBin) {
        $env:Path = "$bunBin;$env:Path"
    }
}

function Ensure-Bun {
    $bun = Get-BunPath
    if (-not $bun) {
        Install-Bun
        $bun = Get-BunPath
    }

    if (-not $bun) {
        Write-Failure "Bun installation finished, but bun was not found on PATH"
    }

    Write-Info "using Bun at $bun"
    & $bun --version *> $null
    return $bun
}

function Save-Archive {
    param(
        [object]$Release,
        [string]$ArchiveName,
        [string]$TempDir,
        [string]$Repository
    )

    if (Test-GhAuthenticated) {
        Write-Info "downloading $ArchiveName with GitHub CLI"
        & gh release download $Release.tag_name --repo $Repository --pattern $ArchiveName --dir $TempDir *> $null
        if ($LASTEXITCODE -eq 0) {
            return
        }

        Write-Info "GitHub CLI download did not find $ArchiveName; trying direct download"
    }
    else {
        Write-Info "downloading $ArchiveName"
    }

    $asset = $Release.assets | Where-Object { $_.name -eq $ArchiveName } | Select-Object -First 1
    if (-not $asset) {
        Write-Failure "matching asset not found: $ArchiveName"
    }

    Invoke-WebRequest `
        -Uri $asset.url `
        -Headers (Get-GitHubHeaders -Accept "application/octet-stream") `
        -OutFile (Join-Path $TempDir $ArchiveName)
}

function Install-Source {
    param(
        [string]$ArchivePath,
        [string]$TempDir,
        [string]$TargetAppRoot,
        [string]$TargetAppDir,
        [string]$Bun
    )

    if (-not (Test-Command "tar")) {
        Write-Failure "tar is required to extract the envswitch source archive"
    }

    $staging = Join-Path $TempDir "staging"
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    New-Item -ItemType Directory -Path $TargetAppRoot -Force | Out-Null

    Write-Info "extracting source archive"
    & tar -xzf $ArchivePath -C $staging
    if ($LASTEXITCODE -ne 0) {
        Write-Failure "failed to extract $ArchivePath"
    }

    $sourceDir = Join-Path $staging "envswitch"
    if (-not (Test-Path (Join-Path $sourceDir "package.json")) -or -not (Test-Path (Join-Path $sourceDir "src\cli.ts"))) {
        Write-Failure "archive did not contain an envswitch source package"
    }

    Write-Info "installing production dependencies"
    Push-Location $sourceDir
    try {
        & $Bun install --production --frozen-lockfile
        if ($LASTEXITCODE -ne 0) {
            Write-Failure "bun install failed"
        }
    }
    finally {
        Pop-Location
    }

    if ($PSCmdlet.ShouldProcess($TargetAppDir, "Install envswitch source")) {
        if (Test-Path $TargetAppDir) {
            Remove-Item -Path $TargetAppDir -Recurse -Force
        }
        Move-Item -Path $sourceDir -Destination $TargetAppDir
    }
}

function Add-ToUserPath {
    param([string]$Directory)

    $current = [System.Environment]::GetEnvironmentVariable("Path", "User")
    $normalizedDirectory = $Directory.TrimEnd([char]'\')
    $entries = @()
    if ($current) {
        $entries = $current -split ";"
    }

    $exists = $entries | Where-Object { $_ -and ([string]::Equals($_.TrimEnd([char]'\'), $normalizedDirectory, [System.StringComparison]::OrdinalIgnoreCase)) }
    if ($exists) {
        return
    }

    $newPath = if ($current) { "$current;$Directory" } else { $Directory }
    if ($PSCmdlet.ShouldProcess("User PATH", "Add $Directory")) {
        [System.Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        $env:Path = "$Directory;$env:Path"
        Write-Info "added $Directory to the user PATH"
    }
}

function Write-Launcher {
    param(
        [string]$TargetInstallDir,
        [string]$TargetAppDir
    )

    New-Item -ItemType Directory -Path $TargetInstallDir -Force | Out-Null
    $targetLauncher = Join-Path $TargetInstallDir $CommandName
    $cliPath = Join-Path $TargetAppDir "src\cli.ts"

    $launcher = @"
@echo off
setlocal
where bun >NUL 2>NUL
if %ERRORLEVEL% EQU 0 (
  bun "$cliPath" %*
  exit /b %ERRORLEVEL%
)
if exist "%USERPROFILE%\.bun\bin\bun.exe" (
  "%USERPROFILE%\.bun\bin\bun.exe" "$cliPath" %*
  exit /b %ERRORLEVEL%
)
echo envswitch: bun is not installed or not on PATH 1>&2
exit /b 127
"@

    if ($PSCmdlet.ShouldProcess($targetLauncher, "Install envswitch launcher")) {
        Set-Content -Path $targetLauncher -Value $launcher -Encoding ASCII
    }

    Add-ToUserPath -Directory $TargetInstallDir
    if (-not (Test-Path $targetLauncher)) {
        Write-Failure "launcher was not installed: $targetLauncher"
    }
    return $targetLauncher
}

Assert-Windows
Assert-Repo $Repo

if (-not $env:LOCALAPPDATA) {
    Write-Failure "LOCALAPPDATA is not set. Pass -InstallDir and -AppRoot explicitly."
}

if (-not $AppRoot) {
    $AppRoot = Join-Path $env:LOCALAPPDATA "Programs\envswitch"
}

if (-not $InstallDir) {
    $InstallDir = Join-Path $AppRoot "bin"
}

$appDir = Join-Path $AppRoot "current"

if ($WhatIfPreference) {
    Write-Host "What if: Install Bun if missing"
    Write-Host "What if: Download the latest envswitch source release asset from $Repo"
    Write-Host "What if: Install envswitch source to $appDir"
    Write-Host "What if: Install envswitch launcher to $(Join-Path $InstallDir $CommandName)"
    Write-Host "What if: Add $InstallDir to the user PATH if missing"
    return
}

$bun = Ensure-Bun
$release = Get-LatestRelease -Repository $Repo
$version = $release.tag_name -replace "^v", ""

if ($version -notmatch "^[a-zA-Z0-9][a-zA-Z0-9._-]*$") {
    Write-Failure "latest release tag is invalid: $($release.tag_name)"
}

$archive = "envswitch_${version}_source.tar.gz"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())

try {
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    Save-Archive -Release $release -ArchiveName $archive -TempDir $tempDir -Repository $Repo
    Install-Source -ArchivePath (Join-Path $tempDir $archive) -TempDir $tempDir -TargetAppRoot $AppRoot -TargetAppDir $appDir -Bun $bun
    Set-Content -Path (Join-Path $appDir ".version") -Value $release.tag_name
    $launcher = Write-Launcher -TargetInstallDir $InstallDir -TargetAppDir $appDir

    Write-Success "installed envswitch $($release.tag_name)"
    Write-Info "source: $appDir"
    Write-Info "launcher: $launcher"
    Write-Info "restart your terminal, or run this command now:"
    Write-Info "`$env:Path = `"$InstallDir;`$env:Path`""
    Write-Info "then run 'envswitch --help' to verify the installation"
}
finally {
    if (Test-Path $tempDir) {
        Remove-Item -Path $tempDir -Recurse -Force
    }
}
