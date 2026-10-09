//! Снимает эталон из настоящей игры: строит сцену модом, пишет состояние каждый тик,
//! копирует файл эталона рядом со сценой.
//!
//!   cargo run --release --bin scene_capture -- tests/fidelity/straight-belt.scene.json [...]
//!
//! Нужен сервер из bench/run-server.ps1 (мод factorio-distributed, RCON на 127.0.0.1).
//! Окружение — FD_ENV_DIR (по умолчанию `../bench-env`): пароль RCON и script-output.
//! Скорость игры на время съёмки — FD_SPEED (по умолчанию 20), потом возвращается 1.
//! Все команды — `/fd <JSON>`, никакого `/c`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rcon::Connection;
use tokio::net::TcpStream;

type Conn = Connection<TcpStream>;
type R<T> = Result<T, Box<dyn std::error::Error>>;

async fn fd(c: &mut Conn, json: &serde_json::Value) -> R<String> {
    let r = c.cmd(&format!("/fd {json}")).await?;
    Ok(r.trim().to_string())
}

async fn fd_ok(c: &mut Conn, json: &serde_json::Value) -> R<String> {
    let r = fd(c, json).await?;
    if r.starts_with("error:") {
        return Err(format!("мод ответил: {r}").into());
    }
    Ok(r)
}

async fn capture(c: &mut Conn, scene_path: &Path, env_dir: &Path, out_dir: &Path) -> R<()> {
    let scene: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(scene_path)?)?;
    let name = scene["name"].as_str().ok_or("в сцене нет name")?.to_string();
    let ticks = scene["ticks"].as_u64().ok_or("в сцене нет ticks")?;
    let file = format!("{name}.dump.jsonl");

    // Старый эталон в script-output убираем, чтобы не принять его за новый.
    let src = env_dir.join("script-output").join(&file);
    let _ = std::fs::remove_file(&src);

    let built = fd_ok(c, &serde_json::json!({"op": "scene_build", "scene": scene})).await?;
    println!("{name}: scene_build -> {built}");

    // Манипуляторам нужно прогреться (энергия), мод просит подождать.
    let started = Instant::now();
    loop {
        let r = fd(c, &serde_json::json!({"op": "scene_run", "ticks": ticks, "file": file})).await?;
        if r == "ok" {
            break;
        }
        if !r.contains("not ready") || started.elapsed() > Duration::from_secs(60) {
            return Err(format!("scene_run: {r}").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    loop {
        let s = fd_ok(c, &serde_json::json!({"op": "scene_status"})).await?;
        if s.starts_with("done") {
            break;
        }
        if s.starts_with("error") || s == "idle" {
            return Err(format!("запись остановилась: {s}").into());
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err("запись не завершилась за 300 с".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    std::fs::create_dir_all(out_dir)?;
    let dst = out_dir.join(&file);
    std::fs::copy(&src, &dst)?;
    let lines = std::fs::read_to_string(&dst)?.lines().count();
    println!(
        "{name}: {} строк (ожидалось {}), {} байт -> {}",
        lines,
        ticks + 1,
        std::fs::metadata(&dst)?.len(),
        dst.display()
    );
    if lines as u64 != ticks + 1 {
        return Err("число строк эталона не совпало с ticks + 1".into());
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `scene_capture raw '{"op":"scene_info"}'` — одна произвольная команда /fd, ответ в stdout.
    let raw = args.first().map(String::as_str) == Some("raw");
    let scenes: Vec<PathBuf> = if raw { Vec::new() } else { args.iter().map(PathBuf::from).collect() };
    if scenes.is_empty() && !raw {
        return Err("использование: scene_capture <name>.scene.json [...]".into());
    }
    let addr = std::env::var("FD_RCON_ADDR").unwrap_or_else(|_| "127.0.0.1:27015".into());
    let env_dir = PathBuf::from(std::env::var("FD_ENV_DIR").unwrap_or_else(|_| "../bench-env".into()));
    let speed: f64 = std::env::var("FD_SPEED").ok().and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let password = std::fs::read_to_string(env_dir.join("rcon-password.txt"))?.trim().to_string();

    let mut c = Connection::<TcpStream>::builder()
        .enable_factorio_quirks(true)
        .connect(addr.as_str(), &password)
        .await?;
    let pong = fd_ok(&mut c, &serde_json::json!({"op": "ping"})).await?;
    if pong != "pong" {
        return Err(format!("мод не ответил на ping: {pong:?}").into());
    }

    if raw {
        let msg: serde_json::Value = serde_json::from_str(args.get(1).ok_or("raw: нужен JSON")?)?;
        println!("{}", fd(&mut c, &msg).await?);
        return Ok(());
    }
    fd_ok(&mut c, &serde_json::json!({"op": "speed", "speed": speed})).await?;
    let mut result = Ok(());
    for scene in &scenes {
        let out_dir = scene.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        if let Err(e) = capture(&mut c, scene, &env_dir, &out_dir).await {
            result = Err(e);
            break;
        }
    }
    let _ = fd(&mut c, &serde_json::json!({"op": "speed", "speed": 1})).await;
    result
}
