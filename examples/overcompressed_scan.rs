//! Отладка: события «вставка, после которой задний предмет ближе 64» и что было дальше.
//!   cargo run --release --example overcompressed_scan -- <папка с *.scene.json и *.dump.jsonl>
use std::collections::HashMap;
use std::path::Path;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::proto::ITEM_SPACING;
use factorio_distributed_core::world::World;

struct Item {
    uid: u64,
    pos: i32,
}

/// Полоса цепочки: (головная линия, полоса, скорость головной линии, предметы (uid, позиция)).
type ChainLane = (usize, usize, i32, Vec<(u64, i32)>);

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
            // позиция предмета — от конца цепочки: начало тайла на цепочке не меняется при нарезке
            let chain_pos = |w: &World| -> Vec<ChainLane> {
                let mut out = Vec::new();
                for (li, l) in w.lines.iter().enumerate().filter(|(_, l)| l.alive && l.out.is_none()) {
                    let (mut base, mut cur, mut v) = (0, Some(li as u32), Vec::new());
                    while let Some(c) = cur {
                        let cl = &w.lines[c as usize];
                        v.extend(cl.positions().map(|(_, p, it)| (it.uid, base + p)));
                        base += cl.len;
                        cur = cl.inp;
                    }
                    out.push((li, l.lane as usize, l.speed, v));
                }
                out
            };
            let prev: HashMap<u64, i32> = chain_pos(&w).into_iter().flat_map(|(_, _, _, v)| v).collect();
            // предметы с `uid >= fresh_uid` положены в этом тике
            let fresh_uid = w.next_uid;
            if i > 0 {
                w.step();
            }
            for (li, lane, speed, items) in chain_pos(&w) {
                {
                    let ln_items = &items;
                    let item = |k: usize| Item { uid: items[k].0, pos: items[k].1 };
                    for k in 1..items.len() {
                        let (a, b) = (item(k - 1), item(k));
                        if a.uid >= fresh_uid && b.pos - a.pos < ITEM_SPACING {
                            let bstate = if b.uid >= fresh_uid {
                                "fresh"
                            } else if prev.get(&b.uid) != Some(&b.pos) {
                                "moved"
                            } else {
                                "still"
                            };
                            let astate = if k >= 2 && a.pos - ln_items[k - 2].1 == ITEM_SPACING { "queue" } else { "free" };
                            events.push(format!(
                                "t={} line={li} lane={lane} v={} gap={} behind={bstate} ahead={astate} newpos={}",
                                w.tick, speed, b.pos - a.pos, a.pos
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
