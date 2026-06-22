[CmdletBinding()]
param(
    [switch]$Yes,
    [switch]$InstallShim,
    [switch]$NoShim,
    [switch]$NoPath,
    [string]$Source,
    [switch]$SkipBuild,
    [string]$RepoUrl = $(if ($env:CDXM_INSTALL_REPO_URL) { $env:CDXM_INSTALL_REPO_URL } else { 'https://github.com/lucianlamp/codex-monitor' }),
    [string]$Ref = $(if ($env:CDXM_INSTALL_REF) { $env:CDXM_INSTALL_REF } else { 'main' }),
    [string]$InstallRoot = $(if ($env:CDXM_INSTALL_ROOT) { $env:CDXM_INSTALL_ROOT } else { Join-Path $HOME '.codex-monitor' }),
    [string]$SkillDir = $(if ($env:CDXM_SKILL_DIR) { $env:CDXM_SKILL_DIR } else { Join-Path $HOME '.codex\skills\codex-monitor' }),
    [string]$AgentsBin = $(if ($env:CDXM_AGENTS_BIN) { $env:CDXM_AGENTS_BIN } else { Join-Path $HOME '.agents\bin' })
)

$ErrorActionPreference = 'Stop'

$BinDir = Join-Path $InstallRoot 'bin'
$ShimTarget = Join-Path $AgentsBin 'codex.cmd'
$ShimScript = Join-Path $BinDir 'codex-monitor-shim.ps1'
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

function Install-CdxmBinaries {
    param([string]$SourceDir)

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    if ($SkipBuild) {
        Write-Host "Skipped cargo install (-SkipBuild)."
        return
    }

    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        throw "cargo is required to build codex-monitor from source. Install Rust/Cargo, then rerun this installer."
    }

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

function Write-CdxmWindowsShimScript {
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    $script = @'
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CodexArgs
)

$ErrorActionPreference = 'Stop'
$env:CODEX_MONITOR_SHIM_WRAPPER = '1'

function Resolve-RealCodex {
    if ($env:CODEX_MONITOR_REAL_CODEX) {
        return $env:CODEX_MONITOR_REAL_CODEX
    }

    $shimTarget = $env:CODEX_MONITOR_SHIM_TARGET
    $commands = Get-Command codex -All -ErrorAction SilentlyContinue
    foreach ($command in $commands) {
        $source = $command.Source
        if (-not $source) {
            continue
        }
        $resolved = try { (Resolve-Path $source -ErrorAction Stop).Path } catch { $source }
        $target = if ($shimTarget) { try { (Resolve-Path $shimTarget -ErrorAction Stop).Path } catch { $shimTarget } } else { '' }
        if ($target -and ($resolved -ieq $target)) {
            continue
        }
        if ($resolved -ieq $PSCommandPath) {
            continue
        }
        return $resolved
    }

    throw 'codex-monitor shim: real codex not found on PATH'
}

function Get-ProjectFromArgs {
    param([string[]]$Args)

    $project = (Get-Location).Path
    for ($i = 0; $i -lt $Args.Count; $i++) {
        $arg = $Args[$i]
        if (($arg -eq '--cd' -or $arg -eq '--cwd' -or $arg -eq '-C') -and ($i + 1 -lt $Args.Count)) {
            $project = $Args[$i + 1]
            $i++
            continue
        }
        if ($arg.StartsWith('--cd=') -or $arg.StartsWith('--cwd=')) {
            $project = $arg.Substring($arg.IndexOf('=') + 1)
            continue
        }
    }
    if (Test-Path $project -PathType Container) {
        return (Resolve-Path $project).Path
    }
    return (Get-Location).Path
}

function Get-FirstNonOption {
    param([string[]]$Args)

    for ($i = 0; $i -lt $Args.Count; $i++) {
        $arg = $Args[$i]
        if (($arg -eq '--cd' -or $arg -eq '--cwd' -or $arg -eq '-C') -and ($i + 1 -lt $Args.Count)) {
            $i++
            continue
        }
        if ($arg.StartsWith('--cd=') -or $arg.StartsWith('--cwd=')) {
            continue
        }
        if ($arg -in @('--help', '--version', '-h', '-V')) {
            return $arg
        }
        if ($arg.StartsWith('-')) {
            continue
        }
        return $arg
    }
    return ''
}

function Get-ProjectHash {
    param([string]$Project)

    $sha1 = [Security.Cryptography.SHA1]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Project)
        return -join ($sha1.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') })
    } finally {
        $sha1.Dispose()
    }
}

function Test-PortOpen {
    param([int]$Port)

    $client = [Net.Sockets.TcpClient]::new()
    try {
        $async = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
        if (-not $async.AsyncWaitHandle.WaitOne(250)) {
            return $false
        }
        $client.EndConnect($async)
        return $true
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

function Ensure-AppServer {
    param(
        [string]$RealCodex,
        [string]$Project
    )

    $runDir = if ($env:CODEX_MONITOR_SHIM_RUN_DIR) {
        $env:CODEX_MONITOR_SHIM_RUN_DIR
    } else {
        Join-Path $env:LOCALAPPDATA 'codex-monitor\shim'
    }
    New-Item -ItemType Directory -Force -Path $runDir | Out-Null

    $hash = Get-ProjectHash $Project
    $serverLog = Join-Path $runDir "codex-app-server.$hash.log"
    $serverErr = Join-Path $runDir "codex-app-server.$hash.err.log"
    $serverPid = Join-Path $runDir "codex-app-server.$hash.pid"
    $portFile = Join-Path $runDir "codex-app-server.$hash.port"

    $port = $null
    if ((Test-Path $portFile) -and (Test-Path $serverPid)) {
        $existingPort = Get-Content -Raw $portFile -ErrorAction SilentlyContinue
        $existingPid = Get-Content -Raw $serverPid -ErrorAction SilentlyContinue
        $existingPort = "$existingPort".Trim()
        $existingPid = "$existingPid".Trim()
        if ($existingPort -match '^\d+$' -and $existingPid -match '^\d+$') {
            $process = Get-Process -Id ([int]$existingPid) -ErrorAction SilentlyContinue
            if ($process -and (Test-PortOpen ([int]$existingPort))) {
                $port = [int]$existingPort
            }
        }
    }

    if (-not $port) {
        Set-Content -Path $serverLog -Value ''
        Set-Content -Path $serverErr -Value ''
        $process = Start-Process -FilePath $RealCodex `
            -ArgumentList @('app-server', '--listen', 'ws://127.0.0.1:0') `
            -RedirectStandardOutput $serverLog `
            -RedirectStandardError $serverErr `
            -WindowStyle Hidden `
            -PassThru
        Set-Content -Path $serverPid -Value $process.Id

        $deadline = (Get-Date).AddSeconds(15)
        while ((Get-Date) -lt $deadline) {
            $log = Get-Content -Raw $serverLog -ErrorAction SilentlyContinue
            if ($log -match 'listening on:\s*ws://127\.0\.0\.1:(\d+)') {
                $port = [int]$Matches[1]
                break
            }
            Start-Sleep -Milliseconds 100
        }

        if (-not $port) {
            throw "codex-monitor shim: app-server did not report a listening port; see $serverLog"
        }
        Set-Content -Path $portFile -Value $port
    }

    if (-not (Test-PortOpen $port)) {
        throw "codex-monitor shim: app-server did not start on ws://127.0.0.1:$port"
    }
    return "ws://127.0.0.1:$port"
}

$realCodex = Resolve-RealCodex

if ($env:CODEX_MONITOR_SHIM_DISABLE -eq '1') {
    & $realCodex @CodexArgs
    exit $LASTEXITCODE
}

$commandName = Get-FirstNonOption $CodexArgs
if ($commandName -in @('app-server', 'exec', 'login', 'logout', 'mcp', 'completion', 'debug', 'apply', 'review', 'sandbox', 'help', '--help', '-h', 'version', '--version', '-V')) {
    & $realCodex @CodexArgs
    exit $LASTEXITCODE
}

$project = Get-ProjectFromArgs $CodexArgs
$socketUrl = Ensure-AppServer $realCodex $project
Set-Location $project

if ($commandName -eq 'resume') {
    $resumeArgs = New-Object System.Collections.Generic.List[string]
    $removed = $false
    foreach ($arg in $CodexArgs) {
        if (-not $removed -and $arg -eq 'resume') {
            $removed = $true
            continue
        }
        [void]$resumeArgs.Add($arg)
    }
    & $realCodex resume --remote $socketUrl @resumeArgs
    exit $LASTEXITCODE
}

& $realCodex --remote $socketUrl @CodexArgs
exit $LASTEXITCODE
'@
    Set-Content -Path $ShimScript -Value $script -Encoding UTF8
    Write-Host "Installed Codex monitor PowerShell shim to $ShimScript"
}

function Write-CdxmCodexCmd {
    Write-CdxmWindowsShimScript
    if (Test-Path $ShimTarget) {
        $kind = Get-CodexEntrypointKind $ShimTarget
        Write-Host "$ShimTarget already exists: detected $kind; Leaving existing codex entrypoint untouched."
        return
    }

    New-Item -ItemType Directory -Force -Path $AgentsBin | Out-Null
    $cmd = @"
@echo off
set "CODEX_MONITOR_SHIM_WRAPPER=1"
set "CODEX_MONITOR_SHIM_TARGET=%~f0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ShimScript" %*
"@
    Set-Content -Path $ShimTarget -Value $cmd -Encoding ASCII
    Write-Host "Installed Codex monitor shim to $ShimTarget"
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
