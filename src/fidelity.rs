//! Сверка модели с эталоном из игры: строим мир из сцены, гоним тики, сравниваем с
//! эталоном (`<name>.dump.jsonl`, одна строка JSON на тик) структурно.

use std::path::Path;

use crate::world::layout::{Scene, TickDump};
use crate::world::World;

/// Итог сверки одной сцены.
#[derive(Debug)]
pub struct Report {
    pub scene: String,
    /// Тиков эталона (кроме тика 0).
    pub total_ticks: u64,
    /// Сколько тиков подряд с начала совпало (тик 0 не считается).
    pub matched: u64,
    /// Первое расхождение: описание.
    pub first_mismatch: Option<String>,
}

/// Имя сцены идёт в путь файла — то же правило, что в моде: `[a-z0-9_-]+` (нижний регистр,
/// не длиннее 100) и не зарезервированное имя устройства Windows (`con`, `nul`, `aux`, `prn`,
/// `com1`–`com9`, `lpt1`–`lpt9`).
pub fn valid_scene_name(name: &str) -> bool {
    let reserved = matches!(name, "con" | "nul" | "aux" | "prn")
        || ((name.starts_with("com") || name.starts_with("lpt"))
            && name.len() == 4
            && name.as_bytes()[3].is_ascii_digit()
            && name.as_bytes()[3] != b'0');
    !name.is_empty()
        && name.len() <= 100
        && !reserved
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn load_scene(path: &Path) -> Result<Scene, Box<dyn std::error::Error>> {
    Ok(Scene::from_json(&std::fs::read_to_string(path)?)?)
}

/// Эталон: строки JSON. Концы строк CRLF/LF допустимы (trim).
pub fn load_dump(path: &Path) -> Result<Vec<TickDump>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let d: TickDump = serde_json::from_str(line).map_err(|e| format!("строка {}: {e}", i + 1))?;
        out.push(d);
    }
    Ok(out)
}

/// Первое отличие двух снимков (человекочитаемо) или `None`.
pub fn diff(want: &TickDump, got: &TickDump) -> Option<String> {
    if want == got {
        return None;
    }
    if want.t != got.t {
        return Some(format!("тик: ожидалось {}, получилось {}", want.t, got.t));
    }
    let tile = |p: &crate::world::layout::Pos| format!("[{}, {}]", p.0 as f64 / 256.0, p.1 as f64 / 256.0);
    if want.belts.len() != got.belts.len() {
        return Some("разное число лент".into());
    }
    for (w, g) in want.belts.iter().zip(&got.belts) {
        if w != g {
            let f = |v: &Vec<crate::world::layout::DumpItem>| {
                v.iter().map(|d| format!("{}@{}", d.item, d.p)).collect::<Vec<_>>().join(" ")
            };
            for lane in 0..2 {
                if w.lines[lane] != g.lines[lane] {
                    return Some(format!(
                        "лента {} полоса {}: ожидалось [{}], получилось [{}]",
                        tile(&w.pos),
                        lane,
                        f(&w.lines[lane]),
                        f(&g.lines[lane])
                    ));
                }
            }
            // подземная часть подземки (линии 3 и 4 сущности)
            match (&w.under, &g.under) {
                (Some(wu), Some(gu)) => {
                    for lane in 0..2 {
                        if wu[lane] != gu[lane] {
                            return Some(format!(
                                "подземка {} под землёй, полоса {}: ожидалось [{}], получилось [{}]",
                                tile(&w.pos),
                                lane,
                                f(&wu[lane]),
                                f(&gu[lane])
                            ));
                        }
                    }
                }
                (None, None) => {}
                (wu, _) => {
                    return Some(format!(
                        "лента {}: в эталоне {}, в модели наоборот",
                        tile(&w.pos),
                        if wu.is_some() { "подземка (есть under)" } else { "обычная лента (нет under)" }
                    ));
                }
            }
            return Some(format!("лента {}: позиция", tile(&w.pos)));
        }
    }
    if want.inserters.len() != got.inserters.len() {
        return Some("разное число манипуляторов".into());
    }
    for (w, g) in want.inserters.iter().zip(&got.inserters) {
        if w != g {
            return Some(format!(
                "манипулятор {}: ожидалось held={:?}×{} hand={:?}, получилось held={:?}×{} hand={:?}",
                tile(&w.pos),
                w.held,
                w.held_count,
                w.hand,
                g.held,
                g.held_count,
                g.hand
            ));
        }
    }
    if want.chests.len() != got.chests.len() {
        return Some("разное число сундуков".into());
    }
    for (w, g) in want.chests.iter().zip(&got.chests) {
        if w != g {
            return Some(format!(
                "сундук {}: ожидалось {:?}, получилось {:?}",
                tile(&w.pos),
                w.items,
                g.items
            ));
        }
    }
    Some("отличие вне известных полей".into())
}

/// Прогон сцены против эталона. Останавливается на первом расхождении.
pub fn run(scene: &Scene, dump: &[TickDump]) -> Result<Report, Box<dyn std::error::Error>> {
    run_with(scene, dump, false)
}

/// То же; `belts_only` — сверять только ленты (остальные поля снимка не сравниваются).
pub fn run_with(scene: &Scene, dump: &[TickDump], belts_only: bool) -> Result<Report, Box<dyn std::error::Error>> {
    let mut world = World::from_scene(scene, crate::world::proto::Prototypes::bundled())?;
    let total = dump.len().saturating_sub(1) as u64;
    let mut report = Report { scene: scene.name.clone(), total_ticks: total, matched: 0, first_mismatch: None };
    for (i, want) in dump.iter().enumerate() {
        if i > 0 {
            world.step();
        }
        let got = world.dump();
        if belts_only && want.belts == got.belts {
            if i > 0 {
                report.matched += 1;
            }
            continue;
        }
        if let Some(d) = diff(want, &got) {
            report.first_mismatch = Some(format!("тик {}: {}", want.t, d));
            return Ok(report);
        }
        if i > 0 {
            report.matched += 1;
        }
    }
    Ok(report)
}

/// Все расхождения (тик, описание) — для отладки: модель не останавливается, а идёт
/// дальше своим ходом (её состояние может уже отличаться от игры).
pub fn all_mismatches(scene: &Scene, dump: &[TickDump]) -> Result<Vec<(u64, String)>, Box<dyn std::error::Error>> {
    let mut world = World::from_scene(scene, crate::world::proto::Prototypes::bundled())?;
    let mut out = Vec::new();
    for (i, want) in dump.iter().enumerate() {
        if i > 0 {
            world.step();
        }
        if let Some(d) = diff(want, &world.dump()) {
            out.push((want.t, d));
        }
    }
    Ok(out)
}

/// Число игры (тайлы, `f64` из JSON) в 1/256 — только если оно ровно кратно 1/256.
fn exact_fx(v: &serde_json::Value) -> Result<crate::world::fixed::Fx, String> {
    let x = v.as_f64().ok_or("не число")? * 256.0;
    if x.fract() != 0.0 || x.abs() > 1e9 {
        return Err(format!("{} не кратно 1/256", x / 256.0));
    }
    Ok(x as crate::world::fixed::Fx)
}

fn exact_pt(v: &serde_json::Value) -> Result<(crate::world::fixed::Fx, crate::world::fixed::Fx), String> {
    Ok((exact_fx(&v[0])?, exact_fx(&v[1])?))
}

/// Сверка геометрии модели с числами игры для построенной сцены — ответ операции мода
/// `scene_info` (массив по сущностям сцены): позиции сущностей, `pickup_position` и
/// `drop_position` манипуляторов, `get_line_item_position(линия, p)` лент.
pub fn check_geometry(world: &World, info: &serde_json::Value) -> Result<(), String> {
    let arr = info.as_array().ok_or("scene_info: не массив")?;
    use crate::world::TileKind;
    // тайлы сцены в порядке сцены (скрытые подземные части пар — не сущности сцены)
    let tiles: Vec<usize> = (0..world.belts.len()).filter(|&b| world.belts[b].kind != TileKind::Under).collect();
    let (mut bi, mut ii) = (0usize, 0usize);
    for (k, e) in arr.iter().enumerate() {
        let at = |m: String| format!("сущность {k} ({}): {m}", e["name"]);
        let pos = exact_pt(&e["position"]).map_err(at)?;
        if let Some(lanes) = e.get("lanes") {
            let b = tiles.get(bi).map(|&b| &world.belts[b]).ok_or_else(|| at("в модели меньше лент".into()))?;
            bi += 1;
            if b.kind != TileKind::Belt {
                return Err(at("в модели на этом месте подземка".into()));
            }
            if b.pos != pos {
                return Err(at(format!("позиция: игра {pos:?}, модель {:?}", b.pos)));
            }
            for lane in 0..2 {
                for pt in lanes[lane].as_array().ok_or_else(|| at("lanes".into()))? {
                    let p = exact_fx(&pt[0]).map_err(at)?;
                    // Точку за концом полосы (`p` не меньше её длины) игра прижимает к последней
                    // точке полосы, `длина − 1`: так у внутренней полосы поворота (длина 106: точки
                    // 128 и 192) и у надземной половины подземки (длина 128). Правило одно.
                    let p = p.min(b.lanes[lane].len - 1);
                    let game = (exact_fx(&pt[1]).map_err(at)?, exact_fx(&pt[2]).map_err(at)?);
                    let model = b.item_pos(lane, p);
                    if game != model {
                        return Err(at(format!(
                            "get_line_item_position({}, {}): игра {game:?}, модель {model:?}",
                            lane + 1,
                            p as f64 / 256.0
                        )));
                    }
                }
            }
        } else if let Some(lines) = e.get("ug_lines") {
            // Подземка: вид, пара, длины линий (надземные — 0.5 тайла, подземная часть у входа с
            // парой — расстояние до выхода) и точки надземных линий.
            let b = tiles.get(bi).map(|&b| &world.belts[b]).ok_or_else(|| at("в модели меньше лент".into()))?;
            bi += 1;
            let kind = match e["ug_type"].as_str() {
                Some("input") => TileKind::UgIn,
                Some("output") => TileKind::UgOut,
                _ => return Err(at("ug_type".into())),
            };
            if b.kind != kind || b.pos != pos {
                return Err(at(format!("подземка: игра {:?} {pos:?}, модель {:?} {:?}", e["ug_type"], b.kind, b.pos)));
            }
            let pair = e.get("pair").filter(|p| !p.is_null()).map(exact_pt).transpose().map_err(at)?;
            if kind == TileKind::UgIn {
                // пара в модели — выход, в который впадает подземная часть входа
                let model = (b.under != crate::world::wake::NONE).then(|| {
                    let u = &world.belts[b.under as usize].lanes[0];
                    let (ux, uy) = b.dir.unit();
                    (b.pos.0 + ux * u.len, b.pos.1 + uy * u.len)
                });
                if pair != model {
                    return Err(at(format!("пара: игра {pair:?}, модель {model:?}")));
                }
            }
            for lane in 0..2 {
                let l = &lines[lane];
                let len = exact_fx(&l["length"]).map_err(at)?;
                if len != b.lanes[lane].len {
                    return Err(at(format!("длина линии {}: игра {len}, модель {}", lane + 1, b.lanes[lane].len)));
                }
                if b.under != crate::world::wake::NONE {
                    let (g, m) = (exact_fx(&lines[lane + 2]["length"]).map_err(at)?, world.belts[b.under as usize].lanes[lane].len);
                    if g != m {
                        return Err(at(format!("длина подземной линии {}: игра {g}, модель {m}", lane + 3)));
                    }
                }
                for pt in l["points"].as_array().ok_or_else(|| at("ug_lines.points".into()))? {
                    // как у ленты: точка за концом полосы прижата к `длина − 1`
                    let p = exact_fx(&pt[0]).map_err(at)?.min(len - 1);
                    let game = (exact_fx(&pt[1]).map_err(at)?, exact_fx(&pt[2]).map_err(at)?);
                    let model = b.item_pos(lane, p);
                    if game != model {
                        return Err(at(format!(
                            "get_line_item_position({}, {}): игра {game:?}, модель {model:?}",
                            lane + 1,
                            p as f64 / 256.0
                        )));
                    }
                }
            }
        } else if let Some(drop) = e.get("drop") {
            let ins = world.inserters.get(ii).ok_or_else(|| at("в модели меньше манипуляторов".into()))?;
            ii += 1;
            if ins.pos != pos {
                return Err(at(format!("позиция: игра {pos:?}, модель {:?}", ins.pos)));
            }
            let d = exact_pt(drop).map_err(at)?;
            if d != ins.drop_point {
                return Err(at(format!("drop_position: игра {d:?}, модель {:?}", ins.drop_point)));
            }
            let p = exact_pt(&e["pickup"]).map_err(at)?;
            if p != ins.pickup_point {
                return Err(at(format!("pickup_position: игра {p:?}, модель {:?}", ins.pickup_point)));
            }
            // Размер руки: модель берёт `1 + stack_size_bonus` прототипа и не знает бонусов
            // исследований силы и ручного предела — в игре они должны быть нулевыми.
            let bonus = e["force_stack_bonus"].as_f64().ok_or_else(|| at("нет force_stack_bonus (старый мод?)".into()))?;
            let bulk = e["force_bulk_bonus"].as_f64().unwrap_or(0.0);
            let ov = e["stack_override"].as_u64().unwrap_or(0);
            if bonus != 0.0 || bulk != 0.0 || ov != 0 {
                return Err(at(format!(
                    "размер руки: бонус силы {bonus}, bulk {bulk}, предел {ov} — модель знает только руку прототипа"
                )));
            }
        }
    }
    if bi != tiles.len() || ii != world.inserters.len() {
        return Err("scene_info: число лент/манипуляторов не совпало с моделью".into());
    }
    Ok(())
}

#[cfg(test)]
mod name_tests {
    use super::valid_scene_name as ok;

    #[test]
    fn scene_names() {
        assert!(ok("s101-00774") && ok("a_b-c9"));
        assert!(!ok("") && !ok("A") && !ok("a/b") && !ok("..") && !ok("a b") && !ok(&"a".repeat(101)));
        assert!(!ok("con") && !ok("com1") && !ok("lpt9") && ok("com0") && ok("com10") && ok("console"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Расхождение только в подземной части подземки называется своим именем, а не «позиция».
    #[test]
    fn diff_names_underground_part() {
        let line = |under: &str| -> TickDump {
            serde_json::from_str(&format!(
                r#"{{"t":5,"belts":[{{"pos":[1.5,0.5],"lines":[[],[]],"ug":"input","under":{under}}}],"inserters":[],"chests":[]}}"#
            ))
            .unwrap()
        };
        let (want, got) = (line(r#"[[],[{"item":"coal","p":508}]]"#), line(r#"[[],[{"item":"coal","p":504}]]"#));
        assert_eq!(diff(&want, &want), None);
        let d = diff(&want, &got).unwrap();
        assert!(d.contains("под землёй") && d.contains("coal@508") && d.contains("coal@504"), "{d}");
        let belt: TickDump = serde_json::from_str(r#"{"t":5,"belts":[{"pos":[1.5,0.5],"lines":[[],[]]}],"inserters":[],"chests":[]}"#).unwrap();
        assert!(diff(&want, &belt).unwrap().contains("подземка"));
    }
}
