//! Отладка: события «вставка, после которой задний предмет ближе 64» и что было дальше.
//!   cargo run --release --example overcompressed_scan -- <папка с *.scene.json и *.dump.jsonl>
use std::collections::HashMap;
use std::path::Path;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::proto::ITEM_SPACING;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".scene.json")).map(String::from))
        .collect();
    names.sort();
    for name in names {
        let Ok(scene) = fidelity::load_scene(&Path::new(&dir).join(format!("{name}.scene.json"))) else { continue };
        let Ok(dump) = fidelity::load_dump(&Path::new(&dir).join(format!("{name}.dump.jsonl"))) else { continue };
        let Ok(mut w) = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled()) else { continue };
        let mut events: Vec<String> = Vec::new();
        let mut first_bad: Option<(u64, bool)> = None;
        for (i, want) in dump.iter().enumerate() {
            let prev: HashMap<u64, i32> =
                w.lines.iter().flat_map(|l| l.lanes.iter().flat_map(|ln| ln.items.iter().map(|it| (it.uid, it.pos)))).collect();
            if i > 0 {
                w.step();
            }
            for (li, l) in w.lines.iter().enumerate() {
                for (lane, ln) in l.lanes.iter().enumerate() {
                    for k in 1..ln.items.len() {
                        let (a, b) = (&ln.items[k - 1], &ln.items[k]);
                        if a.uid >= w.fresh_uid && b.pos - a.pos < ITEM_SPACING {
                            let bstate = if b.uid >= w.fresh_uid {
                                "fresh"
                            } else if prev.get(&b.uid) != Some(&b.pos) {
                                "moved"
                            } else {
                                "still"
                            };
                            let astate = if k >= 2 && a.pos - ln.items[k - 2].pos == ITEM_SPACING { "queue" } else { "free" };
                            events.push(format!(
                                "t={} line={li} lane={lane} v={} gap={} behind={bstate} ahead={astate} newpos={}",
                                w.tick, l.speed, b.pos - a.pos, a.pos
                            ));
                        }
                    }
                }
            }
            if first_bad.is_none() && w.dump() != *want {
                let belts = w.dump().belts != want.belts;
                first_bad = Some((want.t, belts));
            }
        }
        if !events.is_empty() {
            println!("{name}: first_bad={first_bad:?}");
            for e in events {
                println!("   {e}");
            }
        }
    }
    Ok(())
}
