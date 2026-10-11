//! Проверка правил лент отдельно от манипуляторов: действия манипуляторов (кто и когда положил
//! на ленту / взял с ленты) берутся из эталона, а движение лент, нарезка линий и место каждой
//! кладки считаются моделью и сверяются с каждым снимком.
//!   cargo run --release --example belt_replay -- <папка с *.scene.json и *.dump.jsonl> [--warmup N] [--list]
//!
//! Порядок манипуляторов внутри тика и то, какой именно предмет взял сток, здесь не
//! моделируются, а подбираются (порядок сцены, обратный ему, а при числе «настоящих» действий
//! за тик до 5 — все их перестановки; кандидаты на тайле), пока снимок не совпадёт. Поэтому результат говорит только о лентах:
//! «при каком-то порядке манипуляторов правила лент дают ровно то, что в игре».
use std::path::Path;

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::inserter::Target;
use factorio_distributed_core::world::layout::{DumpInserter, TickDump};
use factorio_distributed_core::world::proto::Prototypes;
use factorio_distributed_core::world::World;

/// Действия манипуляторов `order[start..]` в тике `now`; `true`, если ленты после них равны снимку.
fn phase(w: &mut World, order: &[usize], start: usize, a: &[DumpInserter], b: &TickDump, now: i64) -> bool {
    for oi in start..order.len() {
        let i = order[oi];
        let (ha, hb) = (&a[i].held, &b.inserters[i].held);
        let ins = w.inserters[i].clone();
        if let (Target::Belt(t), Some(name)) = (ins.drop, ha) {
            let item = w.item_names.iter().position(|n| n == name).expect("предмет сцены") as u16;
            let (lane, in_tile) = w.belts[t as usize].drop_lane_pos(ins.drop_point);
            let uid = w.next_uid;
            w.next_uid += 1;
            if w.belt_insert(t as usize, lane, in_tile, item, uid, now) != hb.is_none() {
                return false;
            }
        } else if let (Target::Belt(t), None) = (ins.pickup, ha) {
            let t = t as usize;
            if let Some(name) = hb {
                // кандидаты: предметы тайла с этим именем, которых нет в снимке; иначе — все
                let want = &b.belts[t].lines;
                let mut cands: Vec<(usize, usize)> = Vec::new();
                for strict in [true, false] {
                    for (lane, want_lane) in want.iter().enumerate() {
                        let (li, lo, hi) = w.lane_span(t, lane);
                        for (idx, p, it) in w.lines[li as usize].range(lo, hi) {
                            let same = &w.item_names[it.item as usize] == name;
                            let in_snap = want_lane.iter().any(|d| d.p == p - lo && &d.item == name);
                            if same && !(strict && in_snap) {
                                cands.push((lane, idx));
                            }
                        }
                    }
                    if !cands.is_empty() {
                        break;
                    }
                }
                let saved = w.clone();
                for (lane, idx) in cands {
                    *w = saved.clone();
                    let li = w.belts[t].lanes[lane].line;
                    w.touch(t, lane, now);
                    w.belt_remove(li, idx);
                    if phase(w, order, oi + 1, a, b, now) {
                        return true;
                    }
                }
                return false;
            } else if a[i].hand == [ins.pickup_point.0, ins.pickup_point.1] {
                // сток стоит у точки захвата и ищет: отмечает полосы, чьи линии не пусты
                for lane in 0..2 {
                    if !w.lines[w.belts[t].lanes[lane].line as usize].is_empty() {
                        w.touch(t, lane, now);
                    }
                }
            }
        }
    }
    w.dump().belts == b.belts
}

fn permutations(v: &[usize]) -> Vec<Vec<usize>> {
    if v.len() <= 1 {
        return vec![v.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..v.len() {
        let mut rest = v.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x);
            out.push(p);
        }
    }
    out
}

/// `Ok(число тиков с подобранным порядком)` или `Err(первый несовпавший тик)`.
fn replay(w: &mut World, dump: &[TickDump]) -> Result<u32, u64> {
    let mut reordered = 0;
    for k in 0..dump.len() - 1 {
        let now = k as i64;
        w.tick += 1;
        w.step_belts(now);
        let (a, b) = (&dump[k].inserters, &dump[k + 1]);
        let mut acting = Vec::new();
        for (i, ins) in w.inserters.iter().enumerate() {
            let (ha, hb) = (&a[i].held, &b.inserters[i].held);
            let at_drop = b.inserters[i].hand == [ins.drop_point.0, ins.drop_point.1];
            let drops = matches!(ins.drop, Target::Belt(_)) && ha.is_some() && (at_drop || hb.is_none());
            let picks = matches!(ins.pickup, Target::Belt(_)) && ha.is_none();
            if drops || picks {
                acting.push(i);
            }
        }
        let real: Vec<usize> =
            acting.iter().copied().filter(|&i| a[i].held.is_some() || b.inserters[i].held.is_some()).collect();
        let rest: Vec<usize> = acting.iter().copied().filter(|i| !real.contains(i)).collect();
        let mut orders = vec![acting.clone()];
        if real.len() > 5 {
            orders.push(acting.iter().rev().copied().collect());
        }
        if (2..=5).contains(&real.len()) {
            for mut p in permutations(&real) {
                p.extend(&rest);
                if p != acting {
                    orders.push(p);
                }
            }
        }
        let saved = w.clone();
        let mut ok = false;
        for (oi, order) in orders.iter().enumerate() {
            *w = saved.clone();
            if phase(w, order, 0, a, b, now) {
                reordered += (oi > 0) as u32;
                ok = true;
                break;
            }
        }
        if !ok {
            if std::env::var_os("FD_REPLAY_VERBOSE").is_some() {
                *w = saved.clone();
                phase(w, &orders[0], 0, a, b, now);
                let got = w.dump();
                for (i, (wb, gb)) in b.belts.iter().zip(&got.belts).enumerate() {
                    if wb != gb {
                        let f = |l: &[Vec<factorio_distributed_core::world::layout::DumpItem>; 2]| {
                            l.iter().map(|v| format!("{:?}", v.iter().map(|d| d.p).collect::<Vec<_>>())).collect::<Vec<_>>().join(" ")
                        };
                        println!("  тик {}: лента {i}: игра {} | правила {}", b.t, f(&wb.lines), f(&gb.lines));
                    }
                }
                println!("  действуют манипуляторы (порядок сцены): {acting:?}, настоящих действий: {}", real.len());
            }
            return Err(b.t);
        }
    }
    Ok(reordered)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().ok_or("нужна папка со сценами")?;
    let warmup: Option<u64> = args.iter().position(|a| a == "--warmup").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok());
    let list = args.iter().any(|a| a == "--list");
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".scene.json")).map(String::from))
        .collect();
    names.sort();
    let (mut ok, mut bad, mut skipped, mut reordered_scenes) = (0, 0, 0, 0);
    for name in names {
        let Ok(mut scene) = fidelity::load_scene(&Path::new(dir).join(format!("{name}.scene.json"))) else { continue };
        let Ok(dump) = fidelity::load_dump(&Path::new(dir).join(format!("{name}.dump.jsonl"))) else { continue };
        if warmup.is_some() {
            scene.warmup = warmup;
        }
        let Ok(mut w) = World::from_scene(&scene, Prototypes::bundled()) else {
            skipped += 1;
            continue;
        };
        if dump.is_empty() || dump[0].inserters.len() != w.inserters.len() || dump[0].belts.len() != w.belts.len() {
            skipped += 1;
            continue;
        }
        match replay(&mut w, &dump) {
            Ok(r) => {
                ok += 1;
                reordered_scenes += (r > 0) as u32;
                if list {
                    println!("{name}: совпало, тиков с подобранным порядком манипуляторов: {r}");
                }
            }
            Err(t) => {
                bad += 1;
                println!("{name}: расхождение лент на тике {t}");
            }
        }
    }
    println!("ленты совпали: {ok}, разошлись: {bad}, пропущено: {skipped}; сцен, где порядок манипуляторов подбирался: {reordered_scenes}");
    Ok(())
}
