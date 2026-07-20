# deploy-poll.ps1 — Windows schtasks poll wrapper for Zeus deploy-on-merge.
#
# Windows release artifacts are deferred elsewhere, but Windows seats still need
# the poll unit shape. This wrapper records a fail-loud telemetry event instead
# of silently pretending a Unix deploy script can supervise a Windows service.

$ErrorActionPreference = "Stop"

$Repo = if ($env:ZEUS_REPO) { $env:ZEUS_REPO } else { Join-Path $HOME "Zeus" }
$ZeusHome = if ($env:ZEUS_HOME) { $env:ZEUS_HOME } else { Join-Path $HOME ".zeus" }
$LogDir = Join-Path $ZeusHome "logs"
$LogFile = Join-Path $LogDir "fleet-failures.jsonl"
$Branch = if ($env:ZEUS_DEPLOY_BRANCH) { $env:ZEUS_DEPLOY_BRANCH } else { "main" }

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

function Write-FleetEvent([string]$Kind, [string]$Severity, [string]$Summary, [string]$Details = "") {
    $evt = [ordered]@{
        ts = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        seat = if ($env:ZEUS_SEAT) { $env:ZEUS_SEAT } else { $env:COMPUTERNAME }
        host = $env:COMPUTERNAME
        kind = $Kind
        severity = $Severity
        source = "deploy-poll.ps1"
        summary = $Summary
        sha = ""
        details = $Details
    }
    ($evt | ConvertTo-Json -Compress) | Add-Content -Encoding UTF8 -Path $LogFile
}

try {
    Push-Location $Repo
    $remoteFull = (& git ls-remote origin "refs/heads/$Branch" | ForEach-Object { ($_ -split "\s+")[0] })
    if (-not $remoteFull) { throw "could not resolve origin/$Branch" }
    $remote = $remoteFull.Substring(0, [Math]::Min(8, $remoteFull.Length))

    $stateDir = Join-Path $ZeusHome "deploy"
    $stamp = Join-Path $stateDir "last-deploy"
    $last = ""
    if (Test-Path $stamp) {
        $line = Select-String -Path $stamp -Pattern '^sha=' | Select-Object -First 1
        if ($line) { $last = $line.Line.Substring(4) }
    }

    if ($remote -eq $last) { exit 0 }

    Write-FleetEvent "deploy_failure" "warn" "origin/$Branch moved to $remote but Windows deploy is not enabled in this Unix-script slice" "repo=$Repo last=$last"
    Write-Error "Windows deploy-on-merge requires a native deploy-on-merge.ps1 implementation before enabling this task. Remote=$remote Last=$last"
    exit 1
}
catch {
    Write-FleetEvent "deploy_failure" "error" "Windows deploy poll failed" $_.Exception.Message
    throw
}
finally {
    Pop-Location 2>$null
}
