# Prepares an isolated Factorio environment for the RCON benchmark (TAR-166).
# Your normal saves, mods and settings in %APPDATA%\Factorio are not touched:
# the benchmark server uses its own write-data folder ($EnvDir).
param(
    [string]$Factorio = "C:\Program Files (x86)\Steam\steamapps\common\Factorio\bin\x64\factorio.exe",
    [string]$EnvDir = (Join-Path $PSScriptRoot "..\..\bench-env"),
    [string]$ModDir = (Join-Path $PSScriptRoot "..\..\factorio-distributed-mod")
)
$ErrorActionPreference = "Stop"

New-Item -ItemType Directory -Force "$EnvDir\mods", "$EnvDir\saves", "$EnvDir\config" | Out-Null
$EnvDir = (Resolve-Path $EnvDir).Path
$ModDir = (Resolve-Path $ModDir).Path

# Own config: read-data from the game install, write-data into $EnvDir.
$config = "[path]`r`nread-data=__PATH__executable__\..\..\data`r`nwrite-data=$EnvDir`r`n"
[IO.File]::WriteAllText("$EnvDir\config\config.ini", $config)

# Mod is linked, not copied: edits in the repo are picked up on server restart.
$link = "$EnvDir\mods\factorio-distributed"
if (-not (Test-Path $link)) {
    New-Item -ItemType Junction -Path $link -Target $ModDir | Out-Null
}

# Local-only server: not listed publicly or on LAN, never pauses without players.
$settings = @{
    name = "fd-bench"
    description = "RCON benchmark"
    visibility = @{ public = $false; lan = $false }
    require_user_verification = $false
    auto_pause = $false
} | ConvertTo-Json
[IO.File]::WriteAllText("$EnvDir\server-settings.json", $settings)

# Random RCON password, kept only in this file (outside the git repos).
$pwFile = "$EnvDir\rcon-password.txt"
if (-not (Test-Path $pwFile)) {
    $bytes = New-Object byte[] 24
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    [IO.File]::WriteAllText($pwFile, [Convert]::ToBase64String($bytes))
}

$save = "$EnvDir\saves\bench.zip"
if (-not (Test-Path $save)) {
    & $Factorio -c "$EnvDir\config\config.ini" --create $save | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "factorio --create failed ($LASTEXITCODE)" }
}
"Environment ready: $EnvDir"
