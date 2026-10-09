//! Отладка: первое расхождение каждой сцены прогона (все сцены, не только примеры).
//!   cargo run --release --example first_mismatch -- <папка прогона> [фильтр подстроки]
use std::path::PathBuf;

use factorio_distributed_core::fidelity;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("папка прогона"));
    let filt = std::env::args().nth(2).unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".scene.json"))
                .map(String::from)
        })
        .collect();
    names.sort();
    for name in names {
        let Ok(scene) = fidelity::load_scene(&dir.join(format!("{name}.scene.json"))) else {
            continue;
        };
        let Ok(dump) = fidelity::load_dump(&dir.join(format!("{name}.dump.jsonl"))) else {
            continue;
        };
        if let Ok(m) = fidelity::all_mismatches(&scene, &dump) {
            if let Some((t, d)) = m.first() {
                if d.contains(&filt) {
                    println!("{name} t={t} {d}");
                }
            }
        }
    }
    Ok(())
}
