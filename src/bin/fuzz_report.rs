//! Отчёт фаззера M0: для каждой снятой сцены строит модель ядра, гонит её тик в тик и сверяет
//! с эталоном из игры; сводит первые расхождения по видам.
//!
//!   cargo run --release --bin fuzz_report -- --run <папка прогона> [--threads 8] [--list]
//!       [--belts-only] [--warmup N]
//!
//! `--belts-only` — сверять только ленты (правила лент отдельно от руки манипулятора);
//! `--warmup N` — считать, что от постройки сцен до t = 0 прошло N тиков (поле `warmup` сцены).
//!
//! Вид расхождения — «сущность/поле» (`belt/lines`, `inserter/hand`, `chest/slots`, ...). По
//! каждому виду печатается число сцен и до трёх минимальных примеров (минимальный — с
//! наименьшим числом сущностей).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::fuzz;
use factorio_distributed_core::world::layout::{Scene, TickDump};
use factorio_distributed_core::world::World;

/// Первое расхождение модели и игры в одной сцене.
#[derive(Debug, Clone)]
pub struct FirstMismatch {
    pub tick: u64,
    /// «сущность/поле», например `belt/lines`, `inserter/hand`, `chest/items`.
    pub kind: String,
    pub detail: String,
}

fn tile(p: &factorio_distributed_core::world::layout::Pos) -> String {
    format!("[{}, {}]", p.0 as f64 / 256.0, p.1 as f64 / 256.0)
}

fn lines_str(l: &[Vec<factorio_distributed_core::world::layout::DumpItem>; 2]) -> String {
    l.iter()
        .map(|v| format!("[{}]", v.iter().map(|d| format!("{}@{}", d.item, d.p)).collect::<Vec<_>>().join(" ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Первое различие двух снимков: вид и описание.
fn first_diff(want: &TickDump, got: &TickDump) -> Option<(String, String)> {
    if want == got {
        return None;
    }
    if want.belts.len() != got.belts.len() || want.inserters.len() != got.inserters.len() || want.chests.len() != got.chests.len() {
        return Some(("scene/count".into(), "разное число сущностей".into()));
    }
    for (w, g) in want.belts.iter().zip(&got.belts) {
        if w != g {
            return Some(("belt/lines".into(), format!("лента {}: ожидалось {}, получилось {}", tile(&w.pos), lines_str(&w.lines), lines_str(&g.lines))));
        }
    }
    for (w, g) in want.inserters.iter().zip(&got.inserters) {
        if w != g {
            let field = if w.held != g.held || w.held_count != g.held_count {
                "held"
            } else {
                "hand"
            };
            return Some((
                format!("inserter/{field}"),
                format!(
                    "манипулятор {}: ожидалось {:?}×{} {:?}, получилось {:?}×{} {:?}",
                    tile(&w.pos),
                    w.held,
                    w.held_count,
                    w.hand,
                    g.held,
                    g.held_count,
                    g.hand
                ),
            ));
        }
    }
    for (w, g) in want.chests.iter().zip(&got.chests) {
        if w != g {
            let field = if w.items != g.items { "items" } else { "slots" };
            return Some((
                format!("chest/{field}"),
                format!("сундук {}: ожидалось {:?} {:?}, получилось {:?} {:?}", tile(&w.pos), w.items, w.slots, g.items, g.slots),
            ));
        }
    }
    Some(("other".into(), "отличие вне известных полей".into()))
}

/// Сравнение модели с эталоном. `Ok(None)` — совпало на всех тиках, `Ok(Some(..))` — первое
/// расхождение, `Err` — модель не умеет эту сцену.
fn compare(scene: &Scene, dump: &[TickDump], belts_only: bool) -> Result<Option<FirstMismatch>, String> {
    let mut world = World::from_scene(scene, factorio_distributed_core::world::proto::Prototypes::bundled()).map_err(|e| e.to_string())?;
    for (i, want) in dump.iter().enumerate() {
        if i > 0 {
            world.step();
        }
        let got = world.dump();
        if belts_only && want.belts == got.belts {
            continue;
        }
        if let Some((kind, detail)) = first_diff(want, &got) {
            return Ok(Some(FirstMismatch { tick: want.t, kind, detail }));
        }
    }
    Ok(None)
}

enum Outcome {
    Match,
    Mismatch(FirstMismatch),
    Unsupported(String),
    /// Модель упала с паникой (текст паники) — это ошибка модели, а не «не умеет».
    Panic(String),
    Broken,
}

/// Текст паники из `catch_unwind` (`&str` или `String`).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "паника без текста".to_string()
    }
}

fn check_one(run: &Path, name: &str, belts_only: bool, warmup: Option<u64>) -> (usize, Outcome) {
    let mut scene = match fidelity::load_scene(&run.join(format!("{name}.scene.json"))) {
        Ok(s) => s,
        Err(_) => return (0, Outcome::Broken),
    };
    if warmup.is_some() {
        scene.warmup = warmup;
    }
    let n = scene.entities.len();
    let dump = match fidelity::load_dump(&run.join(format!("{name}.dump.jsonl"))) {
        Ok(d) => d,
        Err(_) => return (n, Outcome::Broken),
    };
    if dump.len() as u64 != scene.ticks + 1 {
        return (n, Outcome::Broken);
    }
    let r = std::panic::catch_unwind(|| compare(&scene, &dump, belts_only));
    match r {
        Ok(Ok(None)) => (n, Outcome::Match),
        Ok(Ok(Some(m))) => (n, Outcome::Mismatch(m)),
        Ok(Err(why)) => (n, Outcome::Unsupported(why)),
        Err(p) => (n, Outcome::Panic(panic_message(p.as_ref()))),
    }
}

/// Расхождение по виду: сколько сцен и до трёх минимальных примеров (сущностей, имя сцены, расхождение).
type KindStats = (u32, Vec<(usize, String, FirstMismatch)>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = fuzz::Args::parse();
    let run = a.get("run").ok_or("использование: fuzz_report --run <папка прогона> [--threads N]")?;
    let threads: usize = a.num("threads", 8);
    let belts_only = a.flag("belts-only");
    let warmup: Option<u64> = a.get("warmup").and_then(|s| s.parse().ok());
    let run = Path::new(run);
    let mut names: Vec<String> = std::fs::read_dir(run)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".scene.json")).map(String::from))
        .collect();
    names.sort();

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(String, usize, Outcome)>> = Mutex::new(Vec::new());
    std::panic::set_hook(Box::new(|_| {}));
    std::thread::scope(|sc| {
        for _ in 0..threads.max(1) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(name) = names.get(i) else { break };
                let (n, o) = check_one(run, name, belts_only, warmup);
                results.lock().unwrap().push((name.clone(), n, o));
            });
        }
    });

    let (mut total, mut matched, mut unsupported, mut broken) = (0, 0, 0, 0);
    let mut panics: Vec<(String, String)> = Vec::new();
    let mut by_kind: BTreeMap<String, KindStats> = BTreeMap::new();
    let mut by_custom: BTreeMap<(String, bool), u32> = BTreeMap::new();
    let mut reasons: BTreeMap<String, u32> = BTreeMap::new();
    let mut first_tick: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (name, n, o) in results.into_inner().unwrap() {
        total += 1;
        match o {
            Outcome::Match => matched += 1,
            Outcome::Broken => broken += 1,
            Outcome::Panic(msg) => panics.push((name, msg)),
            Outcome::Unsupported(why) => {
                unsupported += 1;
                *reasons.entry(why).or_default() += 1;
            }
            Outcome::Mismatch(m) => {
                // кастомные векторы — у какого-нибудь манипулятора задан `pickup` или `drop`
                let custom = fidelity::load_scene(&run.join(format!("{name}.scene.json")))
                    .is_ok_and(|sc| sc.entities.iter().any(|e| e.pickup.is_some() || e.drop.is_some()));
                let c = by_custom.entry((m.kind.clone(), custom)).or_default();
                *c += 1;
                if a.flag("list") {
                    println!("{name} {} тик {} кастомные_векторы={custom}", m.kind, m.tick);
                }
                first_tick.entry(m.kind.clone()).or_default().push(m.tick);
                let e = by_kind.entry(m.kind.clone()).or_default();
                e.0 += 1;
                e.1.push((n, name, m));
                e.1.sort_by(|x, y| (x.0, &x.1).cmp(&(y.0, &y.1)));
                e.1.truncate(3);
            }
        }
    }

    let mismatched: u32 = by_kind.values().map(|v| v.0).sum();
    println!("прогон {}", run.display());
    println!("всего сцен: {total}");
    println!("эталон битый или неполный: {broken}");
    println!("не сравнивалось (модель не умеет): {unsupported}");
    for (why, n) in &reasons {
        println!("  {why}: {n}");
    }
    println!("паник модели: {}", panics.len());
    panics.sort();
    for (name, msg) in &panics {
        println!("  {name}: {msg}");
    }
    println!("совпало на всех тиках: {matched}");
    println!("расхождений: {mismatched}, видов: {}", by_kind.len());
    for ((kind, custom), n) in &by_custom {
        println!("  из них {kind}, {}: {n}", if *custom { "со сценами с кастомными векторами" } else { "без кастомных векторов" });
    }
    for (kind, (n, examples)) in &by_kind {
        let mut ticks = first_tick[kind].clone();
        ticks.sort();
        println!("  {kind}: {n} сцен (первое расхождение на тике: медиана {}, мин {})", ticks[ticks.len() / 2], ticks[0]);
        for (ents, name, m) in examples {
            println!("    {name} ({ents} сущн.): тик {}, {}", m.tick, m.detail);
        }
    }
    if !panics.is_empty() {
        eprintln!("ОШИБКА: модель упала с паникой в {} сценах", panics.len());
        std::process::exit(1);
    }
    Ok(())
}
