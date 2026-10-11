//! Отладка: потиковый след одного манипулятора — модель против эталона.
//!   cargo run --release --example trace -- <scene.json> <dump.jsonl> <индекс манипулятора> <от> <до>
//! `game_h` — интервал высоты руки в игре, выведенный из `hand` эталона и плоской позиции модели.
use std::path::PathBuf;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::fixed::hand_offset;
use factorio_distributed_core::world::inserter::Target;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = fidelity::load_scene(&PathBuf::from(&a[0]))?;
    let dump = fidelity::load_dump(&PathBuf::from(&a[1]))?;
    let idx: usize = a[2].parse()?;
    let from: u64 = a[3].parse()?;
    let to: u64 = a[4].parse()?;
    let mut w = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled())?;
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
        let (wi, gi) = (&want.inserters[idx], &got.inserters[idx]);
        let ins = &w.inserters[idx];
        let o = ins.hand.o as f64 / 2f64.powi(64);
        let e = ins.hand.ext.approx_f64();
        let h = ins.hand.h.approx_f64() * 256.0;
        let (_, py) = hand_offset(ins.hand.o, ins.hand.ext);
        let game_h = py as i32 - (wi.hand[1] - ins.pos.1);
        let cand = match ins.pickup {
            Target::Belt(b) => w.select_belt_item(idx, b as usize).0.map(|(l, i, p)| {
                let tl = &w.belts[b as usize].lanes[l];
                format!("L{l} P{p} uid{}", w.lines[tl.line as usize].items[i].uid)
            }),
            _ => None,
        };
        let mark = if wi == gi { "" } else { "<<<" };
        println!(
            "t={} want {:?} {:?} | got {:?} {:?} | o={o:.6} ext={e:.5} h={h:.3} game_h~[{game_h},{}) chase={:?} next={:?} {} order={:?} {mark}",
            want.t,
            wi.held,
            [wi.hand[0] - ins.pos.0, wi.hand[1] - ins.pos.1],
            gi.held,
            [gi.hand[0] - ins.pos.0, gi.hand[1] - ins.pos.1],
            game_h + 1,
            ins.chase,
            cand,
            if ins.awake { "awake" } else { "asleep" },
            w.active_inserters(),
        );
    }
    Ok(())
}
