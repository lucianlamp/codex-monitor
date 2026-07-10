[CmdletBinding()]
param(
    [switch]$Yes,
    [switch]$InstallShim,
    [switch]$NoShim,
    [switch]$NoPath,
    [string]$Source,
    [switch]$SkipBuild,
    [switch]$BuildFromSource,
    [string]$ReleaseBase = $(if ($env:CDXM_INSTALL_RELEASE_BASE) { $env:CDXM_INSTALL_RELEASE_BASE } else { 'https://github.com/lucianlamp/codex-monitor/releases/latest/download' }),
    [string]$RepoUrl = $(if ($env:CDXM_INSTALL_REPO_URL) { $env:CDXM_INSTALL_REPO_URL } else { 'https://github.com/lucianlamp/codex-monitor' }),
    [string]$Ref = $(if ($env:CDXM_INSTALL_REF) { $env:CDXM_INSTALL_REF } else { 'main' }),
    [string]$InstallRoot = $(if ($env:CDXM_INSTALL_ROOT) { $env:CDXM_INSTALL_ROOT } else { Join-Path $HOME '.codex-monitor' }),
    [string]$SkillDir = $(if ($env:CDXM_SKILL_DIR) { $env:CDXM_SKILL_DIR } else { Join-Path $HOME '.codex\skills\codex-monitor' }),
    [string]$AgentsBin = $(if ($env:CDXM_AGENTS_BIN) { $env:CDXM_AGENTS_BIN } else { Join-Path $HOME '.agents\bin' })
)

$ErrorActionPreference = 'Stop'

$BinDir = Join-Path $InstallRoot 'bin'
$ShimTarget = Join-Path $AgentsBin 'codex.cmd'
$AppBridgeTarget = Join-Path $BinDir 'cdxm-codex-app-bridge.exe'
$AppBridgeEnvBackup = Join-Path $InstallRoot 'app-bridge-env.json'
$UserPathBackup = Join-Path $InstallRoot 'user-path-backup.json'
$RuntimeDir = Join-Path $InstallRoot 'runtime'
$LegacyRuntimePaths = @(
    (Join-Path $RuntimeDir 'codex-app-real.exe'),
    (Join-Path $RuntimeDir 'codex-code-mode-host.exe'),
    (Join-Path $RuntimeDir 'codex-command-runner.exe'),
    (Join-Path $RuntimeDir 'codex-windows-sandbox-setup.exe')
)
$TempDir = $null

function Confirm-CdxmStep {
    param(
        [string]$Prompt,
        [bool]$DefaultYes
    )

    if ($Yes) {
        return $DefaultYes
    }

    $suffix = if ($DefaultYes) { '[Y/n]' } else { '[y/N]' }
    $answer = Read-Host "$Prompt $suffix"
    if ([string]::IsNullOrWhiteSpace($answer)) {
        return $DefaultYes
    }
    return $answer -match '^(y|yes)$'
}

function Resolve-CdxmSource {
    if ($Source) {
        return (Resolve-Path $Source).Path
    }

    if ($PSScriptRoot -and
        (Test-Path (Join-Path $PSScriptRoot 'Cargo.toml')) -and
        (Test-Path (Join-Path $PSScriptRoot 'skills\codex-monitor'))) {
        return $PSScriptRoot
    }

    $script:TempDir = Join-Path ([IO.Path]::GetTempPath()) ("codex-monitor-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $script:TempDir | Out-Null
    $zip = Join-Path $script:TempDir 'codex-monitor.zip'
    $archiveUrl = "$RepoUrl/archive/refs/heads/$Ref.zip"
    Write-Host "Downloading codex-monitor from $archiveUrl"
    Invoke-WebRequest -Uri $archiveUrl -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath $script:TempDir -Force
    $sourceDir = Get-ChildItem -Path $script:TempDir -Directory |
        Where-Object { $_.Name -like 'codex-monitor-*' } |
        Select-Object -First 1
    if (-not $sourceDir) {
        throw "Downloaded archive did not contain a codex-monitor source directory"
    }
    return $sourceDir.FullName
}

function Install-CdxmPrebuilt {
    $archive = 'codex-monitor-x86_64-pc-windows-msvc.zip'
    $url = "$ReleaseBase/$archive"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $zip = Join-Path $tmp $archive
    Write-Host "Downloading prebuilt binaries: $url"
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip
    } catch {
        Write-Host "Prebuilt download failed; falling back to source build."
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        return $false
    }
    # Fetch the checksum with the same -OutFile mechanism the archive download
    # uses above: the in-memory ($resp.StatusCode -eq 200) form returns null
    # through GitHub's releases/latest/download redirect on Windows, which would
    # spuriously look like a missing sidecar and fail back to a source build.
    $checksum = $null
    $shaFile = "$zip.sha256"
    try {
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $shaFile -UseBasicParsing
        $checksum = (Get-Content -Raw $shaFile).Trim().ToLower()
    } catch {
        $checksum = $null
    }
    # Integrity is mandatory: a missing checksum is fail-safe (fall back to a
    # source build), never fail-open (install an unverified binary). Only a
    # checksum that is present AND mismatches is treated as active tampering.
    if ($null -eq $checksum) {
        Write-Host "No published checksum for $archive; refusing to install an unverified binary. Falling back to source build."
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        return $false
    }
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($checksum -ne $actual) {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        throw "Checksum mismatch for $archive (expected $checksum, got $actual)"
    }

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    # Extract only the expected top-level binaries by exact entry name, so a
    # tampered archive cannot drop extra files or traverse outside $BinDir
    # (Zip Slip). Mirrors the bash shim's named-member tar extraction.
    try { Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue } catch { }
    $allowed = @('codex-monitor.exe', 'cdxm.exe')
    $zipFile = [System.IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($entry in $zipFile.Entries) {
            if ($allowed -contains $entry.FullName) {
                $dest = Join-Path $BinDir $entry.Name
                [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dest, $true)
            }
        }
    } finally {
        $zipFile.Dispose()
    }
    if (-not (Test-Path (Join-Path $BinDir 'codex-monitor.exe')) -or
        -not (Test-Path (Join-Path $BinDir 'cdxm.exe'))) {
        Write-Host "Prebuilt archive did not contain the expected binaries; falling back to source build."
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        return $false
    }
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    Write-Host "Installed prebuilt cdxm to $(Join-Path $BinDir 'cdxm.exe')"
    return $true
}

function Install-CdxmBinaries {
    param([string]$SourceDir)

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    if ($SkipBuild) {
        Write-Host "Skipped binary install (-SkipBuild)."
        return
    }
    if (-not $BuildFromSource) {
        if (Install-CdxmPrebuilt) { return }
    }

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        throw "cargo is required to build codex-monitor from source. Install Rust/Cargo, then rerun this installer."
    }
    Write-Host "Note: building from source requires the Rust MSVC toolchain plus MSVC Build Tools."
    & cargo install --path $SourceDir --bins --force --root $InstallRoot
    if ($LASTEXITCODE -ne 0) {
        throw "cargo install failed with exit code $LASTEXITCODE"
    }
    Write-Host "Installed cdxm to $(Join-Path $BinDir 'cdxm.exe')"
}

function Install-CdxmSkill {
    param([string]$SourceDir)

    $sourceSkill = Join-Path $SourceDir 'skills\codex-monitor'
    if (-not (Test-Path $sourceSkill)) {
        throw "missing skill source: $sourceSkill"
    }

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $SkillDir) | Out-Null
    if (Test-Path $SkillDir) {
        Remove-Item -Recurse -Force $SkillDir
    }
    Copy-Item -Recurse -Force $sourceSkill $SkillDir
    Write-Host "Installed Codex skill to $SkillDir"
}

function Get-CodexEntrypointKind {
    param([string]$Path)

    if (-not (Test-Path $Path)) {
        return 'missing'
    }
    $content = Get-Content -Path $Path -Raw -ErrorAction SilentlyContinue
    if ($content -match 'CODEX_MONITOR_SHIM_WRAPPER=1') {
        return 'codex-monitor'
    }
    if ($content -match 'AGMSG_CODEX_SHIM_WRAPPER|agmsg monitor mode|codex-shim') {
        return 'agmsg'
    }
    return 'custom-or-unknown'
}

function Write-CdxmCodexCmd {
    # The Windows codex.cmd is a thin launcher that runs the shared bash shim
    # (skills/codex-monitor/scripts/codex-shim.sh) through Git Bash, so the
    # routing logic stays identical to macOS/Linux instead of being a separate
    # PowerShell reimplementation.
    $shimSource = Join-Path $SkillDir 'scripts\codex-shim.sh'
    if (-not (Test-Path $shimSource)) {
        throw "codex shim script not found at $shimSource; install the codex-monitor skill before the shim."
    }
    $shimBashPath = '/' + $shimSource.Substring(0, 1).ToLower() + ($shimSource.Substring(2) -replace '\\', '/')

    $cmd = @"
@echo off
rem codex-monitor shim (Windows) -- generated by codex-monitor install.ps1.
rem Routes codex through Git Bash into the shared bash shim
rem (skills/codex-monitor/scripts/codex-shim.sh) so behavior matches macOS/Linux.
set "WINPTY_SPAWNED=1"
set "CODEX_MONITOR_SHIM_WRAPPER=1"
set "CODEX_MONITOR_SHIM_TARGET=%~f0"
set "CDXM_BASH=%CDXM_GIT_BASH%"
if "%CDXM_BASH%"=="" set "CDXM_BASH=%GIT_BASH%"
if "%CDXM_BASH%"=="" set "CDXM_BASH=C:\Program Files\Git\bin\bash.exe"
"%CDXM_BASH%" -l "$shimBashPath" %*
"@

    New-Item -ItemType Directory -Force -Path $AgentsBin | Out-Null

    $refresh = $false
    if (Test-Path $ShimTarget) {
        $kind = Get-CodexEntrypointKind $ShimTarget
        if ($kind -eq 'codex-monitor') {
            # Already our shim: refresh in place (repairs a moved skill dir). No backup needed.
            $refresh = $true
        }
        else {
            # A foreign entrypoint (agmsg/custom) is installed. Reaching here means the
            # caller explicitly asked for the codex-monitor shim, so take over -- but keep
            # a timestamped backup so the previous entrypoint stays recoverable. A
            # conditional agmsg shim only app-server-binds projects whose delivery mode is
            # `monitor`, so it cannot guarantee cdxm a loaded thread; the codex-monitor
            # shim binds every interactive launch.
            if (-not (Confirm-CdxmStep "Replace existing '$kind' codex entrypoint at $ShimTarget with the codex-monitor shim (a backup will be kept)?" $true)) {
                Write-Host "Leaving existing codex entrypoint untouched at $ShimTarget (detected $kind)."
                Write-Host "  A conditional '$kind' shim may not make every interactive codex launch app-server-bound,"
                Write-Host "  so cdxm/codex-monitor may not find a loaded thread until the codex-monitor shim is installed."
                return
            }
            $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
            $backup = "$ShimTarget.bak-$stamp"
            Copy-Item -Path $ShimTarget -Destination $backup -Force
            Write-Host "Backed up existing '$kind' codex entrypoint to $backup"
        }
    }

    Set-Content -Path $ShimTarget -Value $cmd -Encoding ASCII
    if ($refresh) {
        Write-Host "Refreshed existing codex-monitor shim at $ShimTarget"
    }
    else {
        Write-Host "Installed Codex monitor shim to $ShimTarget"
    }
    Write-Host "  routes through Git Bash to $shimSource"
}

function Get-CdxmNormalizedPath {
    param(
        [AllowNull()][string]$Current,
        [string[]]$Preferred,
        [string[]]$Removed
    )

    $result = [Collections.Generic.List[string]]::new()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $removedSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $Removed) {
        if (-not [string]::IsNullOrWhiteSpace($entry)) {
            [void]$removedSet.Add($entry.Trim().TrimEnd('\', '/'))
        }
    }
    foreach ($entry in @($Preferred) + @($Current -split ';')) {
        if ([string]::IsNullOrWhiteSpace($entry)) { continue }
        $trimmed = $entry.Trim().TrimEnd('\', '/')
        if ($removedSet.Contains($trimmed)) { continue }
        if ($seen.Add($trimmed)) { $result.Add($entry.Trim()) }
    }
    return ($result -join ';')
}

function Repair-CdxmUserPath {
    $currentUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (Test-Path -LiteralPath $UserPathBackup)) {
        New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
        $temporaryBackup = "$UserPathBackup.tmp-$PID"
        $backup = [ordered]@{
            version = 1
            userPath = $currentUserPath
        }
        $backup | ConvertTo-Json | Set-Content -LiteralPath $temporaryBackup -Encoding utf8
        Move-Item -LiteralPath $temporaryBackup -Destination $UserPathBackup
    }

    $preferred = @($AgentsBin, $BinDir, (Join-Path $env:APPDATA 'npm'))
    $removed = @((Join-Path $env:LOCALAPPDATA 'OpenAI\Codex\bin'))
    $normalizedUserPath = Get-CdxmNormalizedPath -Current $currentUserPath -Preferred $preferred -Removed $removed
    $normalizedProcessPath = Get-CdxmNormalizedPath -Current $env:Path -Preferred $preferred -Removed $removed
    [Environment]::SetEnvironmentVariable('Path', $normalizedUserPath, 'User')
    $env:Path = $normalizedProcessPath
    Write-Host 'Normalized the user PATH for the codex-monitor shim and npm Codex CLI.'
}

function Set-ProcessEnvironmentValue {
    param([string]$Name, [AllowNull()][string]$Value)

    if ($null -eq $Value) {
        Remove-Item -LiteralPath "Env:$Name" -ErrorAction SilentlyContinue
    }
    else {
        Set-Item -LiteralPath "Env:$Name" -Value $Value
    }
}

function Test-CdxmPathEqual {
    param([AllowNull()][string]$Left, [AllowNull()][string]$Right)

    if ([string]::IsNullOrWhiteSpace($Left) -or [string]::IsNullOrWhiteSpace($Right)) {
        return $false
    }
    try {
        return [IO.Path]::GetFullPath($Left) -ieq [IO.Path]::GetFullPath($Right)
    }
    catch {
        return $false
    }
}

function Migrate-CdxmLegacyAppBridge {
    $legacyPaths = @($AppBridgeTarget) + @($LegacyRuntimePaths)
    $normalizedLegacyPaths = @($legacyPaths | ForEach-Object { [IO.Path]::GetFullPath($_) })
    $active = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.ExecutablePath -and
            $normalizedLegacyPaths -icontains [IO.Path]::GetFullPath($_.ExecutablePath)
        })
    if ($active.Count -gt 0) {
        $details = ($active | ForEach-Object { "PID $($_.ProcessId) ($($_.ExecutablePath))" }) -join ', '
        throw "Fully quit Codex App before migrating the legacy bridge (active: $details)"
    }

    $currentCli = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
    $owned = Test-CdxmPathEqual $currentCli $AppBridgeTarget
    if ($owned) {
        if (-not (Test-Path -LiteralPath $AppBridgeEnvBackup -PathType Leaf)) {
            throw "CODEX_CLI_PATH is the legacy bridge but its ownership file is missing: $AppBridgeEnvBackup"
        }
        $backup = Get-Content -LiteralPath $AppBridgeEnvBackup -Raw | ConvertFrom-Json
        if ($backup.version -ne 1 -or -not (Test-CdxmPathEqual $backup.bridgePath $AppBridgeTarget)) {
            throw "Legacy App bridge ownership file does not match this installation: $AppBridgeEnvBackup"
        }
        $previousCli = $backup.previousCodexCliPath
        $previousReal = $backup.previousCdxmRealCodex
        [Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $previousCli, 'User')
        [Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $previousReal, 'User')
        Set-ProcessEnvironmentValue 'CODEX_CLI_PATH' $previousCli
        Set-ProcessEnvironmentValue 'CDXM_REAL_CODEX' $previousReal
        Remove-Item -LiteralPath $AppBridgeEnvBackup -Force
        Write-Host 'Restored the user environment from the legacy App bridge backup.'
    }
    elseif (Test-Path -LiteralPath $AppBridgeEnvBackup -PathType Leaf) {
        Write-Warning 'Preserving CODEX_CLI_PATH because it is not owned by this codex-monitor installation.'
    }

    foreach ($legacyPath in $legacyPaths) {
        if (Test-Path -LiteralPath $legacyPath -PathType Leaf) {
            Remove-Item -LiteralPath $legacyPath -Force
        }
    }
    if ((Test-Path -LiteralPath $RuntimeDir -PathType Container) -and
        @(Get-ChildItem -LiteralPath $RuntimeDir -Force).Count -eq 0) {
        Remove-Item -LiteralPath $RuntimeDir -Force
    }
}

try {
    $SourceDir = Resolve-CdxmSource
    if (-not (Test-Path (Join-Path $SourceDir 'Cargo.toml'))) {
        throw "source does not look like codex-monitor: $SourceDir"
    }

    Write-Host 'codex-monitor Windows installer'
    Write-Host "source: $SourceDir"
    Write-Host "binary root: $InstallRoot"
    Write-Host "skill dir: $SkillDir"
    Write-Host "codex shim target: $ShimTarget"

    Migrate-CdxmLegacyAppBridge

    if (Confirm-CdxmStep "Install cdxm and codex-monitor binaries to $BinDir?" $true) {
        Install-CdxmBinaries $SourceDir
    } else {
        Write-Host 'Skipped binary install.'
    }

    if (Confirm-CdxmStep "Install Codex skill to $SkillDir?" $true) {
        Install-CdxmSkill $SourceDir
    } else {
        Write-Host 'Skipped skill install.'
    }

    $shouldInstallShim = $false
    if ($InstallShim) {
        $shouldInstallShim = $true
    } elseif ($NoShim) {
        $shouldInstallShim = $false
    } else {
        $shouldInstallShim = Confirm-CdxmStep "Install Codex shim at $ShimTarget?" $false
    }

    if ($shouldInstallShim) {
        Write-CdxmCodexCmd
    } else {
        Write-Host 'Skipped Codex shim install.'
    }

    if (-not $NoPath) {
        if (Confirm-CdxmStep "Normalize the user PATH for $AgentsBin, $BinDir, and the npm Codex CLI?" $true) {
            Repair-CdxmUserPath
        } else {
            Write-Host 'Skipped PATH update.'
        }
    } else {
        Write-Host 'Skipped PATH update.'
    }

    Write-Host ''
    Write-Host 'Done.'
    Write-Host 'Open a new PowerShell window, then verify:'
    Write-Host '  Get-Command cdxm'
    Write-Host '  Get-Command codex -All'
} finally {
    if ($TempDir -and (Test-Path $TempDir)) {
        Remove-Item -Recurse -Force $TempDir
    }
}
