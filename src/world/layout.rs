//! Формат сцены и эталона (контракт мода и модели, см. tests/fidelity/README.md)
//! и сборка мира из сцены.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::belt::{BeltLine, ItemId, Lane};
use super::fixed::{fx_from_f64, hypot_f64, orientation_f32, Fx, TILE};
use super::inserter::{Hand, HandTarget, Inserter, Target};
use super::inventory::{Chest, Inventory};
use super::proto::{BeltProto, InserterProto, Prototypes};
use super::softfloat::Sf;
use super::{BeltTile, Dir, World};

// ---------- сцена ----------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scene {
    pub name: String,
    pub ticks: u64,
    pub entities: Vec<SceneEntity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneEntity {
    pub name: String,
    /// Центр сущности в тайлах.
    pub position: [f64; 2],
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub items: Option<BTreeMap<String, u32>>,
    /// Кастомные векторы манипулятора: смещения [dx, dy] от его центра, тайлы
    /// (мод с `allow_custom_vectors`). Игра хранит точку с усечением вниз до 1/256.
    #[serde(default)]
    pub pickup: Option<[f64; 2]>,
    #[serde(default)]
    pub drop: Option<[f64; 2]>,
}

impl Scene {
    pub fn from_json(s: &str) -> Result<Scene, serde_json::Error> {
        serde_json::from_str(s)
    }
}

// ---------- эталон (одна строка на тик) ----------

/// Позиция в 1/256 тайла; в JSON — число тайлов (как пишет мод).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pos(pub Fx, pub Fx);

impl Serialize for Pos {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // вывод: тайлы как в моде ("%.4f" без хвостовых нулей) — точные двоичные дроби
        [self.0 as f64 / 256.0, self.1 as f64 / 256.0].serialize(s)
    }
}

impl<'de> Deserialize<'de> for Pos {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = <[f64; 2]>::deserialize(d)?;
        Ok(Pos(fx_from_f64(v[0]), fx_from_f64(v[1])))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpItem {
    pub item: String,
    pub p: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpBelt {
    pub pos: Pos,
    pub lines: [Vec<DumpItem>; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpInserter {
    pub pos: Pos,
    pub held: Option<String>,
    pub held_count: u32,
    pub hand: [Fx; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpChest {
    pub pos: Pos,
    pub items: BTreeMap<String, u32>,
    /// Слоты по порядку инвентаря: `[имя, число]` или `null`; хвост пустых слотов отрезан.
    pub slots: Vec<Option<(String, u32)>>,
}

/// Состояние за один тик — строка эталона.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickDump {
    pub t: u64,
    pub belts: Vec<DumpBelt>,
    pub inserters: Vec<DumpInserter>,
    pub chests: Vec<DumpChest>,
}

// ---------- сборка ----------

#[derive(Debug)]
pub struct LayoutError(pub String);

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "сцена: {}", self.0)
    }
}

impl std::error::Error for LayoutError {}

fn err<T>(s: String) -> Result<T, LayoutError> {
    Err(LayoutError(s))
}

#[derive(Clone, Copy)]
enum Kind {
    Belt(usize),
    Inserter,
    Chest(usize),
}

/// Тайл, на котором лежит точка (1/256), — целые координаты тайла.
fn tile_of(x: Fx, y: Fx) -> (i32, i32) {
    (x.div_euclid(TILE), y.div_euclid(TILE))
}

/// Индекс прототипа в списке использованных (в `World` хранятся только они).
fn proto_index<T: PartialEq + Clone>(list: &mut Vec<T>, p: &T) -> u8 {
    match list.iter().position(|x| x == p) {
        Some(i) => i as u8,
        None => {
            list.push(p.clone());
            u8::try_from(list.len() - 1).expect("больше 256 прототипов одного вида в сцене")
        }
    }
}

/// Повернуть вектор, заданный для «севера», в направление `dir` (i64).
fn rot64(dir: Dir, v: (i64, i64)) -> (i64, i64) {
    let (x, y) = v;
    match dir {
        Dir::North => (x, y),
        Dir::East => (-y, x),
        Dir::South => (-x, -y),
        Dir::West => (y, -x),
    }
}

/// Наибольшая допустимая координата центра сущности, тайлы. 10^6 · 256 = 2,56·10^8 помещается
/// в `i32` с большим запасом, поэтому сложения позиций с векторами манипуляторов и смещениями
/// полос в `build`, `item_pos` и `pickup_point` переполниться не могут.
pub const MAX_COORD_TILES: f64 = 1e6;

/// Наибольшая допустимая компонента кастомного вектора манипулятора, тайлы: генератор фаззера
/// использует до `fuzz::MAX_CUSTOM_REACH` = 3, остальное — запас. Не даёт векторам руки
/// выйти за точное представление длины в `softfloat` (квадрат длины в 1/256 тайла < 2^53).
pub const MAX_VECTOR_TILES: f64 = 16.0;

fn check_range(what: &str, i: usize, name: &str, v: [f64; 2], max: f64) -> Result<(), LayoutError> {
    if !v.iter().all(|x| x.is_finite() && x.abs() <= max) {
        return err(format!("сущность {i} {name}: {what} {v:?} вне диапазона (не больше {max} тайлов по модулю)"));
    }
    Ok(())
}

/// Тайл → 1/256 с усечением вниз; значение уже проверено [`check_range`].
fn floor256(v: f64) -> Fx {
    (v * 256.0).floor() as Fx
}

pub fn build(scene: &Scene, protos: &Prototypes) -> Result<World, LayoutError> {
    let mut item_names: Vec<String> = Vec::new();
    let mut item_stack: Vec<u32> = Vec::new();
    let mut belt_protos: Vec<BeltProto> = Vec::new();
    let mut ins_protos: Vec<InserterProto> = Vec::new();
    let item_id = |name: &str,
                       names: &mut Vec<String>,
                       stacks: &mut Vec<u32>|
     -> Result<ItemId, LayoutError> {
        if let Some(i) = names.iter().position(|n| n == name) {
            return ItemId::try_from(i).or_else(|_| err(format!("слишком много видов предметов ({name})")));
        }
        let Some(&ss) = protos.stack_size.get(name) else {
            return err(format!("неизвестный предмет {name}"));
        };
        let Ok(id) = ItemId::try_from(names.len()) else {
            return err(format!("слишком много видов предметов ({name})"));
        };
        names.push(name.to_string());
        stacks.push(ss);
        Ok(id)
    };

    let mut belts: Vec<BeltTile> = Vec::new();
    let mut inserters: Vec<Inserter> = Vec::new();
    let mut chests: Vec<Chest> = Vec::new();
    let mut at: HashMap<(i32, i32), Kind> = HashMap::new();

    for (i, e) in scene.entities.iter().enumerate() {
        check_range("позиция", i, &e.name, e.position, MAX_COORD_TILES)?;
        if let Some(v) = e.pickup {
            check_range("pickup", i, &e.name, v, MAX_VECTOR_TILES)?;
        }
        if let Some(v) = e.drop {
            check_range("drop", i, &e.name, v, MAX_VECTOR_TILES)?;
        }
        let pos = (fx_from_f64(e.position[0]), fx_from_f64(e.position[1]));
        if pos.0.rem_euclid(TILE) != TILE / 2 || pos.1.rem_euclid(TILE) != TILE / 2 {
            return err(format!("сущность {i} {}: центр не в середине тайла", e.name));
        }
        let dir = match e.direction.as_deref() {
            None | Some("north") => Dir::North,
            Some("east") => Dir::East,
            Some("south") => Dir::South,
            Some("west") => Dir::West,
            Some(d) => return err(format!("сущность {i}: плохое direction {d}")),
        };
        let kind = if let Some(bp) = protos.belts.get(&e.name) {
            let proto = proto_index(&mut belt_protos, bp);
            belts.push(BeltTile { pos, dir, proto, line: 0, k: 0 });
            Kind::Belt(belts.len() - 1)
        } else if let Some(ip) = protos.inserters.get(&e.name) {
            let proto = proto_index(&mut ins_protos, ip);
            // Векторы прототипа заданы для «севера» и поворачиваются с direction
            // (проверено по эталону: direction — сторона, откуда берёт).
            // Точки на карте — 1/256 с усечением к нулю; длина и угол руки — от точного вектора
            // прототипа (Q48): цель вылета у выгрузки — |insert_position| = 1.2, а не 307/256
            // (проверено по эталону: рука у выгрузки на 1.2 тайла).
            let mut pv = dir.rotate(ip.pickup);
            let mut dv = dir.rotate(ip.insert);
            let mut pq = rot64(dir, ip.pickup_q48);
            let mut dq = rot64(dir, ip.insert_q48);
            let (mut pl, mut dl) = (ip.pickup_len, ip.insert_len);
            // Кастомные векторы из сцены вместо повёрнутых векторов прототипа. Игра округляет
            // точку вниз до 1/256 (проверено в игре, docs/mechanics/inserters.md); рука идёт
            // к записанной точке.
            let q_of = |v: (Fx, Fx)| ((v.0 as i64) << 40, (v.1 as i64) << 40); // 1/256 → Q48
            if let Some(v) = e.pickup {
                pv = (floor256(v[0]), floor256(v[1]));
                pq = q_of(pv);
                pl = hypot_f64(pv.0 as i64, pv.1 as i64);
            }
            if let Some(v) = e.drop {
                dv = (floor256(v[0]), floor256(v[1]));
                dq = q_of(dv);
                dl = hypot_f64(dv.0 as i64, dv.1 as i64);
            }
            let at_pickup = HandTarget { o: orientation_f32(pq.0, pq.1), ext: pl };
            let at_drop = HandTarget { o: orientation_f32(dq.0, dq.1), ext: dl };
            inserters.push(Inserter {
                pos,
                pickup_point: (pos.0 + pv.0, pos.1 + pv.1),
                proto,
                pickup: Target::None,
                drop: Target::None,
                drop_point: (pos.0 + dv.0, pos.1 + dv.1),
                at_pickup,
                at_drop,
                // После прогрева рука стоит у точки захвата (эталон, тик 0), высота 0.
                hand: Hand { o: at_pickup.o, ext: at_pickup.ext, h: Sf::ZERO },
                held: None,
                held_count: 0,
                chase: None,
            });
            Kind::Inserter
        } else if let Some(cp) = protos.chests.get(&e.name) {
            let mut inv = Inventory::new(cp.slots);
            if let Some(items) = &e.items {
                for (name, &count) in items {
                    let id = item_id(name, &mut item_names, &mut item_stack)?;
                    let ss = item_stack[id as usize];
                    if inv.insert(id, count, ss) != count {
                        return err(format!("сундук {i} не вместил {name}"));
                    }
                }
            }
            chests.push(Chest { pos, inv, fresh: Vec::new() });
            Kind::Chest(chests.len() - 1)
        } else if let Some(why) = protos.unsupported.get(&e.name) {
            return err(format!("сущность {i}: {} не поддержан моделью: {why}", e.name));
        } else {
            return err(format!("сущность {i}: тип {} не поддержан в M0", e.name));
        };
        let tile = tile_of(pos.0, pos.1);
        if at.insert(tile, kind).is_some() {
            return err(format!("сущность {i}: тайл {tile:?} уже занят"));
        }
    }

    // ---- линии лент ----
    let belt_at = |t: (i32, i32)| -> Option<usize> {
        match at.get(&t) {
            Some(Kind::Belt(b)) => Some(*b),
            _ => None,
        }
    };
    let front_of = |b: &BeltTile| -> (i32, i32) {
        let (tx, ty) = tile_of(b.pos.0, b.pos.1);
        let (dx, dy) = b.dir.unit();
        (tx + dx, ty + dy)
    };
    // Проверка: повороты и боковая загрузка в M0 не поддержаны.
    for (i, b) in belts.iter().enumerate() {
        if let Some(f) = belt_at(front_of(b)) {
            let fd = belts[f].dir;
            if fd != b.dir && fd != b.dir.opposite() {
                return err(format!(
                    "лента {:?} упирается в бок ленты {:?}: поворот/боковая загрузка — не M0",
                    belts[i].pos, belts[f].pos
                ));
            }
        }
    }
    // next[i] — лента впереди с тем же направлением (продолжение линии).
    let next: Vec<Option<usize>> = belts
        .iter()
        .map(|b| belt_at(front_of(b)).filter(|&f| belts[f].dir == b.dir))
        .collect();
    // Линия: от «головы» назад; голова — лента, у которой впереди нет ленты того же типа.
    let mut line_of: Vec<Option<u32>> = vec![None; belts.len()];
    let mut lines: Vec<BeltLine> = Vec::new();
    let protos: Vec<u8> = belts.iter().map(|b| b.proto).collect();
    let same_line = |a: usize, b: usize| protos[a] == protos[b];
    // prev[i] — лента позади (её next == i) с тем же типом
    let mut prev: Vec<Option<usize>> = vec![None; belts.len()];
    for (i, n) in next.iter().enumerate() {
        if let Some(n) = *n {
            prev[n] = Some(i);
        }
    }
    for head in 0..belts.len() {
        let is_head = match next[head] {
            None => true,
            Some(n) => !same_line(head, n),
        };
        if !is_head {
            continue;
        }
        let mut tiles = vec![head as u32];
        let mut cur = head;
        while let Some(p) = prev[cur] {
            if !same_line(p, cur) {
                break;
            }
            tiles.push(p as u32);
            cur = p;
        }
        let li = lines.len() as u32;
        for (k, &t) in tiles.iter().enumerate() {
            line_of[t as usize] = Some(li);
            belts[t as usize].line = li;
            belts[t as usize].k = k as u32;
        }
        let speed = belt_protos[belts[head].proto as usize].speed;
        lines.push(BeltLine { tiles, speed, lanes: [Lane::default(), Lane::default()], next: None });
    }
    // Кольцо из лент одного типа (без головы) — не M0.
    if line_of.iter().any(|l| l.is_none()) {
        return err("замкнутое кольцо лент — не M0".into());
    }
    // Связи линий разных типов.
    for line in lines.iter_mut() {
        let head = line.tiles[0] as usize;
        if let Some(n) = next[head] {
            line.next = Some(belts[n].line);
        }
    }
    let line_order = line_order(&lines)?;

    // ---- цели манипуляторов ----
    for ins in inserters.iter_mut() {
        let pick_tile = tile_of(ins.pickup_point.0, ins.pickup_point.1);
        let drop_tile = tile_of(ins.drop_point.0, ins.drop_point.1);
        ins.pickup = match at.get(&pick_tile) {
            Some(Kind::Chest(c)) => Target::Chest(*c as u32),
            Some(Kind::Belt(b)) => Target::Belt(*b as u32),
            _ => Target::None,
        };
        ins.drop = match at.get(&drop_tile) {
            Some(Kind::Chest(c)) => Target::Chest(*c as u32),
            Some(Kind::Belt(b)) => Target::Belt(*b as u32),
            _ => Target::None,
        };
        // Точка выгрузки ровно на хвосте линии (на её внешнем крае): поведение игры не сверено.
        if let Target::Belt(b) = ins.drop {
            let tile = &belts[b as usize];
            let (_, drop_pos) = tile.drop_lane_pos(ins.drop_point);
            if drop_pos >= lines[tile.line as usize].len() {
                return err(format!(
                    "манипулятор {:?}: точка выгрузки на хвосте линии лент — не сверено с игрой",
                    ins.pos
                ));
            }
        }
    }

    let ins_order = inserter_order(&inserters, &chests);
    Ok(World {
        tick: 0,
        next_uid: 0,
        fresh_uid: 0,
        item_names,
        item_stack,
        belt_protos,
        ins_protos,
        belts,
        lines,
        line_order,
        inserters,
        ins_order,
        chests,
    })
}

/// Порядок обновления манипуляторов.
///
/// Проверено по эталону (пробы `tests/fidelity/probes/t1*`, `c1*`, `c3*`): последний
/// разбуженный ходит первым. Объяснение (вывод из проб, исходников движка нет): у игры список
/// активных сущностей, и проснувшийся манипулятор встаёт в его **начало**. Мод строит сцену, ждёт прогрева (манипуляторы засыпают: брать
/// нечего) и в t = 0 наполняет сундуки по порядку сцены; наполнение сундука будит манипуляторы,
/// которые из него берут. Поэтому порядок — обратный порядку наполнения их сундуков-источников,
/// а не порядку постройки: пробы t1c/t1d (манипуляторы построены в одном порядке, сундуки — в
/// другом) решает порядок сундуков. Несколько манипуляторов у одного сундука просыпаются в
/// порядке постройки (последний построенный — первым; c1a–c1d). Наполнение сундука-приёмника
/// не будит (вариант «будит» ломает c4b).
///
/// Не проверено: относительный порядок манипуляторов, которых сундуки не будят (берут с ленты),
/// — они остаются в конце в порядке постройки; в игре они просыпаются позже, когда на тайл
/// приходит предмет (конкуренция стоков c2*, c4 — вне M0).
fn inserter_order(inserters: &[Inserter], chests: &[Chest]) -> Vec<u32> {
    // Номер «пробуждения»: индекс сундука-источника, если мод его наполняет (сундуки
    // наполняются по порядку сцены; пустые мод не трогает).
    let wake = |ins: &Inserter| match ins.pickup {
        Target::Chest(c) if chests[c as usize].inv.slots.iter().any(|s| s.is_some()) => Some(c),
        _ => None,
    };
    let mut woken: Vec<(u32, u32)> =
        inserters.iter().enumerate().filter_map(|(i, ins)| wake(ins).map(|c| (c, i as u32))).collect();
    // каждый проснувшийся встаёт в начало списка: позже разбуженный — раньше
    woken.sort_unstable_by(|a, b| b.cmp(a));
    let mut order: Vec<u32> = woken.into_iter().map(|(_, i)| i).collect();
    order.extend(inserters.iter().enumerate().filter(|(_, ins)| wake(ins).is_none()).map(|(i, _)| i as u32));
    order
}

/// Порядок обновления линий: каждая — после той, в которую выводит (`next`). У каждой
/// линии не больше одного `next`, поэтому граф — лес цепочек; кольцо из лент разных типов
/// (обычная → быстрая → обычная по кругу) — ошибка сцены. Без рекурсии.
fn line_order(lines: &[BeltLine]) -> Result<Vec<u32>, LayoutError> {
    let n = lines.len();
    // 0 — не посещена, 1 — в текущей цепочке, 2 — готова
    let mut state = vec![0u8; n];
    let mut order = Vec::with_capacity(n);
    let mut chain = Vec::new();
    for start in 0..n {
        let mut cur = Some(start);
        while let Some(li) = cur {
            match state[li] {
                2 => break,
                1 => return err("кольцо лент — не M0".into()),
                _ => {}
            }
            state[li] = 1;
            chain.push(li);
            cur = lines[li].next.map(|x| x as usize);
        }
        // цепочка от `start` вниз по течению: обновлять с конца
        while let Some(li) = chain.pop() {
            state[li] = 2;
            order.push(li as u32);
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(ents: serde_json::Value) -> Scene {
        serde_json::from_value(serde_json::json!({"name": "t", "ticks": 1, "entities": ents})).unwrap()
    }

    fn rejected(ents: serde_json::Value) -> String {
        match build(&scene(ents), Prototypes::bundled()) {
            Ok(_) => panic!("сцена должна быть отклонена"),
            Err(e) => e.0,
        }
    }

    /// Огромные позиции и векторы — `Err`, а не паника или переполнение.
    #[test]
    fn out_of_range_rejected() {
        let belt = |x: f64, y: f64| serde_json::json!({"name": "transport-belt", "position": [x, y], "direction": "east"});
        rejected(serde_json::json!([belt(2_000_000.5, 0.5)]));
        rejected(serde_json::json!([belt(0.5, -1e300)]));
        rejected(serde_json::json!([belt(0.5, f64::MAX)]));
        // граница: 10^6 тайлов ещё можно (центр в середине тайла, 999999.5)
        assert!(build(&scene(serde_json::json!([belt(999_999.5, -999_999.5)])), Prototypes::bundled()).is_ok());
        let ins = |pickup: serde_json::Value, drop: serde_json::Value| {
            serde_json::json!({"name": "inserter", "position": [0.5, 0.5], "pickup": pickup, "drop": drop})
        };
        rejected(serde_json::json!([ins(serde_json::json!([1e9, 0.0]), serde_json::json!([0.0, 1.0]))]));
        rejected(serde_json::json!([ins(serde_json::json!([0.0, -1.0]), serde_json::json!([0.0, -1e300]))]));
        rejected(serde_json::json!([ins(serde_json::json!([0.0, -1.0]), serde_json::json!([17.0, 0.0]))]));
        assert!(build(&scene(serde_json::json!([ins(serde_json::json!([0.0, -1.0]), serde_json::json!([16.0, 0.0]))])), Prototypes::bundled()).is_ok());
    }

    /// Точка выгрузки ровно на хвосте линии не сверена с игрой — сцена отклоняется.
    #[test]
    fn drop_on_line_tail_rejected() {
        let ents = |dx: f64| {
            serde_json::json!([
                {"name": "transport-belt", "position": [0.5, 0.5], "direction": "east"},
                {"name": "inserter", "position": [0.5, 1.5], "direction": "south",
                 "pickup": [0.0, 1.0], "drop": [dx, -0.796875]},
            ])
        };
        // x = 0.0 — западный край тайла: p = 256 = длина линии
        let why = rejected(ents(-0.5));
        assert!(why.contains("хвосте"), "{why}");
        // чуть внутрь тайла — обычная кладка
        assert!(build(&scene(ents(-0.49609375)), Prototypes::bundled()).is_ok());
    }

    #[test]
    fn mixed_type_ring_rejected() {
        // кольцо 2×2: обычная → быстрая → обычная → быстрая по кругу (у каждой линии есть голова)
        let s = scene(serde_json::json!([
            {"name": "transport-belt", "position": [0.5, 0.5], "direction": "east"},
            {"name": "fast-transport-belt", "position": [1.5, 0.5], "direction": "south"},
            {"name": "transport-belt", "position": [1.5, 1.5], "direction": "west"},
            {"name": "fast-transport-belt", "position": [0.5, 1.5], "direction": "north"},
        ]));
        // повороты в M0 не поддержаны — кольцо отсекается раньше; прямое кольцо невозможно,
        // поэтому проверяем сам обход на искусственном графе ниже.
        assert!(build(&s, Prototypes::bundled()).is_err());
        let line = |next: Option<u32>| BeltLine { tiles: vec![0], speed: 8, lanes: Default::default(), next };
        assert!(line_order(&[line(Some(1)), line(Some(2)), line(Some(0))]).is_err());
        assert_eq!(line_order(&[line(Some(1)), line(None), line(Some(0))]).unwrap(), vec![1, 0, 2]);
    }
}
