# Starts Factorio WITH a window in the benchmark environment (run setup-env.ps1 first).
# Factorio 2.0 has no command-line switch to host a multiplayer game with a window,
# so after start: Multiplayer -> Host saved game -> bench -> turn off public/LAN/Steam -> Host.
# RCON then listens on 127.0.0.1:$RconPort (local-rcon-socket in the generated config).
param(
    [string]$Factorio = "C:\Program Files (x86)\Steam\steamapps\common\Factorio\bin\x64\factorio.exe",
    [string]$EnvDir = (Join-Path $PSScriptRoot "..\..\bench-env"),
    [int]$RconPort = 27015
)
$ErrorActionPreference = "Stop"
$EnvDir = (Resolve-Path $EnvDir).Path
$password = (Get-Content "$EnvDir\rcon-password.txt" -Raw).Trim()

# Same paths as the server config, plus menu-hosted RCON and the FPS/UPS overlay.
$config = "[path]`r`nread-data=__PATH__executable__\..\..\data`r`nwrite-data=$EnvDir`r`n" +
    "[other]`r`nlocal-rcon-socket=127.0.0.1:$RconPort`r`nlocal-rcon-password=$password`r`n" +
    "[debug]`r`nshow-fps=always`r`nshow-multiplayer-ups=always`r`n"
[IO.File]::WriteAllText("$EnvDir\config\config-gui.ini", $config)

& $Factorio -c "$EnvDir\config\config-gui.ini" --disable-audio --window-size 1600x900
