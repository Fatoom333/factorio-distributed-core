//! Генератор случайных сцен для фаззера M0. Детерминирован по seed.
//!
//!   cargo run --release --bin fuzz_gen -- --seed 1 --count 1000 --out fuzz-scenes/1
//!
//! Флаги: --turns (повороты и боковая загрузка, это M0.5; по умолчанию выключено),
//!        --underground (сцены с подземными лентами, M0.5; отдельная ветка генератора),
//!        --no-vectors (без кастомных векторов манипулятора).
//! Без --out печатает сводку и ничего не пишет.

use factorio_distributed_core::fuzz;

use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = fuzz::Args::parse();
    let seed: u64 = a.num("seed", 1);
    let count: u32 = a.num("count", 100);
    let opts = a.opts();
    let out = a.get("out").map(std::path::PathBuf::from);
    if let Some(d) = &out {
        std::fs::create_dir_all(d)?;
    }
    let mut stats: BTreeMap<String, u32> = BTreeMap::new();
    for idx in 0..count {
        let s = fuzz::generate(seed, idx, opts);
        fuzz::check_scene(&s)?;
        for e in &s.entities {
            *stats.entry(e.name.to_string()).or_default() += 1;
            if e.pickup.is_some() || e.drop.is_some() {
                *stats.entry("(кастомный вектор)".into()).or_default() += 1;
            }
        }
        if let Some(d) = &out {
            let text = serde_json::to_string(&s.to_json())?;
            std::fs::write(d.join(format!("{}.scene.json", s.name)), text + "\n")?;
        }
    }
    println!("seed {seed}: {count} сцен");
    for (k, v) in stats {
        println!("  {k}: {v}");
    }
    Ok(())
}
