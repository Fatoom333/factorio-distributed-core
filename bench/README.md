# Замер RCON (TAR-166)

Отвечает на вопрос: потянет ли RCON поток изменений мира тик в тик.

## Как запустить

```powershell
# 1. Отдельное окружение Factorio (свои сейвы/моды не трогает) — один раз
.\bench\setup-env.ps1
# 2. Сервер без окна, RCON на 127.0.0.1:27015
.\bench\run-server.ps1
# 3. В другом окне — замер
$env:FD_RCON_PASSWORD_FILE = "..\bench-env\rcon-password.txt"
cargo run --release --bin rcon_bench
```

Окружение создаётся в `../bench-env` (рядом с репозиториями, не в git): свой `config.ini`,
карта, ссылка на мод `../factorio-distributed-mod`, случайный пароль RCON.
Сервер слушает только `127.0.0.1`, не виден ни в публичном списке, ни в LAN,
не встаёт на паузу без игроков (`auto_pause = false`).

Все команды — `/fd <JSON>` (своя команда мода): она не отключает достижения и не
выполняет произвольный Lua. Результаты — [RESULTS.md](RESULTS.md).
