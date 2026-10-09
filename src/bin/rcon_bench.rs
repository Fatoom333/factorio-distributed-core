//! Замер пропускной способности RCON Factorio (TAR-166).
//!
//! Нужен сервер Factorio с модом factorio-distributed и RCON на 127.0.0.1
//! (см. bench/README.md). Запуск:
//!
//!   FD_RCON_PASSWORD_FILE=<путь> cargo run --release --bin rcon_bench
//!
//! Все команды — `/fd <JSON>` (команда мода, достижения не отключает).
//! Печатает markdown-таблицы: задержка и скорость по размеру команды,
//! сколько команд Factorio успевает за тик, размер ответа, применение
//! пачек обновлений к сундукам и UPS под нагрузкой.

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> R<()> {
    let addr = std::env::var("FD_RCON_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".into());
    let pw_file = std::env::var("FD_RCON_PASSWORD_FILE")
        .map_err(|_| "задайте FD_RCON_PASSWORD_FILE — путь к файлу с паролем RCON")?;
    let password = std::fs::read_to_string(&pw_file)?.trim().to_string();
    let target = Target { addr, password };

    let mut c = connect(&target).await?;
    let pong = fd(&mut c, r#"{"op":"ping"}"#).await?;
    if pong.trim() != "pong" {
        return Err(format!("мод не ответил на ping: {pong:?}").into());
    }

    idle_ups(&mut c).await?;
    raw_payload(&mut c, &target).await?;
    cmds_per_tick(&mut c).await?;
    response_size(&mut c, &target).await?;
    apply(&mut c).await?;
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
