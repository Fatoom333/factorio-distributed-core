//! Отладка: потиковый след полос одной линии — модель против эталона.
//!   cargo run --release --example belt_line_diff -- <scene.json> <dump.jsonl> <лента> <полоса> <от> <до>
//! `<лента>` — номер любой ленты цепочки (по порядку лент в сцене). Позиции — от конца цепочки
//! (256·k + p), голова слева. Строка `want`/`got`, `<<<` — расхождение; `cut` — границы линий
//! модели (номера тайлов от головы, с которых начинается новая линия) и `*` у активных линий.
use std::path::PathBuf;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = fidelity::load_scene(&PathBuf::from(&a[0]))?;
    let dump = fidelity::load_dump(&PathBuf::from(&a[1]))?;
    let belt: usize = a[2].parse()?;
    let lane: usize = a[3].parse()?;
    let from: u64 = a[4].parse()?;
    let to: u64 = a[5].parse()?;
    let mut w = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled())?;
    // цепочка от головы к хвосту
    let chain = |w: &World| -> Vec<(u32, u32)> {
        let mut li = w.belts[belt].lanes[lane].line;
        while let Some(o) = w.lines[li as usize].out {
            li = o;
        }
        let mut v = Vec::new();
        let mut cur = Some(li);
        while let Some(l) = cur {
            v.extend(w.lines[l as usize].tiles.iter().map(|&t| (t, l)));
            cur = w.lines[l as usize].inp;
        }
        v
    };
    let tiles: Vec<u32> = chain(&w).iter().map(|&(t, _)| t).collect();
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
        let ch = chain(&w);
        let cuts: Vec<String> = ch
            .iter()
            .enumerate()
            .filter(|&(k, &(_, l))| k == 0 || ch[k - 1].1 != l)
            .map(|(k, &(_, l))| format!("{k}{}", if w.lines[l as usize].active { "*" } else { "" }))
            .collect();
        println!("t={}  cut {}", want.t, cuts.join(" "));
    }
    Ok(())
}
