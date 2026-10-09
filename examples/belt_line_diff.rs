//! Отладка: потиковый след полос одной линии — модель против эталона.
//!   cargo run --release --example belt_line_diff -- <scene.json> <dump.jsonl> <линия> <полоса> <от> <до>
//! Позиции — от конца линии (256·k + p), голова слева. Строка `want`/`got`, `<<<` — расхождение.
use std::path::PathBuf;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = fidelity::load_scene(&PathBuf::from(&a[0]))?;
    let dump = fidelity::load_dump(&PathBuf::from(&a[1]))?;
    let line: usize = a[2].parse()?;
    let lane: usize = a[3].parse()?;
    let from: u64 = a[4].parse()?;
    let to: u64 = a[5].parse()?;
    let mut w = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled())?;
    let tiles = w.lines[line].tiles.clone();
    for want in dump.iter() {
        if want.t > 0 {
            w.step();
        }
        if want.t < from {
            continue;
        }
        if want.t > to {
            break;
        }
        let got = w.dump();
        let collect = |d: &factorio_distributed_core::world::layout::TickDump| -> Vec<i32> {
            let mut v = Vec::new();
            for (k, &b) in tiles.iter().enumerate() {
                for it in &d.belts[b as usize].lines[lane] {
                    v.push(k as i32 * 256 + it.p);
                }
            }
            v.sort();
            v
        };
        let (wv, gv) = (collect(want), collect(&got));
        let mark = if wv == gv { "" } else { " <<<" };
        println!("t={} want {:?}", want.t, wv);
        println!("t={}  got {:?}{mark}", want.t, gv);
    }
    Ok(())
}
