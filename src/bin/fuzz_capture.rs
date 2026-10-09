//! Съёмщик эталонов пачками для фаззера M0: сцены уходят в мод операциями `scene_build_many` /
//! `scene_run_all` (каждая на своей поверхности, общий t = 0), дампы забираются из script-output.
//!
//!   cargo run --release --bin fuzz_capture -- --seed 1 --count 1000 --batch 100
//!   cargo run --release --bin fuzz_capture -- --scenes tests/fidelity --out /tmp/ref
//!
//! Откуда сцены: генератор (--seed, --count, --turns, --no-vectors) или папка (--scenes DIR,
//! файлы *.scene.json). Куда: --out DIR, иначе `$FD_RUNS_DIR/<seed>` (FD_RUNS_DIR по умолчанию
//! `./fuzz-runs`; папка прогонов лежит вне git). Уже снятые сцены (есть и .scene.json, и
//! .dump.jsonl) пропускаются, поэтому прогон можно продолжать.
//!
//! Окружение: FD_RCON_ADDR (127.0.0.1:27015), FD_ENV_DIR (../bench-env: пароль RCON и
//! script-output), скорость игры — --speed (по умолчанию 30; потом возвращается 1).
//! Все команды — `/fd <JSON>`, никакого `/c`.

use factorio_distributed_core::fidelity;
use factorio_distributed_core::fuzz;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rcon::Connection;
use serde_json::{json, Value};
use tokio::net::TcpStream;

type Conn = Connection<TcpStream>;
type R<T> = Result<T, Box<dyn std::error::Error>>;

async fn fd(c: &mut Conn, msg: &Value) -> R<String> {
    Ok(c.cmd(&format!("/fd {msg}")).await?.trim().to_string())
}

async fn fd_ok(c: &mut Conn, msg: &Value) -> R<String> {
    let r = fd(c, msg).await?;
    if r.starts_with("error:") {
        return Err(format!("мод ответил: {r}").into());
    }
    Ok(r)
}

fn load_scenes_dir(dir: &Path) -> R<Vec<Value>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".scene.json")))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        out.push(serde_json::from_str(&std::fs::read_to_string(f)?)?);
    }
    Ok(out)
}

fn move_file(src: &Path, dst: &Path) -> R<()> {
    if std::fs::rename(src, dst).is_err() {
        std::fs::copy(src, dst)?;
        std::fs::remove_file(src)?;
    }
    Ok(())
}

/// Один пакет: scene_build_many -> scene_run_all (после прогрева) -> ждём конца всех сцен ->
/// забираем дампы -> scene_clear. Возвращает число снятых сцен и список отказов.
async fn run_batch(c: &mut Conn, scenes: &[Value], env_dir: &Path, out_dir: &Path) -> R<(usize, Vec<String>)> {
    let out = env_dir.join("script-output");
    for s in scenes {
        let _ = std::fs::remove_file(out.join(format!("{}.dump.jsonl", s["name"].as_str().unwrap_or_default())));
    }
    let _ = fd(c, &json!({"op": "scene_clear", "all": true})).await?;
    let r = fd(c, &json!({"op": "scene_build_many", "scenes": scenes})).await?;
    if !r.starts_with("ok") {
        let _ = fd(c, &json!({"op": "scene_clear", "all": true})).await;
        return Err(format!("scene_build_many: {r}").into());
    }
    let started = Instant::now();
    loop {
        let r = fd(c, &json!({"op": "scene_run_all", "ext": true})).await?;
        if r.starts_with("ok") {
            break;
        }
        if !r.contains("not ready") || started.elapsed() > Duration::from_secs(120) {
            let _ = fd(c, &json!({"op": "scene_clear", "all": true})).await;
            return Err(format!("scene_run_all: {r}").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let statuses: serde_json::Map<String, Value> = loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let r = fd_ok(c, &json!({"op": "scene_status_all"})).await?;
        let m: serde_json::Map<String, Value> = serde_json::from_str(&r).unwrap_or_default();
        let busy = m.values().any(|v| v.as_str().is_some_and(|s| s.starts_with("running") || s == "idle"));
        if !busy {
            break m;
        }
        if started.elapsed() > Duration::from_secs(1800) {
            let _ = fd(c, &json!({"op": "scene_abort", "all": true})).await;
            return Err("пакет не завершился за 30 минут".into());
        }
    };
    let mut got = 0;
    let mut failed = Vec::new();
    for s in scenes {
        let name = s["name"].as_str().unwrap_or_default();
        let st = statuses.get(name).and_then(Value::as_str).unwrap_or("нет статуса");
        if !st.starts_with("done") {
            failed.push(format!("{name}: {st}"));
            continue;
        }
        let src = out.join(format!("{name}.dump.jsonl"));
        move_file(&src, &out_dir.join(format!("{name}.dump.jsonl"))).map_err(|e| format!("нет дампа {}: {e}", src.display()))?;
        std::fs::write(out_dir.join(format!("{name}.scene.json")), serde_json::to_string(s)? + "
")?;
        got += 1;
    }
    let _ = fd(c, &json!({"op": "scene_clear", "all": true})).await?;
    Ok((got, failed))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> R<()> {
    let a = fuzz::Args::parse();
    let seed: u64 = a.num("seed", 1);
    let batch: usize = a.num("batch", 100);
    let speed: f64 = a.num("speed", 30.0);
    let opts = a.opts();

    let scenes: Vec<Value> = if let Some(dir) = a.get("scenes") {
        load_scenes_dir(Path::new(dir))?
    } else {
        let count: u32 = a.num("count", 100);
        (0..count).map(|i| fuzz::generate(seed, i, opts).to_json()).collect()
    };
    let out_dir = match a.get("out") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("FD_RUNS_DIR").unwrap_or_else(|_| "fuzz-runs".into())).join(seed.to_string()),
    };
    std::fs::create_dir_all(&out_dir)?;

    // Имя сцены идёт в путь файла (дамп забирается из script-output, сцена пишется рядом) —
    // проверяем до всякой работы с файлами, как в scene_capture.
    for s in &scenes {
        let n = s["name"].as_str().unwrap_or_default();
        if !fidelity::valid_scene_name(n) {
            return Err(format!("плохое имя сцены {n:?} (допустимо [A-Za-z0-9_-], не длиннее 100)").into());
        }
    }

    // Что уже снято — пропускаем.
    let todo: Vec<Value> = scenes
        .into_iter()
        .filter(|s| {
            let n = s["name"].as_str().unwrap_or_default();
            !(out_dir.join(format!("{n}.scene.json")).exists() && out_dir.join(format!("{n}.dump.jsonl")).exists())
        })
        .collect();
    println!("к съёмке: {} сцен, пакеты по {batch}, папка {}", todo.len(), out_dir.display());
    if todo.is_empty() {
        return Ok(());
    }

    let addr = std::env::var("FD_RCON_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".into());
    let env_dir = PathBuf::from(std::env::var("FD_ENV_DIR").unwrap_or_else(|_| "../bench-env".into()));
    let password = std::fs::read_to_string(env_dir.join("rcon-password.txt"))?.trim().to_string();
    let mut c = Connection::<TcpStream>::builder()
        .enable_factorio_quirks(true)
        .connect(addr.as_str(), &password)
        .await?;
    let pong = fd_ok(&mut c, &json!({"op": "ping"})).await?;
    if pong != "pong" {
        return Err(format!("мод не ответил на ping: {pong:?}").into());
    }
    fd_ok(&mut c, &json!({"op": "speed", "speed": speed})).await?;

    let t0 = Instant::now();
    let mut total = 0usize;
    let mut total_ticks = 0u64;
    let mut all_failed: Vec<String> = Vec::new();
    let mut result: R<()> = Ok(());
    for (k, chunk) in todo.chunks(batch).enumerate() {
        let bt = Instant::now();
        match run_batch(&mut c, chunk, &env_dir, &out_dir).await {
            Ok((got, failed)) => {
                total += got;
                total_ticks += chunk.iter().filter_map(|s| s["ticks"].as_u64()).sum::<u64>();
                println!("пакет {k}: снято {got} из {} за {:.1} с", chunk.len(), bt.elapsed().as_secs_f64());
                for f in &failed {
                    println!("  отказ: {f}");
                }
                all_failed.extend(failed);
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    let _ = fd(&mut c, &json!({"op": "speed", "speed": 1})).await;
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "итого: {total} сцен, {total_ticks} тиков сцен, {:.1} с ({:.2} с на сцену, {:.0} сцен-тиков/с), отказов {}",
        secs,
        secs / total.max(1) as f64,
        total_ticks as f64 / secs,
        all_failed.len()
    );
    result
}
