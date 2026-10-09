//! Замер пропускной способности RCON Factorio (TAR-166).
//!
//! Нужен сервер Factorio с модом factorio-distributed и RCON на 127.0.0.1
//! (см. bench/README.md). Запуск:
//!
//!   cargo run --release --bin rcon_bench -- [all|transport|events|freeze]
//!
//! Все команды — `/fd <JSON>` (команда мода, достижения не отключает).
//! Печатает markdown-таблицы: задержка и скорость по размеру команды,
//! сколько команд Factorio успевает за тик, размер ответа, применение
//! пачек обновлений к сундукам и UPS под нагрузкой.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rcon::Connection;
use tokio::net::TcpStream;

type Conn = Connection<TcpStream>;
type R<T> = Result<T, Box<dyn std::error::Error>>;

struct Target {
    addr: String,
    password: String,
}

async fn connect(t: &Target) -> R<Conn> {
    Ok(Connection::<TcpStream>::builder()
        .enable_factorio_quirks(true)
        .connect(t.addr.as_str(), &t.password)
        .await?)
}

/// Отправляет `/fd <json>`; ответ мода «error: ...» превращает в ошибку.
async fn fd(c: &mut Conn, json: &str) -> R<String> {
    let r = c.cmd(&format!("/fd {json}")).await?;
    if r.starts_with("error:") {
        return Err(format!("мод ответил: {}", r.trim()).into());
    }
    Ok(r)
}

async fn tick(c: &mut Conn) -> R<u64> {
    Ok(fd(c, r#"{"op":"tick"}"#).await?.trim().parse()?)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Перцентиль p (0..1) в миллисекундах; сортирует срез на месте.
fn pct(v: &mut [Duration], p: f64) -> f64 {
    v.sort();
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    ms(v[i])
}

fn pct_f(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

const TICK_MS: f64 = 1000.0 / 60.0;

/// Набор замеров — первый аргумент: `all` (по умолчанию), `transport`, `events` или `freeze`.
/// Окружение — FD_ENV_DIR (по умолчанию `../bench-env`): пароль RCON и script-output.
#[tokio::main(flavor = "current_thread")]
async fn main() -> R<()> {
    let suite = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let addr = std::env::var("FD_RCON_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".into());
    let env_dir = PathBuf::from(std::env::var("FD_ENV_DIR").unwrap_or_else(|_| "../bench-env".into()));
    let password = std::fs::read_to_string(env_dir.join("rcon-password.txt"))?.trim().to_string();
    let target = Target { addr, password };

    let mut c = connect(&target).await?;
    let pong = fd(&mut c, r#"{"op":"ping"}"#).await?;
    if pong.trim() != "pong" {
        return Err(format!("мод не ответил на ping: {pong:?}").into());
    }

    idle_ups(&mut c).await?;
    if suite == "all" || suite == "transport" {
        raw_payload(&mut c, &target).await?;
        cmds_per_tick(&mut c).await?;
        response_size(&mut c, &target).await?;
        apply(&mut c).await?;
    }
    if suite == "all" || suite == "events" {
        events(&mut c, &env_dir).await?;
    }
    if suite == "all" || suite == "freeze" {
        freeze(&mut c).await?;
    }
    Ok(())
}

/// UPS без нагрузки: сколько тиков игра проходит за 3 секунды.
async fn idle_ups(c: &mut Conn) -> R<()> {
    let (t0, w0) = (tick(c).await?, Instant::now());
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (t1, w1) = (tick(c).await?, Instant::now());
    let ups = (t1 - t0) as f64 / (w1 - w0).as_secs_f64();
    println!("## Базовый UPS без нагрузки: {ups:.1}\n");
    Ok(())
}

/// Команда с балластом заданного размера: передача + разбор JSON, без работы.
async fn raw_payload(c: &mut Conn, t: &Target) -> R<()> {
    println!("## Размер команды (JSON с балластом)\n");
    println!("| байт | повторов | p50 мс | p95 мс | макс мс | МБ/с (по p50) |");
    println!("|---:|---:|---:|---:|---:|---:|");
    for size in [100usize, 1_000, 10_000, 100_000, 1_000_000, 4_000_000, 16_000_000] {
        let json = format!(r#"{{"op":"noop","pad":"{}"}}"#, "x".repeat(size));
        let reps = match size {
            0..=10_000 => 50,
            10_001..=100_000 => 20,
            _ => 5,
        };
        let mut lat = Vec::with_capacity(reps);
        let mut failed = None;
        for _ in 0..reps {
            let s = Instant::now();
            match fd(c, &json).await {
                Ok(_) => lat.push(s.elapsed()),
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
        }
        if let Some(e) = failed {
            println!("| {size} | — | ошибка: {e} | | | |");
            *c = connect(t).await?;
            break;
        }
        let bytes = json.len() + 4; // + "/fd "
        let p50 = pct(&mut lat, 0.5);
        let mbps = bytes as f64 / 1e6 / (p50 / 1000.0);
        println!(
            "| {bytes} | {reps} | {p50:.2} | {:.2} | {:.2} | {mbps:.1} |",
            pct(&mut lat, 0.95),
            pct(&mut lat, 1.0)
        );
    }
    println!();
    Ok(())
}

/// Сколько последовательных команд Factorio выполняет за один тик.
async fn cmds_per_tick(c: &mut Conn) -> R<()> {
    const N: usize = 600;
    let mut lat = Vec::with_capacity(N);
    let mut ticks = Vec::with_capacity(N);
    let start = Instant::now();
    for _ in 0..N {
        let s = Instant::now();
        ticks.push(tick(c).await?);
        lat.push(s.elapsed());
    }
    let wall = start.elapsed().as_secs_f64();
    let span = ticks[N - 1] - ticks[0] + 1;
    println!("## Мелкие команды подряд ({N} шт.)\n");
    println!("- команд за тик: {:.2}", N as f64 / span as f64);
    println!("- команд в секунду: {:.0}", N as f64 / wall);
    println!(
        "- задержка p50 / p95: {:.2} / {:.2} мс\n",
        pct(&mut lat, 0.5),
        pct(&mut lat, 0.95)
    );
    Ok(())
}

/// Обратное направление: ответ клиента ядру заданного размера.
async fn response_size(c: &mut Conn, t: &Target) -> R<()> {
    println!("## Размер ответа (rcon.print)\n");
    println!("| байт | p50 мс | МБ/с |");
    println!("|---:|---:|---:|");
    for size in [1_000usize, 100_000, 1_000_000, 4_000_000] {
        let json = format!(r#"{{"op":"echo","n":{size}}}"#);
        let mut lat = Vec::new();
        for _ in 0..5 {
            let s = Instant::now();
            match fd(c, &json).await {
                Ok(r) if r.trim_end().len() == size => lat.push(s.elapsed()),
                Ok(r) => {
                    println!("| {size} | обрезано до {} байт | |", r.trim_end().len());
                    break;
                }
                Err(e) => {
                    println!("| {size} | ошибка: {e} | |");
                    *c = connect(t).await?;
                    break;
                }
            }
        }
        if lat.len() == 5 {
            let p50 = pct(&mut lat, 0.5);
            println!("| {size} | {p50:.2} | {:.1} |", size as f64 / 1e6 / (p50 / 1000.0));
        }
    }
    println!();
    Ok(())
}

/// Пачка обновлений сундуков: только разбор (parse) и с применением (apply).
/// Каждый режим гоняется 5 секунд подряд; UPS — сколько тиков прошло за это время.
async fn apply(c: &mut Conn) -> R<()> {
    println!("## Пачки обновлений сундуков, 5 с непрерывно\n");
    println!("| сундуков | режим | байт/команду | команд/с | обновлений/с | МБ/с | p50 мс | p95 мс | UPS |");
    println!("|---:|---|---:|---:|---:|---:|---:|---:|---:|");
    for n in [100usize, 1_000, 10_000, 50_000] {
        fd(c, &format!(r#"{{"op":"bench_setup","n":{n}}}"#)).await?;
        for mode in ["parse", "apply"] {
            // Два набора чисел по очереди, чтобы содержимое сундуков реально менялось.
            let payloads: Vec<String> = (0..2)
                .map(|k| {
                    let list: Vec<String> = (1..=n)
                        .map(|i| format!("[{i},{}]", (i + k * 7) % 50))
                        .collect();
                    format!(r#"{{"op":"bench_{mode}","u":[{}]}}"#, list.join(","))
                })
                .collect();
            let mut lat = Vec::new();
            let mut bytes = 0usize;
            let t0 = tick(c).await?;
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                let p = &payloads[lat.len() % 2];
                let s = Instant::now();
                fd(c, p).await?;
                lat.push(s.elapsed());
                bytes += p.len() + 4;
            }
            let wall = start.elapsed().as_secs_f64();
            let ups = (tick(c).await? - t0) as f64 / wall;
            let cmds = lat.len() as f64 / wall;
            println!(
                "| {n} | {mode} | {} | {cmds:.1} | {:.0} | {:.2} | {:.2} | {:.2} | {ups:.1} |",
                payloads[0].len() + 4,
                cmds * n as f64,
                bytes as f64 / 1e6 / wall,
                pct(&mut lat, 0.5),
                pct(&mut lat, 0.95)
            );
        }
    }
    println!();
    Ok(())
}

/// Частота синтетических «действий игрока»: каждые `every` тиков по `per_tick` событий.
struct Rate {
    name: &'static str,
    every: u32,
    per_tick: u32,
}

const RATES: [Rate; 3] = [
    Rate { name: "1 за 10 тиков", every: 10, per_tick: 1 },
    Rate { name: "10 за тик", every: 1, per_tick: 10 },
    Rate { name: "1000 за тик", every: 1, per_tick: 1000 },
];

const EVENTS_RUN: Duration = Duration::from_secs(10);

/// Клиент → ядро: опрос `/fd poll` каждый тик против чтения файла, который пишет мод.
/// Задержка — от тика, в котором событие создано, до получения ядром.
async fn events(c: &mut Conn, env_dir: &Path) -> R<()> {
    println!("## Клиент → ядро: события, 10 с на прогон\n");
    println!("| частота | способ | получено | потеряно | p50 мс | p95 мс | макс мс | КБ/с | UPS |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|");
    for rate in &RATES {
        for mode in ["poll", "file"] {
            run_events(c, env_dir, rate, mode).await?;
        }
    }
    println!(
        "\nТочность задержки ±1 тик (~17 мс): время тика события восстанавливается по \
         опорной точке «тик ↔ часы» (для poll — из того же ответа, для file — запрос тика раз в 0.5 с).\n"
    );
    Ok(())
}

/// Тик и момент на часах ядра, когда игра его выполняла (середина запроса).
async fn anchor(c: &mut Conn) -> R<(u64, Instant)> {
    let s = Instant::now();
    let t = tick(c).await?;
    Ok((t, s + s.elapsed() / 2))
}

/// Разбирает событие {"t": тик, "i": номер}.
fn ev_fields(v: &serde_json::Value) -> Option<(f64, u64)> {
    Some((v.get("t")?.as_f64()?, v.get("i")?.as_f64()? as u64))
}

async fn run_events(c: &mut Conn, env_dir: &Path, rate: &Rate, mode: &str) -> R<()> {
    let file = env_dir.join("script-output").join("fd-events.jsonl");
    let _ = std::fs::remove_file(&file);
    fd(c, &format!(
        r#"{{"op":"ev_start","every":{},"per_tick":{},"mode":"{mode}"}}"#,
        rate.every, rate.per_tick
    ))
    .await?;

    let mut lat: Vec<f64> = Vec::new();
    let mut received = 0u64;
    let mut bytes = 0usize;
    let t0 = tick(c).await?;
    let start = Instant::now();

    if mode == "poll" {
        while start.elapsed() < EVENTS_RUN {
            let s = Instant::now();
            let r = fd(c, r#"{"op":"poll"}"#).await?;
            let rtt_half = ms(s.elapsed()) / 2.0;
            bytes += r.len();
            let v: serde_json::Value = serde_json::from_str(r.trim())?;
            let now = v["now"].as_f64().ok_or("poll: нет now")?;
            // Пустая очередь приходит как {} — это не массив, событий нет.
            for e in v["ev"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                if let Some((t, _)) = ev_fields(e) {
                    lat.push(rtt_half + (now - t) * TICK_MS);
                    received += 1;
                }
            }
        }
    } else {
        let mut pos = 0u64;
        let mut tail = String::new();
        let (mut at, mut aw) = anchor(c).await?;
        let mut last_anchor = Instant::now();
        while start.elapsed() < EVENTS_RUN {
            if last_anchor.elapsed() > Duration::from_millis(500) {
                (at, aw) = anchor(c).await?;
                last_anchor = Instant::now();
            }
            if let Ok(mut f) = std::fs::File::open(&file) {
                f.seek(SeekFrom::Start(pos))?;
                let mut chunk = String::new();
                pos += f.read_to_string(&mut chunk)? as u64;
                let arrival = Instant::now();
                bytes += chunk.len();
                tail.push_str(&chunk);
                // Обрабатываем только целые строки, хвост ждёт следующего чтения.
                if let Some(cut) = tail.rfind('\n') {
                    for line in tail[..cut].lines() {
                        let v: serde_json::Value = serde_json::from_str(line)?;
                        if let Some((t, _)) = ev_fields(&v) {
                            let born = ms(arrival.duration_since(aw)) - (t - at as f64) * TICK_MS;
                            lat.push(born);
                            received += 1;
                        }
                    }
                    tail.drain(..=cut);
                }
            }
            // std-сон: на Windows он точный (~1 мс), в отличие от таймера tokio.
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    let wall = start.elapsed().as_secs_f64();
    let ups = (tick(c).await? - t0) as f64 / wall;
    let generated: u64 = fd(c, r#"{"op":"ev_stop"}"#).await?.trim().parse()?;
    // Созданные в последние миллисекунды прогона ещё в пути — не считаем их потерей.
    let in_flight = (rate.per_tick as u64) * 3;
    let lost = generated.saturating_sub(received + in_flight);
    if lat.is_empty() {
        println!("| {} | {mode} | 0 | {generated} | | | | | {ups:.1} |", rate.name);
        return Ok(());
    }
    println!(
        "| {} | {mode} | {received} | {lost} | {:.1} | {:.1} | {:.1} | {:.0} | {ups:.1} |",
        rate.name,
        pct_f(&mut lat, 0.5),
        pct_f(&mut lat, 0.95),
        pct_f(&mut lat, 1.0),
        bytes as f64 / 1e3 / wall
    );
    Ok(())
}

const LOOPS: usize = 5000;

/// Заморозка `disabled_by_script`: какие сущности её принимают, останавливаются ли
/// конвейеры, сколько UPS экономит и сколько стоит переключение.
async fn freeze(c: &mut Conn) -> R<()> {
    println!("## Заморозка (`disabled_by_script`)
");

    let v: serde_json::Value = serde_json::from_str(fd(c, r#"{"op":"fz_probe"}"#).await?.trim())?;
    println!("### Какие сущности её принимают
");
    println!("| сущность | is_updatable | disabled после записи | active |");
    println!("|---|---|---|---|");
    for (name, r) in v.as_object().ok_or("fz_probe: не объект")? {
        match r.as_array() {
            Some(a) => println!("| {name} | {} | {} | {} |", a[0], a[1], a[2]),
            None => println!("| {name} | {r} | | |"),
        }
    }

    let setup = fd(c, &format!(r#"{{"op":"fz_loops_setup","n":{LOOPS}}}"#)).await?;
    println!("
### {LOOPS} колец 2×2 с плитами: {}
", setup.trim());

    println!("- до заморозки предметы едут: {}", moving(c).await?);
    let n = fd(c, r#"{"op":"fz_set","disabled":true}"#).await?;
    println!("- заморожено (читаются как disabled): {}", n.trim());
    println!("- после заморозки предметы едут: {}", moving(c).await?);
    fd(c, r#"{"op":"fz_set","disabled":false}"#).await?;
    println!("- после разморозки едут: {}
", moving(c).await?);

    // Цена переключения: команда с repeat повторами минус пустая команда.
    let base = latency_p50(c, r#"{"op":"ping"}"#, 10).await?;
    let belts = LOOPS * 4;
    let rep = 10;
    let all = latency_p50(c, &format!(r#"{{"op":"fz_set","disabled":false,"repeat":{rep}}}"#), 5).await?;
    let per_toggle_us = (all - base) * 1000.0 / (rep * belts) as f64;
    // Полоса на краю окна: 1 чанк шириной на высоту ~экрана.
    let strip = r#""x0":0,"y0":300,"x1":32,"y1":364"#;
    let found = fd(c, &format!(r#"{{"op":"fz_area_set","disabled":false,{strip}}}"#)).await?;
    let srep = 100;
    let area = latency_p50(c, &format!(r#"{{"op":"fz_area_set","disabled":false,"repeat":{srep},{strip}}}"#), 5).await?;
    println!("### Цена переключения
");
    println!("- одно переключение по готовому списку: {per_toggle_us:.2} мкс ({belts} конвейеров × {rep})");
    println!(
        "- полоса 32×64 клетки (поиск + переключение, {} конвейеров): {:.3} мс
",
        found.trim(),
        (area - base) / srep as f64
    );

    // Сколько UPS съедают кольца: ускоряем игру и меряем, сколько тиков она успевает.
    println!("### Максимальный UPS (game.speed = 1000)
");
    println!("| состояние | UPS |");
    println!("|---|---:|");
    fd(c, r#"{"op":"fz_speed","speed":1000}"#).await?;
    println!("| {belts} конвейеров едут | {:.0} |", max_ups(c).await?);
    fd(c, r#"{"op":"fz_set","disabled":true}"#).await?;
    println!("| {belts} конвейеров заморожены | {:.0} |", max_ups(c).await?);
    fd(c, r#"{"op":"fz_clear"}"#).await?;
    println!("| конвейеров нет | {:.0} |", max_ups(c).await?);
    fd(c, r#"{"op":"fz_speed","speed":1}"#).await?;
    println!();
    Ok(())
}

/// Едут ли предметы: два снимка позиций с разницей ~20 тиков.
async fn moving(c: &mut Conn) -> R<bool> {
    let a = fd(c, r#"{"op":"fz_snapshot"}"#).await?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let b = fd(c, r#"{"op":"fz_snapshot"}"#).await?;
    Ok(a != b)
}

async fn latency_p50(c: &mut Conn, json: &str, reps: usize) -> R<f64> {
    let mut lat = Vec::with_capacity(reps);
    for _ in 0..reps {
        let s = Instant::now();
        fd(c, json).await?;
        lat.push(s.elapsed());
    }
    Ok(pct(&mut lat, 0.5))
}

/// Тиков в секунду за 3 с (имеет смысл при большом game.speed).
async fn max_ups(c: &mut Conn) -> R<f64> {
    let (t0, w0) = (tick(c).await?, Instant::now());
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (t1, w1) = (tick(c).await?, Instant::now());
    Ok((t1 - t0) as f64 / (w1 - w0).as_secs_f64())
}
