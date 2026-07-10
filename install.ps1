[CmdletBinding()]
param(
    [switch]$Yes,
    [switch]$InstallShim,
    [switch]$NoShim,
    [switch]$InstallAppBridge,
    [switch]$RemoveAppBridge,
    [switch]$NoPath,
    [string]$RealCodexPath,
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
$RuntimeDir = Join-Path $InstallRoot 'runtime'
$ManagedRealCodex = Join-Path $RuntimeDir 'codex-app-real.exe'
$ManagedCodeModeHost = Join-Path $RuntimeDir 'codex-code-mode-host.exe'
$ManagedCommandRunner = Join-Path $RuntimeDir 'codex-command-runner.exe'
$ManagedSandboxSetup = Join-Path $RuntimeDir 'codex-windows-sandbox-setup.exe'
$TempDir = $null

if ($InstallAppBridge.IsPresent -and $RemoveAppBridge.IsPresent) {
    throw '-InstallAppBridge and -RemoveAppBridge are mutually exclusive.'
}

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
    Write-Host "Installed Codex App bridge to $AppBridgeTarget"
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

function Add-UserPathEntry {
    param([string]$PathEntry)

    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @()
    if ($current) {
        $parts = $current -split ';' | Where-Object { $_ }
    }
    if ($parts | Where-Object { $_ -ieq $PathEntry }) {
        Write-Host "PATH already contains $PathEntry"
        return
    }
    $newPath = if ($current) { "$PathEntry;$current" } else { $PathEntry }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$PathEntry;$env:Path"
    Write-Host "Added $PathEntry to the user PATH"
}

function Test-CdxmOwnedAppBridge {
    param([string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $false
    }
    try {
        return [IO.Path]::GetFullPath($Value) -ieq [IO.Path]::GetFullPath($AppBridgeTarget)
    }
    catch {
        return $false
    }
}

function Resolve-RealCodexPath {
    param([string]$ExplicitPath)

    $candidates = @()
    if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
        $candidates += $ExplicitPath
    }
    try {
        $package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop |
            Sort-Object Version -Descending |
            Select-Object -First 1
        if ($package.InstallLocation) {
            $candidates += (Join-Path $package.InstallLocation 'app\resources\codex.exe')
        }
    }
    catch {
        # The saved and per-user Codex paths below remain the fallback.
    }

    $saved = [Environment]::GetEnvironmentVariable('CDXM_REAL_CODEX', 'User')
    if (-not [string]::IsNullOrWhiteSpace($saved)) {
        $candidates += $saved
    }
    $candidates += (Join-Path $HOME 'AppData\Local\OpenAI\Codex\bin\codex.exe')

    foreach ($candidate in $candidates) {
        if ([string]::IsNullOrWhiteSpace($candidate) -or -not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            continue
        }
        $resolved = (Resolve-Path -LiteralPath $candidate).Path
        if ([IO.Path]::GetFullPath($resolved) -ieq [IO.Path]::GetFullPath($AppBridgeTarget)) {
            continue
        }
        $codeModeHost = Join-Path (Split-Path -Parent $resolved) 'codex-code-mode-host.exe'
        if (-not (Test-Path -LiteralPath $codeModeHost -PathType Leaf)) {
            if (-not [string]::IsNullOrWhiteSpace($ExplicitPath) -and
                [IO.Path]::GetFullPath($resolved) -ieq [IO.Path]::GetFullPath($ExplicitPath)) {
                throw "The selected Codex executable has no sibling codex-code-mode-host.exe: $resolved"
            }
            continue
        }
        return $resolved
    }
    throw 'Could not find a complete Codex App runtime. Pass -RealCodexPath <path-to-codex.exe> from a directory that also contains codex-code-mode-host.exe.'
}

function Resolve-CodexRuntimeCompanionPath {
    param(
        [string]$ResolvedRealCodexPath,
        [string]$FileName,
        [bool]$Required
    )

    $candidate = Join-Path (Split-Path -Parent $ResolvedRealCodexPath) $FileName
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        return (Resolve-Path -LiteralPath $candidate).Path
    }
    if ($Required) {
        throw "Required Codex runtime companion is missing next to $ResolvedRealCodexPath`: $FileName"
    }
    return $null
}

function Copy-CdxmRuntimeFile {
    param(
        [string]$SourcePath,
        [string]$DestinationPath
    )

    if ([string]::IsNullOrWhiteSpace($SourcePath)) {
        return
    }
    if ([IO.Path]::GetFullPath($SourcePath) -ieq [IO.Path]::GetFullPath($DestinationPath)) {
        return
    }
    if (Test-Path -LiteralPath $DestinationPath -PathType Leaf) {
        $sourceHash = (Get-FileHash -LiteralPath $SourcePath -Algorithm SHA256).Hash
        $destinationHash = (Get-FileHash -LiteralPath $DestinationPath -Algorithm SHA256).Hash
        if ($sourceHash -eq $destinationHash) {
            return
        }
    }
    Copy-Item -LiteralPath $SourcePath -Destination $DestinationPath -Force
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

function Enable-CdxmAppBridge {
    param([string]$ResolvedRealCodexPath)

    if (-not (Test-Path -LiteralPath $AppBridgeTarget -PathType Leaf)) {
        throw "Codex App bridge binary is missing: $AppBridgeTarget"
    }
    $codeModeHost = Resolve-CodexRuntimeCompanionPath $ResolvedRealCodexPath 'codex-code-mode-host.exe' $true
    $commandRunner = Resolve-CodexRuntimeCompanionPath $ResolvedRealCodexPath 'codex-command-runner.exe' $false
    $sandboxSetup = Resolve-CodexRuntimeCompanionPath $ResolvedRealCodexPath 'codex-windows-sandbox-setup.exe' $false
    New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null
    # Packaged WindowsApps executables are readable but cannot be launched by
    # the external bridge. Keep a private copy of Codex and its sibling runtime
    # executables so features such as code-mode tools resolve the matching host.
    Copy-CdxmRuntimeFile $ResolvedRealCodexPath $ManagedRealCodex
    Copy-CdxmRuntimeFile $codeModeHost $ManagedCodeModeHost
    Copy-CdxmRuntimeFile $commandRunner $ManagedCommandRunner
    Copy-CdxmRuntimeFile $sandboxSetup $ManagedSandboxSetup
    if (-not (Test-Path -LiteralPath $AppBridgeEnvBackup)) {
        Write-Host 'Preserving the current user environment before enabling the Codex App bridge.'
        $backup = [ordered]@{
            version = 1
            previousCodexCliPath = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
            previousCdxmRealCodex = [Environment]::GetEnvironmentVariable('CDXM_REAL_CODEX', 'User')
            bridgePath = $AppBridgeTarget
        }
        New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
        $backup | ConvertTo-Json | Set-Content -LiteralPath $AppBridgeEnvBackup -Encoding utf8
    }

    [Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $AppBridgeTarget, 'User')
    [Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $ManagedRealCodex, 'User')
    Set-ProcessEnvironmentValue 'CODEX_CLI_PATH' $AppBridgeTarget
    Set-ProcessEnvironmentValue 'CDXM_REAL_CODEX' $ManagedRealCodex
    Write-Host "Enabled Codex App bridge: CODEX_CLI_PATH=$AppBridgeTarget"
    Write-Host "Real Codex executable: $ManagedRealCodex"
    Write-Host "Codex code-mode host: $ManagedCodeModeHost"
    Write-Host 'Codex App must be restarted before the shared app-server is active.'
}

function Disable-CdxmAppBridge {
    $current = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
    if (-not (Test-CdxmOwnedAppBridge $current)) {
        Write-Warning 'Preserving the current user environment because CODEX_CLI_PATH is not owned by codex-monitor.'
        return
    }

    $previousCli = $null
    $previousReal = $null
    if (Test-Path -LiteralPath $AppBridgeEnvBackup) {
        $backup = Get-Content -LiteralPath $AppBridgeEnvBackup -Raw | ConvertFrom-Json
        $previousCli = $backup.previousCodexCliPath
        $previousReal = $backup.previousCdxmRealCodex
    }
    [Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $previousCli, 'User')
    [Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $previousReal, 'User')
    Set-ProcessEnvironmentValue 'CODEX_CLI_PATH' $previousCli
    Set-ProcessEnvironmentValue 'CDXM_REAL_CODEX' $previousReal
    Remove-Item -LiteralPath $AppBridgeEnvBackup -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $ManagedRealCodex -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $ManagedCodeModeHost -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $ManagedCommandRunner -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $ManagedSandboxSetup -Force -ErrorAction SilentlyContinue
    Write-Host 'Disabled Codex App bridge and restored the previous user environment.'
    Write-Host 'Codex App must be restarted before the change takes effect.'
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

    if ($InstallAppBridge) {
        Enable-CdxmAppBridge (Resolve-RealCodexPath $RealCodexPath)
    }
    elseif ($RemoveAppBridge) {
        Disable-CdxmAppBridge
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
        if (Confirm-CdxmStep "Add $BinDir and $AgentsBin to the user PATH?" $true) {
            Add-UserPathEntry $BinDir
            Add-UserPathEntry $AgentsBin
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
