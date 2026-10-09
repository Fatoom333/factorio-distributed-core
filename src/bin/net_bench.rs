//! Замер сети агент ↔ ядро (TAR-166): задержка и скорость TCP-канала.
//!
//! Сервер — эхо кадров `[u32 длина LE][данные]`:
//!   net_bench server 127.0.0.1:27100
//! Клиент — пинг-понг кадрами разного размера и поток «60 кадров в секунду»:
//!   net_bench client 127.0.0.1:27100
//!
//! Только стандартная библиотека. Сервер по умолчанию слушает лишь 127.0.0.1;
//! для замера через интернет его кладут за SSH-туннель.

use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type R<T> = Result<T, Box<dyn std::error::Error>>;

/// Больше кадр не принимаем — защита сервера от запроса на гигабайты памяти.
const MAX_FRAME: usize = 16 * 1024 * 1024;

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(2).map(String::as_str).unwrap_or("127.0.0.1:27100");
    match args.get(1).map(String::as_str) {
        Some("server") => server(addr),
        Some("client") => client(addr),
        _ => Err("использование: net_bench server|client [адрес:порт]".into()),
    }
}

fn read_frame(r: &mut impl Read, buf: &mut Vec<u8>) -> std::io::Result<()> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "кадр слишком большой"));
    }
    buf.resize(len, 0);
    r.read_exact(buf)
}

fn write_frame(w: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    w.flush()
}

fn server(addr: &str) -> R<()> {
    let listener = TcpListener::bind(addr)?;
    println!("net_bench: слушаю {addr}");
    for conn in listener.incoming() {
        let conn = conn?;
        std::thread::spawn(move || {
            let _ = conn.set_nodelay(true);
            let mut r = BufReader::new(conn.try_clone().expect("clone"));
            let mut w = conn;
            let mut buf = Vec::new();
            while read_frame(&mut r, &mut buf).is_ok() {
                if write_frame(&mut w, &buf).is_err() {
                    break;
                }
            }
        });
    }
    Ok(())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn client(addr: &str) -> R<()> {
    let conn = TcpStream::connect(addr)?;
    conn.set_nodelay(true)?;
    let mut r = BufReader::new(conn.try_clone()?);
    let mut w = conn.try_clone()?;
    let mut buf = Vec::new();

    println!("## Пинг-понг (кадр туда и обратно)\n");
    println!("| байт | повторов | p50 мс | p95 мс | макс мс | МБ/с (туда+обратно, по p50) |");
    println!("|---:|---:|---:|---:|---:|---:|");
    for size in [64usize, 1_000, 10_000, 100_000, 1_000_000] {
        let data = vec![b'x'; size];
        let reps = if size >= 1_000_000 { 20 } else { 100 };
        let mut lat = Vec::with_capacity(reps);
        for _ in 0..reps {
            let s = Instant::now();
            write_frame(&mut w, &data)?;
            read_frame(&mut r, &mut buf)?;
            lat.push(ms(s.elapsed()));
        }
        let p50 = pct(&mut lat, 0.5);
        println!(
            "| {size} | {reps} | {p50:.1} | {:.1} | {:.1} | {:.1} |",
            pct(&mut lat, 0.95),
            pct(&mut lat, 1.0),
            2.0 * size as f64 / 1e6 / (p50 / 1000.0)
        );
    }

    // Поток как в игре: кадр каждые 16.7 мс, не дожидаясь ответа; RTT каждого кадра.
    for size in [1_000usize, 10_000] {
        const N: usize = 600;
        let sent: Arc<Mutex<Vec<Option<Instant>>>> = Arc::new(Mutex::new(vec![None; N]));
        let sent_w = Arc::clone(&sent);
        let mut w2 = conn.try_clone()?;
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            let start = Instant::now();
            for i in 0..N {
                let due = start + Duration::from_micros(16_667 * i as u64);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                let mut data = vec![b'x'; size];
                data[..8].copy_from_slice(&(i as u64).to_le_bytes());
                sent_w.lock().unwrap()[i] = Some(Instant::now());
                write_frame(&mut w2, &data)?;
            }
            Ok(())
        });
        let mut rtt = Vec::with_capacity(N);
        for _ in 0..N {
            read_frame(&mut r, &mut buf)?;
            let now = Instant::now();
            let i = u64::from_le_bytes(buf[..8].try_into()?) as usize;
            if let Some(t) = sent.lock().unwrap()[i] {
                rtt.push(ms(now - t));
            }
        }
        writer.join().map_err(|_| "поток записи упал")??;
        let late = rtt.iter().filter(|&&x| x > 2.0 * 16.667).count();
        println!(
            "\n## Поток 60 кадров/с по {size} байт ({N} кадров)\n\n- RTT p50 / p95 / макс: {:.1} / {:.1} / {:.1} мс\n- кадров с RTT больше 2 тиков: {late}",
            pct(&mut rtt, 0.5),
            pct(&mut rtt, 0.95),
            pct(&mut rtt, 1.0)
        );
    }
    println!();
    Ok(())
}
