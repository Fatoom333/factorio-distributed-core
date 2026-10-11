//! Снимает эталоны из настоящей игры: строит сцены модом (каждую на своей поверхности),
//! сверяет геометрию модели с числами игры, пишет состояние каждый тик, копирует файлы
//! эталонов рядом со сценами.
//!
//!   cargo run --release --bin scene_capture -- tests/fidelity/straight-belt.scene.json [...]
//!
//! Все сцены из командной строки строятся и пишутся одновременно (общий t = 0, `scene_run_all`).
//! Нужен сервер из bench/run-server.ps1 (мод factorio-distributed, RCON на 127.0.0.1).
//! Окружение — FD_ENV_DIR (по умолчанию `../bench-env`): пароль RCON и script-output.
//! Скорость игры на время съёмки — FD_SPEED (по умолчанию 20), потом возвращается прежняя.
//! FD_RCON_ADDR — адрес RCON; не loopback — только с FD_ALLOW_REMOTE=1 (пароль RCON идёт
//! открытым текстом). Все команды — `/fd <JSON>`, никакого `/c`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::layout::{Scene, WARMUP_SETTLED};
use factorio_distributed_core::world::World;
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

struct Job {
    name: String,
    ticks: u64,
    json: serde_json::Value,
    world: World,
    out_dir: PathBuf,
}

fn load_job(path: &Path) -> R<Job> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    let scene = Scene::from_json(&text)?;
    if !fidelity::valid_scene_name(&scene.name) {
        return Err(format!("{}: плохое имя сцены {:?}", path.display(), scene.name).into());
    }
    let world = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled()).map_err(|e| format!("{}: {e}", path.display()))?;
    let out_dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    Ok(Job { name: scene.name.clone(), ticks: scene.ticks, json, world, out_dir })
}

/// Ждём, пока после постройки сцен пройдёт не меньше `WARMUP_SETTLED` тиков игры — столько же,
/// сколько модель проигрывает прогрев сцены без поля `warmup`. Первые до 600 тиков после постройки
/// каждая лента — своя линия, а руки едут к точкам захвата; от длины ожидания зависело бы
/// начальное состояние (docs/mechanics/belts.md, «Состояние после постройки»). После
/// `WARMUP_SETTLED` тиков цепочки слиты, заявок нет, руки на месте — это начальное состояние модели.
async fn wait_settled(c: &mut Conn) -> R<()> {
    let tick = |s: String| s.trim().parse::<u64>().map_err(|_| format!("tick: {s}"));
    let built = tick(fd(c, &serde_json::json!({"op": "tick"})).await?)?;
    let started = Instant::now();
    while tick(fd(c, &serde_json::json!({"op": "tick"})).await?)? < built + WARMUP_SETTLED {
        if started.elapsed() > Duration::from_secs(120) {
            return Err(format!("игра не прошла {WARMUP_SETTLED} тиков за 120 с после постройки сцен").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

async fn capture(c: &mut Conn, jobs: &[Job], env_dir: &Path, speed: f64) -> R<()> {
    // Старые эталоны в script-output убираем, чтобы не принять их за новые.
    for j in jobs {
        let _ = std::fs::remove_file(env_dir.join("script-output").join(format!("{}.dump.jsonl", j.name)));
    }
    let scenes: Vec<&serde_json::Value> = jobs.iter().map(|j| &j.json).collect();
    let built = fd_ok(c, &serde_json::json!({"op": "scene_build_many", "scenes": scenes})).await?;
    println!("scene_build_many -> {built}");

    // Геометрия модели = числа игры (drop/pickup_position, get_line_item_position).
    for j in jobs {
        let info: serde_json::Value =
            serde_json::from_str(&fd_ok(c, &serde_json::json!({"op": "scene_info", "name": j.name})).await?)?;
        fidelity::check_geometry(&j.world, &info).map_err(|e| format!("{}: геометрия: {e}", j.name))?;
    }
    println!("геометрия модели совпала с игрой ({} сцен)", jobs.len());

    // Линии лент после постройки сливаются не сразу — ждём (сцены с полем `warmup` модель
    // строит с заданным прогревом; выдержать его точно съёмщик не умеет и не ждёт).
    if jobs.iter().all(|j| j.json.get("warmup").map_or(true, |w| w.is_null())) {
        wait_settled(c).await?;
    } else {
        println!("внимание: в сцене задан warmup — съёмщик не выдерживает его точно");
    }
    // Манипуляторам нужно прогреться (энергия), мод просит подождать.
    let started = Instant::now();
    loop {
        let r = fd(c, &serde_json::json!({"op": "scene_run_all"})).await?;
        if r.starts_with("ok") {
            break;
        }
        if !r.contains("not ready") || started.elapsed() > Duration::from_secs(60) {
            return Err(format!("scene_run_all: {r}").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Таймаут — по самой длинной сцене: втрое дольше расчётного времени и 30 с запаса.
    let max_ticks = jobs.iter().map(|j| j.ticks).max().unwrap_or(0);
    let limit = Duration::from_secs_f64(max_ticks as f64 / (60.0 * speed) * 3.0 + 30.0);
    let started = Instant::now();
    loop {
        let st: serde_json::Value =
            serde_json::from_str(&fd_ok(c, &serde_json::json!({"op": "scene_status_all"})).await?)?;
        let mut all_done = true;
        for j in jobs {
            let s = st[&j.name].as_str().unwrap_or("idle");
            if s.starts_with("error") || s == "idle" {
                return Err(format!("{}: запись остановилась: {s}", j.name).into());
            }
            all_done &= s.starts_with("done");
        }
        if all_done {
            break;
        }
        if started.elapsed() > limit {
            return Err(format!("запись не завершилась за {} с", limit.as_secs()).into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    for j in jobs {
        let file = format!("{}.dump.jsonl", j.name);
        std::fs::create_dir_all(&j.out_dir)?;
        let dst = j.out_dir.join(&file);
        std::fs::copy(env_dir.join("script-output").join(&file), &dst)?;
        let lines = std::fs::read_to_string(&dst)?.lines().count();
        println!(
            "{}: {} строк (ожидалось {}), {} байт -> {}",
            j.name,
            lines,
            j.ticks + 1,
            std::fs::metadata(&dst)?.len(),
            dst.display()
        );
        if lines as u64 != j.ticks + 1 {
            return Err(format!("{}: число строк эталона не совпало с ticks + 1", j.name).into());
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `scene_capture raw '{"op":"scene_info","name":"..."}'` — одна команда /fd, ответ в stdout.
    let raw = args.first().map(String::as_str) == Some("raw");
    let paths: Vec<PathBuf> = if raw { Vec::new() } else { args.iter().map(PathBuf::from).collect() };
    if paths.is_empty() && !raw {
        return Err("использование: scene_capture <name>.scene.json [...]".into());
    }
    let jobs = paths.iter().map(|p| load_job(p)).collect::<R<Vec<Job>>>()?;
    {
        let mut names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err("имена сцен повторяются".into());
        }
    }

    let addr: SocketAddr = std::env::var("FD_RCON_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:27015".into())
        .parse()
        .map_err(|e| format!("FD_RCON_ADDR: {e}"))?;
    if !addr.ip().is_loopback() {
        if std::env::var("FD_ALLOW_REMOTE").as_deref() != Ok("1") {
            return Err("FD_RCON_ADDR не loopback: пароль RCON пойдёт открытым текстом; нужен FD_ALLOW_REMOTE=1".into());
        }
        eprintln!("внимание: RCON на {addr} — пароль идёт открытым текстом");
    }
    let env_dir = PathBuf::from(std::env::var("FD_ENV_DIR").unwrap_or_else(|_| "../bench-env".into()));
    let speed: f64 = std::env::var("FD_SPEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v: &f64| v.is_finite() && (0.01..=1000.0).contains(v))
        .unwrap_or(20.0);
    let pw_path = env_dir.join("rcon-password.txt");
    let password = std::fs::read_to_string(&pw_path)
        .map_err(|e| format!("{}: {e}", pw_path.display()))?
        .trim()
        .to_string();

    let mut c = Connection::<TcpStream>::builder()
        .enable_factorio_quirks(true)
        .connect(addr, &password)
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
    // Прежняя скорость игры (операция `speed` без поля — только чтение).
    let old_speed: f64 = fd_ok(&mut c, &serde_json::json!({"op": "speed"})).await?.parse().unwrap_or(1.0);
    fd_ok(&mut c, &serde_json::json!({"op": "speed", "speed": speed})).await?;
    let result = capture(&mut c, &jobs, &env_dir, speed).await;
    if result.is_err() {
        // Запись могла остаться идти — отменяем, чтобы мод не был занят.
        let _ = fd(&mut c, &serde_json::json!({"op": "scene_abort", "all": true})).await;
    }
    for j in &jobs {
        let _ = fd(&mut c, &serde_json::json!({"op": "scene_clear", "name": j.name})).await;
    }
    let _ = fd(&mut c, &serde_json::json!({"op": "speed", "speed": old_speed})).await;
    result
}
