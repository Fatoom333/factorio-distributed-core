# Starts the benchmark server (run setup-env.ps1 first). Stop it with Ctrl+C.
# Game port and RCON listen on 127.0.0.1 only.
param(
    [string]$Factorio = "C:\Program Files (x86)\Steam\steamapps\common\Factorio\bin\x64\factorio.exe",
    [string]$EnvDir = (Join-Path $PSScriptRoot "..\..\bench-env"),
    [int]$RconPort = 27015
)
$ErrorActionPreference = "Stop"
$EnvDir = (Resolve-Path $EnvDir).Path
$password = (Get-Content "$EnvDir\rcon-password.txt" -Raw).Trim()

& $Factorio -c "$EnvDir\config\config.ini" `
    --start-server "$EnvDir\saves\bench.zip" `
    --server-settings "$EnvDir\server-settings.json" `
    --bind 127.0.0.1 `
    --rcon-bind "127.0.0.1:$RconPort" `
    --rcon-password $password
