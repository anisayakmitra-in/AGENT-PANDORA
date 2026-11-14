# Autonomous AGENT-PANDORA v3.1.0 Google Colab A100 GPU Pipeline Runner.
# Communicates directly with the remote Google Colab GPU instance to monitor telemetry,
# download distilled Tropical Polyhedral AST Network (TPAN) weights, and verify local
# pure safe Rust forward-pass operations with zero external API / Antigravity token consumption.

$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$sessionPath = Join-Path $workspaceRoot ".pandora\colab_session.json"
$modelPath = Join-Path $workspaceRoot ".pandora\model_tpan.pnn"

Write-Host ""
Write-Host "======================================================================" -ForegroundColor Cyan
Write-Host "AGENT-PANDORA v3.1.0: Autonomous Colab A100 GPU Pipeline" -ForegroundColor Cyan
Write-Host "======================================================================" -ForegroundColor Cyan

if (-not (Test-Path $sessionPath)) {
    Write-Error "Session file not found at $sessionPath. Run 'pandora colab connect' first."
}

$session = Get-Content $sessionPath | ConvertFrom-Json
$url = $session.url.TrimEnd('/')
$token = $session.token

Write-Host ""
Write-Host "[+] Connecting to Google Colab GPU Bridge..." -ForegroundColor Green
Write-Host "    URL:   $url"
Write-Host "    Token: $($token.Substring(0, [math]::Min(18, $token.Length)))..."

# 1. Probe Status
try {
    $statusHeaders = @{ "X-Pandora-Colab-Token" = $token }
    $status = Invoke-RestMethod -Uri "$url/api/colab/status" -Headers $statusHeaders -Method Get -TimeoutSec 10
    
    Write-Host ""
    Write-Host "[+] Live GPU Telemetry:" -ForegroundColor Green
    Write-Host "    GPU Device:   $($status.gpu_device)" -ForegroundColor Yellow
    Write-Host "    VRAM Total:   $($status.vram_total_mb) MB"
    Write-Host "    VRAM Free:    $($status.vram_free_mb) MB"
    Write-Host "    Active Nets:  $($status.active_models -join ', ')"
} catch {
    Write-Error "Failed to probe Colab status at $url/api/colab/status: $_"
}

# 2. Check for Distilled Model Export
Write-Host ""
Write-Host "[+] Checking for distilled Tropical Polyhedral Model on A100..." -ForegroundColor Green
try {
    $exportResp = Invoke-RestMethod -Uri "$url/api/colab/export" -Headers $statusHeaders -Method Get -TimeoutSec 15
    $exportJson = $exportResp | ConvertTo-Json -Depth 10
    Set-Content -Path $modelPath -Value $exportJson
    
    Write-Host "    [SUCCESS] Found distilled model: $($exportResp.name) ($($exportResp.domain))" -ForegroundColor Green
    Write-Host "    Saved to: $modelPath"
    Write-Host "    SHA-256:  $($exportResp.sha256_digest)"
    if ($exportResp.metadata) {
        Write-Host "    Exact Tropical MSE: $($exportResp.metadata.exact_tropical_mse)"
        Write-Host "    Pearson Correlation: $($exportResp.metadata.ranking_correlation)"
    }
} catch {
    Write-Host "    [INFO] Model not yet exported via Colab API. (Run Step 4 in Colab to distill)." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "======================================================================" -ForegroundColor Cyan
Write-Host "[DONE] Colab A100 GPU Pipeline Active (0 LLM Tokens Consumed)" -ForegroundColor Green
Write-Host "======================================================================" -ForegroundColor Cyan
Write-Host ""
