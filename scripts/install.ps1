$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "pandora installer: $Message"
}

$defaultVersion = "v2.0.0-beta.7"
$version = if ([string]::IsNullOrWhiteSpace($env:PANDORA_VERSION)) {
    $defaultVersion
} else {
    $env:PANDORA_VERSION
}
if ($version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
    Fail "PANDORA_VERSION must be a SemVer tag such as v2.0.0-beta.7"
}

$base = $env:PANDORA_RELEASE_BASE_URL
if ([string]::IsNullOrWhiteSpace($base)) {
    $base = "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download"
}
$baseUri = [Uri]$base
if ($baseUri.Scheme -ne "https" -or $baseUri.UserInfo -or $baseUri.Query -or $baseUri.Fragment) {
    Fail "PANDORA_RELEASE_BASE_URL must use HTTPS without credentials or query parameters"
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($architecture) {
    "X64" { $artifact = "pandora-x86_64-pc-windows-msvc.exe" }
    default { Fail "unsupported Windows architecture: $architecture" }
}

$installDir = $env:PANDORA_INSTALL_DIR
if ([string]::IsNullOrWhiteSpace($installDir)) {
    $installDir = Join-Path $env:LOCALAPPDATA "Pandora\bin"
}
$temporary = Join-Path ([IO.Path]::GetTempPath()) ("pandora-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $temporary | Out-Null

try {
    $base = $base.TrimEnd('/')
    $checksumsPath = Join-Path $temporary "checksums.txt"
    $artifactPath = Join-Path $temporary $artifact
    Invoke-WebRequest -Uri "$base/$version/checksums.txt" -OutFile $checksumsPath
    Invoke-WebRequest -Uri "$base/$version/$artifact" -OutFile $artifactPath

    $checksumLine = Get-Content -LiteralPath $checksumsPath | Where-Object {
        $parts = $_ -split '\s+', 2
        $parts.Count -eq 2 -and (
            $parts[1].TrimStart('*') -eq $artifact -or
            $parts[1].TrimStart('*') -eq "dist/$artifact"
        )
    } | Select-Object -First 1
    if ($null -eq $checksumLine) { Fail "release checksum is missing" }
    $expected = ($checksumLine -split '\s+')[0]
    if ($expected -notmatch '^[0-9A-Fa-f]{64}$') { Fail "release checksum is malformed" }
    $actual = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash
    if ($actual.ToLowerInvariant() -ne $expected.ToLowerInvariant()) {
        Fail "release checksum verification failed"
    }

    if ($env:PANDORA_REQUIRE_SIGNATURE -eq "1") {
        if (-not (Get-Command cosign -ErrorAction SilentlyContinue)) {
            Fail "cosign is required when PANDORA_REQUIRE_SIGNATURE=1"
        }
        if ([string]::IsNullOrWhiteSpace($env:PANDORA_COSIGN_IDENTITY)) {
            Fail "PANDORA_COSIGN_IDENTITY is required for signature verification"
        }
        $signaturePath = Join-Path $temporary "checksums.txt.sig"
        $certificatePath = Join-Path $temporary "checksums.txt.pem"
        Invoke-WebRequest -Uri "$base/$version/checksums.txt.sig" -OutFile $signaturePath
        Invoke-WebRequest -Uri "$base/$version/checksums.txt.pem" -OutFile $certificatePath
        cosign verify-blob $checksumsPath `
            --certificate $certificatePath `
            --signature $signaturePath `
            --certificate-identity $env:PANDORA_COSIGN_IDENTITY `
            --certificate-oidc-issuer $(if ($env:PANDORA_COSIGN_OIDC_ISSUER) { $env:PANDORA_COSIGN_OIDC_ISSUER } else { "https://token.actions.githubusercontent.com" }) | Out-Null
        if ($LASTEXITCODE -ne 0) { Fail "release signature verification failed" }
    }

    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $target = Join-Path $installDir "pandora.exe"
    if (Test-Path -LiteralPath $target) {
        $item = Get-Item -Force -LiteralPath $target
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Fail "refusing to replace reparse point: $target"
        }
    }
    $staged = Join-Path $installDir (".pandora." + [Guid]::NewGuid().ToString("N") + ".new")
    Copy-Item -LiteralPath $artifactPath -Destination $staged
    Move-Item -LiteralPath $staged -Destination $target -Force
    Write-Output "Pandora installed at $target"
}
finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
