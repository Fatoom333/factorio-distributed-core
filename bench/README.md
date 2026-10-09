# Замер RCON (TAR-166)

Отвечает на вопрос: потянет ли RCON поток изменений мира тик в тик.

## Как запустить

```powershell
# 1. Отдельное окружение Factorio (свои сейвы/моды не трогает) — один раз
.\bench\setup-env.ps1
# 2. Сервер без окна, RCON на 127.0.0.1:27015 (или клиент с окном: .enchun-client.ps1, затем хост из меню)
.\bench\run-server.ps1
# 3. В другом окне — замер
# пароль и script-output берутся из ..\bench-env (другое место — $env:FD_ENV_DIR)
cargo run --release --bin rcon_bench -- all   # или transport / events / freeze / insert
```

Окружение создаётся в `../bench-env` (рядом с репозиториями, не в git): свой `config.ini`,
карта, ссылка на мод `../factorio-distributed-mod`, случайный пароль RCON.
Сервер слушает только `127.0.0.1`, не виден ни в публичном списке, ни в LAN,
не встаёт на паузу без игроков (`auto_pause = false`).

Все команды — `/fd <JSON>` (своя команда мода): она не отключает достижения и не
выполняет произвольный Lua. Результаты — [RESULTS.md](RESULTS.md).
