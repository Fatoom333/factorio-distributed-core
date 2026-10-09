//! Отладка: выгрузка наблюдений руки манипулятора — состояние модели (угол, вылет, высота)
//! рядом с `hand` эталона, пока всё остальное в сцене совпадает.
//!   cargo run --release --example hand_obs -- <папка прогона> > obs.jsonl
//! Строка: сцена, тик, индекс, центр, угол (2^-64 оборота), вылет (тайлы, приближённо), высота (Q48),
//! рука в игре и в модели (абсолютные, 1/256), векторы захвата/выгрузки (1/256).
use std::path::PathBuf;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("папка прогона"));
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
        let custom = scene
            .entities
            .iter()
            .any(|e| e.pickup.is_some() || e.drop.is_some());
        let Ok(mut w) = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled()) else {
            continue;
        };
        'ticks: for want in dump.iter() {
            if want.t > 0 {
                w.step();
            }
            let got = w.dump();
            if want.belts != got.belts
                || want.chests != got.chests
                || want.inserters.len() != got.inserters.len()
            {
                break;
            }
            for (wi, gi) in want.inserters.iter().zip(&got.inserters) {
                if wi.held != gi.held || wi.held_count != gi.held_count {
                    break 'ticks;
                }
                if (wi.hand[0] - gi.hand[0]).abs() > 1 || (wi.hand[1] - gi.hand[1]).abs() > 1 {
                    break 'ticks;
                }
            }
            for (k, (wi, gi)) in want.inserters.iter().zip(&got.inserters).enumerate() {
                let ins = &w.inserters[k];
                println!(
                    "{{\"s\":\"{name}\",\"c\":{custom},\"t\":{},\"k\":{k},\"pos\":[{},{}],\"o\":\"{}\",\"e\":\"{}\",\"h\":\"{}\",\"want\":[{},{}],\"got\":[{},{}],\"pv\":[{},{}],\"dv\":[{},{}],\"held\":{}}}",
                    want.t,
                    ins.pos.0,
                    ins.pos.1,
                    ins.hand.o,
                    ins.hand.ext.approx_f64(),
                    (ins.hand.h.approx_f64() * 2f64.powi(48)) as i64,
                    wi.hand[0],
                    wi.hand[1],
                    gi.hand[0],
                    gi.hand[1],
                    ins.pickup_point.0 - ins.pos.0,
                    ins.pickup_point.1 - ins.pos.1,
                    ins.drop_point.0 - ins.pos.0,
                    ins.drop_point.1 - ins.pos.1,
                    ins.held.is_some(),
                );
            }
        }
    }
    Ok(())
}
