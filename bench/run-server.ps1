# Starts the benchmark server (run setup-env.ps1 first). Stop it with Ctrl+C.
# Game port and RCON listen on 127.0.0.1 only.
# -Save / -ModDirectory: another save and mod set, e.g. the megabase without Space Age:
#   .\bench\run-server.ps1 -Save ..\bench-env\megabase\VTN_by_PMAP_Megabase2k.zip -ModDirectory ..\bench-env\mods-mega
param(
    [string]$Factorio = "C:\Program Files (x86)\Steam\steamapps\common\Factorio\bin\x64\factorio.exe",
    [string]$EnvDir = (Join-Path $PSScriptRoot "..\..\bench-env"),
    [string]$Save = "",
    [string]$ModDirectory = "",
    [int]$RconPort = 27015
)
$ErrorActionPreference = "Stop"
$EnvDir = (Resolve-Path $EnvDir).Path
if (-not $Save) { $Save = "$EnvDir\saves\bench.zip" }
$Save = (Resolve-Path $Save).Path
$password = (Get-Content "$EnvDir\rcon-password.txt" -Raw).Trim()

$modArgs = @()
if ($ModDirectory) { $modArgs = @("--mod-directory", (Resolve-Path $ModDirectory).Path) }

& $Factorio -c "$EnvDir\config\config.ini" `
    --start-server $Save `
    --server-settings "$EnvDir\server-settings.json" `
    --bind 127.0.0.1 `
    --rcon-bind "127.0.0.1:$RconPort" `
    --rcon-password $password `
    @modArgs
