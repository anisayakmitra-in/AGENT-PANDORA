<#
.SYNOPSIS
    Runs the CI verify job locally, in CI's order, and prints a summary.

.DESCRIPTION
    This mirrors the `verify` job in .github/workflows/ci.yml. The commands are
    transcribed from that file, not from memory; if you change ci.yml, change
    this script in the same commit or it stops being a gate.

    STEP 1 shipped a formatting failure that every local check passed, because
    the local check enumerated lib.rs, main.rs and tests/*.rs but not
    src/bin/*.rs. A gate that reimplements the job is a gate that can drift
    from it. This runs the job's commands, so the file list is never ours to
    get wrong.

    Steps that are environment provisioning in CI (checkout, toolchain
    install, cache restore, artifact upload) have no local equivalent and are
    reported as SKIPPED rather than silently dropped.

    Exit code 0 if every runnable step passed, 1 otherwise.

.PARAMETER Full
    Also run the slow release-path steps: cargo install of the CLI and the
    performance baseline. These are the last four steps of the CI job and are
    excluded by default because `cargo install --path` recompiles the CLI from
    scratch and the baseline takes minutes.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\prepush.ps1
#>
[CmdletBinding()]
param(
    [switch]$Full
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot

# Records every step so the run ends with one pasteable summary.
$results = [System.Collections.Generic.List[object]]::new()

function Invoke-Step {
    param(
        [Parameter(Mandatory)] [string]$Name,
        [Parameter(Mandatory)] [string]$Command,
        [string]$WorkingDirectory = '.',
        [hashtable]$Environment = @{},
        [string[]]$Expect = @()
    )

    Write-Host ''
    Write-Host "==> $Name" -ForegroundColor Cyan
    Write-Host "    $Command" -ForegroundColor DarkGray

    $previousLocation = Get-Location
    $previousEnvironment = @{}
    try {
        Set-Location $WorkingDirectory
        foreach ($key in $Environment.Keys) {
            $previousEnvironment[$key] = [Environment]::GetEnvironmentVariable($key)
            [Environment]::SetEnvironmentVariable($key, $Environment[$key])
        }

        # cargo, npm and python all write progress to stderr on success. With
        # $ErrorActionPreference = 'Stop' that becomes a terminating
        # NativeCommandError before the exit code is ever read, so the step
        # would report failure no matter what the tool actually returned. The
        # exit code is the verdict; stderr is not.
        $output = & {
            $ErrorActionPreference = 'Continue'
            & ([scriptblock]::Create($Command)) 2>&1
        } | Out-String
        $code = $LASTEXITCODE

        if ($code -ne 0) {
            Write-Host $output -ForegroundColor Red
        } else {
            # Only show the tail. Successful cargo and npm output is long and
            # the summary is the point, not the scroll.
            $tail = ($output -split "`r?`n" | Where-Object { $_.Trim() } | Select-Object -Last 6) -join "`n"
            if ($tail) { Write-Host $tail -ForegroundColor DarkGray }
        }

        # Some steps legitimately succeed without proving what we care about.
        # The workflow-hardening test skips when GITHUB_TOKEN is absent, which
        # is the normal local case; reporting that as a plain pass would
        # overstate what was checked.
        $skipNote = $null
        if ($code -eq 0 -and $Expect -and ($output -match 'skipped=\d+')) {
            $skipped = [regex]::Match($output, 'skipped=(\d+)').Groups[1].Value
            if ([int]$skipped -gt 0) {
                $skipNote = "passed, but $skipped test(s) skipped without GITHUB_TOKEN"
            }
        }

        $results.Add([pscustomobject]@{
            Step     = $Name
            Status   = if ($code -eq 0) { 'PASS' } else { 'FAIL' }
            Exit     = $code
            Duration = ''
            Note     = $skipNote
        })
        return ($code -eq 0)
    } finally {
        Set-Location $previousLocation
        foreach ($key in $previousEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($key, $previousEnvironment[$key])
        }
    }
}

function Add-NotApplicable {
    param([Parameter(Mandatory)] [string]$Name, [Parameter(Mandatory)] [string]$Reason)
    $results.Add([pscustomobject]@{
        Step = $Name; Status = 'SKIP'; Exit = 0; Duration = ''; Note = $Reason
    })
}

Write-Host "Verifying against the CI verify job in .github/workflows/ci.yml" -ForegroundColor White
Write-Host "  repo:     $repoRoot"
Write-Host "  cargo:    $(& cargo --version)"
Write-Host "  rustc:    $(& rustc --version)"
Write-Host "  python:   $(& python --version 2>&1)"

# ---------------------------------------------------------------------------
# Setup steps. Provisioning in CI; here they are assertions that the local
# toolchain is the one CI pins, because a gate run on the wrong rustc proves
# nothing about the merge.
# ---------------------------------------------------------------------------
Add-NotApplicable 'Check out source' 'provisioning in CI'
Add-NotApplicable 'Install Rust' "local toolchain is pinned by rust-toolchain.toml: $(& rustup show active-toolchain)"
Add-NotApplicable 'Restore Rust build cache' 'provisioning in CI'

$expectedNode = 'v22'
$actualNode = (& node --version) 2>&1
if ($actualNode -notlike "$expectedNode.*") {
    Write-Host "    WARNING: CI pins Node $expectedNode, local is $actualNode" -ForegroundColor Yellow
}

# ---------------------------------------------------------------------------
# The verify job, in order.
# ---------------------------------------------------------------------------
$failed = $false

if (-not (Invoke-Step -Name 'Check formatting' -Command 'cargo fmt --all -- --check')) { $failed = $true }
if (-not (Invoke-Step -Name 'Check workspace' -Command 'cargo check --workspace --locked')) { $failed = $true }
if (-not (Invoke-Step -Name 'Run Clippy' -Command 'cargo clippy --workspace --all-targets --locked -- -D warnings')) { $failed = $true }
if (-not (Invoke-Step -Name 'Run Rust tests' -Command 'cargo test --workspace --lib --tests --locked')) { $failed = $true }

# npm.ps1 is blocked by this machine's execution policy, so the shim is called
# as npm.cmd. On a normal shell plain `npm` is equivalent.
if (-not (Invoke-Step -Name 'Build TypeScript launcher boundary' `
        -Command 'npm.cmd ci --ignore-scripts; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; npm.cmd run build' `
        -WorkingDirectory 'npm/pandora-cli')) { $failed = $true }

if (-not (Invoke-Step -Name 'Check TypeScript launcher output is current' `
        -Command 'git diff --exit-code -- npm/pandora-cli/lib')) { $failed = $true }

if (-not (Invoke-Step -Name 'Test npm launcher' -Command 'node scripts/test_npm_launcher.js')) { $failed = $true }
if (-not (Invoke-Step -Name 'Test TypeScript client' -Command 'node scripts/test_typescript_client.js')) { $failed = $true }

if (-not (Invoke-Step -Name 'Run Python tests' -Command 'python -m unittest discover -s scripts -p "test_*.py" -v' `
        -Expect @('python'))) { $failed = $true }

# No GITHUB_TOKEN locally, so this step's own comment says it will skip rather
# than fail. Passing it through unchanged keeps the behaviour honest.
if (-not (Invoke-Step -Name 'Check workflow hardening' `
        -Command 'python -m unittest scripts.test_workflow_hardening -v' `
        -Expect @('python'))) { $failed = $true }

if (-not (Invoke-Step -Name 'Validate repository' -Command 'python scripts/validate_repo.py')) { $failed = $true }
if (-not (Invoke-Step -Name 'Validate documentation' -Command 'python scripts/validate_docs.py')) { $failed = $true }

if (-not (Invoke-Step -Name 'Build release CLI' -Command 'cargo build --release -p pandora-cli --locked')) { $failed = $true }

if ($Full) {
    $installRoot = Join-Path $env:TEMP 'pandora-cargo-install'
    if (-not (Invoke-Step -Name 'Verify cargo-installable CLI (Windows)' `
            -Command "cargo install --path crates/pandora-cli --locked --root '$installRoot' --force; if (`$LASTEXITCODE -ne 0) { exit `$LASTEXITCODE }; & '$installRoot\bin\pandora.exe' --version")) {
        $failed = $true
    }

    $baseline = Join-Path $env:TEMP 'pandora-cli-baseline.json'
    if (-not (Invoke-Step -Name 'Measure CLI baseline (Windows)' `
            -Command "python scripts/measure_cli.py --binary target/release/pandora.exe --iterations 5 --timeout-seconds 10 --output '$baseline'")) {
        $failed = $true
    }
} else {
    Add-NotApplicable 'Verify cargo-installable CLI (Windows)' 'pass -Full to run; it recompiles the CLI from scratch'
    Add-NotApplicable 'Measure CLI baseline (Windows)' 'pass -Full to run; timing-sensitive and takes minutes'
}

Add-NotApplicable 'Upload CLI baseline' 'artifact upload, no local equivalent'

# ---------------------------------------------------------------------------
# Summary. Shaped to paste into a PR description.
# ---------------------------------------------------------------------------
$width = ($results | ForEach-Object { $_.Step.Length } | Measure-Object -Maximum).Maximum
Write-Host ''
Write-Host '----------------------------------------' -ForegroundColor DarkGray
Write-Host 'prepush summary' -ForegroundColor White
Write-Host '----------------------------------------' -ForegroundColor DarkGray
foreach ($result in $results) {
    $colour = switch ($result.Status) {
        'PASS' { 'Green' }
        'FAIL' { 'Red' }
        default { 'DarkYellow' }
    }
    $line = "  {0,-$width}  {1}" -f $result.Step, $result.Status
    if ($result.Note) { $line += "  ($($result.Note))" }
    Write-Host $line -ForegroundColor $colour
}
Write-Host ''

$counts = $results | Group-Object Status | Sort-Object Name
foreach ($group in $counts) { Write-Host ("  {0}: {1}" -f $group.Name, $group.Count) }
Write-Host ("  source: .github/workflows/ci.yml verify job") -ForegroundColor DarkGray

Pop-Location
if ($failed) {
    Write-Host ''
    Write-Host 'prepush: FAILED - do not push' -ForegroundColor Red
    exit 1
}
Write-Host ''
Write-Host 'prepush: passed' -ForegroundColor Green
exit 0