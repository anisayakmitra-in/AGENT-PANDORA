# Start AGENT-PANDORA External Messaging Bridge
param (
    [string]$TelegramToken = $env:TELEGRAM_BOT_TOKEN,
    [string]$TelegramChatId = $env:TELEGRAM_CHAT_ID,
    [string]$DiscordWebhook = $env:DISCORD_WEBHOOK_URL,
    [switch]$Smoke
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoDir = Split-Path -Parent (Split-Path -Parent $scriptDir)
$pythonPath = "C:\Users\infin\.gemini\antigravity\scratch\sandbox-workspace\python-embed\python.exe"

if (-not (Test-Path $pythonPath)) {
    $pythonPath = "python"
}

$bridgePy = Join-Path $scriptDir "pandora_bridge.py"

$argsList = @($bridgePy)

if ($Smoke) {
    $argsList += "--smoke"
}
if ($TelegramToken) {
    $argsList += "--telegram-token", $TelegramToken
}
if ($TelegramChatId) {
    $argsList += "--telegram-chat-ids", $TelegramChatId
}
if ($DiscordWebhook) {
    $argsList += "--discord-webhook", $DiscordWebhook
}

& $pythonPath @argsList
