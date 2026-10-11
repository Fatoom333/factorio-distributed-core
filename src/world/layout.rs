//! Формат сцены и эталона (контракт мода и модели, см. tests/fidelity/README.md)
//! и сборка мира из сцены.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::belt::{ItemId, Line, LineGroup, TileLane};
use super::fixed::{fx_from_f64, hypot_f64, orientation_f32, Fx, TILE};
use super::inserter::{Hand, HandTarget, Inserter, Target};
use super::inventory::{Chest, Inventory};
use super::proto::{BeltProto, InserterProto, Prototypes, TILE_LANE_LEN};
use super::softfloat::Sf;
use super::wake::{WakeRef, NONE};
use super::{BeltTile, Dir, World};

// ---------- сцена ----------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scene {
    pub name: String,
    pub ticks: u64,
    pub entities: Vec<SceneEntity>,
    /// Сколько тиков прошло от постройки сцены до t = 0 (прогрев, см. [`settle`]). От него
    /// зависят линии лент (первые до 600 тиков после постройки каждая лента — своя линия) и
    /// руки манипуляторов (едут от начальной позы к точке захвата). Без поля модель считает,
    /// что прошло [`WARMUP_SETTLED`] тиков (столько ждут съёмщики): линии слиты, руки на месте,
    /// все манипуляторы спят.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warmup: Option<u64>,
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
    /// (мод с `allow_custom_vectors`). Мод записывает абсолютную точку «центр + вектор», игра
    /// хранит её в 1/256 с усечением к нулю по абсолютной координате.
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

/// Кастомная точка манипулятора: абсолютная координата `центр + вектор` (тайлы, `double`, как её
/// считает мод) → 1/256 с усечением к нулю; возвращает смещение от центра в 1/256. При
/// положительной координате это усечение вниз, при отрицательной — вверх (проверено по эталонам:
/// `drop` = 1.2 даёт смещение 307/256 при положительной абсолютной координате и 308/256 при
/// отрицательной). Значения уже проверены [`check_range`].
fn custom_point(center: f64, v: f64, center_fx: Fx) -> Fx {
    ((center + v) * 256.0).trunc() as Fx - center_fx
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
    // номер сущности сцены у каждой ленты — порядок постройки
    let mut built: Vec<usize> = Vec::new();

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
            let lane = |line: u32| TileLane { line, start: 0, touched: None };
            let b = u32::try_from(belts.len()).ok().and_then(|b| b.checked_mul(2)).ok_or_else(|| LayoutError("слишком много лент".into()))?;
            // каждая лента сначала — своя линия на каждую полосу (как сразу после постройки)
            belts.push(BeltTile { pos, dir, proto, lanes: [lane(b), lane(b + 1)] });
            built.push(i);
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
            // Кастомные векторы из сцены вместо повёрнутых векторов прототипа. Игра хранит
            // абсолютную точку в 1/256 с усечением к нулю (docs/mechanics/inserters.md); рука идёт
            // к записанной точке.
            let q_of = |v: (Fx, Fx)| ((v.0 as i64) << 40, (v.1 as i64) << 40); // 1/256 → Q48
            if let Some(v) = e.pickup {
                pv = (custom_point(e.position[0], v[0], pos.0), custom_point(e.position[1], v[1], pos.1));
                pq = q_of(pv);
                pl = hypot_f64(pv.0 as i64, pv.1 as i64);
            }
            if let Some(v) = e.drop {
                dv = (custom_point(e.position[0], v[0], pos.0), custom_point(e.position[1], v[1], pos.1));
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
                // Только что построенный манипулятор: рука смотрит в сторону `direction`
                // (откуда берёт), вылет — `starting_distance` прототипа, высота 0; идёт брать.
                // К точке захвата рука приезжает за прогрев (`settle`).
                hand: Hand {
                    o: match dir {
                        Dir::North => 0,
                        Dir::East => 1 << 62,
                        Dir::South => 2 << 62,
                        Dir::West => 3 << 62,
                    },
                    ext: ip.starting_distance,
                    h: Sf::ZERO,
                },
                held: None,
                held_count: 0,
                chase: None,
                has_target: false,
                pickup_mode: true,
                awake: false,
                act_prev: NONE,
                act_next: NONE,
                regs: Vec::new(),
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
            chests.push(Chest::new(pos, inv));
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
    // next[i] — лента впереди с тем же направлением (продолжение цепочки); prev — обратное.
    let next: Vec<Option<usize>> = belts
        .iter()
        .map(|b| belt_at(front_of(b)).filter(|&f| belts[f].dir == b.dir))
        .collect();
    let mut prev: Vec<Option<usize>> = vec![None; belts.len()];
    for (i, n) in next.iter().enumerate() {
        if let Some(n) = *n {
            prev[n] = Some(i);
        }
    }
    // Цепочки: от последней по ходу ленты назад. Полоса цепочки — группа линий.
    let mut group_of: Vec<Option<u32>> = vec![None; belts.len()];
    let mut heads: Vec<usize> = Vec::new();
    for (head, n) in next.iter().enumerate() {
        if n.is_some() {
            continue;
        }
        let g = (heads.len() * 2) as u32;
        heads.push(head);
        let mut cur = Some(head);
        while let Some(c) = cur {
            group_of[c] = Some(g);
            cur = prev[c];
        }
    }
    // Кольцо лент (цепочка без последней ленты) — не M0.
    if group_of.iter().any(|g| g.is_none()) {
        return err("замкнутое кольцо лент — не M0".into());
    }
    let mut lines: Vec<Line> = Vec::with_capacity(belts.len() * 2);
    for (b, tile) in belts.iter().enumerate() {
        let speed = belt_protos[tile.proto as usize].speed;
        let g = group_of[b].expect("проверено выше");
        for lane in 0..2u8 {
            let mut l = Line::new(lane, g + lane as u32, speed, TILE_LANE_LEN);
            l.tiles.push(b as u32);
            lines.push(l);
        }
    }

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
    }

    let mut world = World {
        tick: 0,
        next_uid: 0,
        item_names,
        item_stack,
        belt_protos,
        ins_protos,
        belts,
        lines,
        free_lines: Vec::new(),
        line_groups: vec![LineGroup::default(); heads.len() * 2],
        active_groups: Vec::new(),
        line_alarms: Vec::new(),
        group_scratch: Vec::new(),
        chain_scratch: Vec::new(),
        line_cursor: None,
        remerges: Vec::new(),
        remerge_seq: 0,
        inserters,
        ins_head: NONE,
        chests,
    };
    settle(&mut world, &next, &built, scene.warmup);
    Ok(world)
}

/// Прогрев длиннее этого ничего не меняет: за 600 тиков исполняются все заявки на слияние
/// линий, а рука с самым дальним допустимым вектором ([`MAX_VECTOR_TILES`]) доезжает до точки
/// захвата меньше чем за 700 тиков. Съёмщики (`scene_capture`, `fuzz_capture`) ждут после
/// постройки столько же.
pub const WARMUP_SETTLED: u64 = 1024;

/// Прогрев сцены: от постройки до t = 0 — так, как это происходит в игре при съёмке эталона.
///
/// 1. Сущности строятся в одном тике по порядку сцены. Построенный манипулятор активен и встаёт
///    в начало списка активных; рука — в начальной позе. Ленты — по линии на тайл, с заявками
///    на слияние ([`connect_belts`]). Сундуки пока пустые.
/// 2. `warmup` тиков мир идёт сам: руки едут к точкам захвата, манипуляторы засыпают (брать
///    нечего) — каждый в тот тик, когда доехала его рука; линии лент сливаются и при этом будят
///    тех, кто на них спал. Порядок записи в общий список спящих — по тику прихода руки, при
///    равенстве — по списку активных (построенный позже — раньше).
/// 3. В t = 0 мод наполняет сундуки по порядку сцены; каждое наполнение будит спящих у этого
///    сундука.
///
/// Отсюда начальный порядок манипуляторов (проверено по эталону: пробы `t1*`, `c1*`, `c3*`,
/// `ins-warm-*`; docs/mechanics/inserters.md, «Начальный порядок»).
fn settle(world: &mut World, next: &[Option<usize>], built: &[usize], warmup: Option<u64>) {
    let w = warmup.unwrap_or(WARMUP_SETTLED).min(WARMUP_SETTLED) as i64;
    connect_belts(world, next, built, -w);
    for i in 0..world.inserters.len() {
        world.wake(i);
    }
    let stock: Vec<Inventory> = world
        .chests
        .iter_mut()
        .map(|c| {
            let empty = Inventory::new(c.inv.slots.len() as u32);
            std::mem::replace(&mut c.inv, empty)
        })
        .collect();
    for now in -w..0 {
        // все спят и заявок на слияние нет — дальше ничего не изменится
        if world.ins_head == NONE && world.remerges.is_empty() {
            break;
        }
        world.step_belts(now);
        world.step_inserters(now);
    }
    for (c, inv) in stock.into_iter().enumerate() {
        let filled = inv.slots.iter().any(|s| s.is_some());
        world.chests[c].inv = inv;
        if filled {
            world.alarm(WakeRef::ChestOut(c as u32));
        }
    }
}

/// Состояние линий после постройки сцены (постройка — в тике `start`).
///
/// Новая лента — своя линия на каждую полосу. Когда лента соединяется с соседней (та, что
/// построена позже, соединяется с уже стоящей), на последнюю по ходу линию цепочки ставится
/// заявка на перенарезку; первая сработавшая заявка сливает всю цепочку. Поэтому первые до 600
/// тиков после постройки линии стоят по одному тайлу, а момент слияния зависит от порядка
/// постройки и координат (проверено по эталону: пробы `build-*`, прогрев 60 тиков).
///
/// Все сущности сцены строятся в одном тике, по порядку сцены. Заявки исполняет прогрев
/// ([`settle`]).
fn connect_belts(world: &mut World, next: &[Option<usize>], built: &[usize], start: i64) {
    let link = |world: &mut World, up: usize, down: usize| {
        for lane in 0..2 {
            let (u, d) = ((up * 2 + lane) as u32, (down * 2 + lane) as u32);
            world.lines[u as usize].out = Some(d);
            world.lines[d as usize].inp = Some(u);
            world.schedule_remerge(u, start);
        }
    };
    // по порядку постройки: лента соединяется с теми соседями, что уже стоят
    let mut prev: Vec<Option<usize>> = vec![None; next.len()];
    for (i, n) in next.iter().enumerate() {
        if let Some(n) = *n {
            prev[n] = Some(i);
        }
    }
    let mut order: Vec<usize> = (0..next.len()).collect();
    order.sort_by_key(|&b| built[b]);
    let mut done = vec![false; next.len()];
    for b in order {
        if let Some(n) = next[b].filter(|&n| done[n]) {
            link(world, b, n);
        }
        if let Some(p) = prev[b].filter(|&p| done[p]) {
            link(world, p, b);
        }
        done[b] = true;
    }
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

    /// Точка выгрузки ровно на заднем крае тайла — кладка на 255 (пробы `edge-*`).
    #[test]
    fn drop_on_tile_back_edge() {
        let ents = |dx: f64| {
            serde_json::json!([
                {"name": "transport-belt", "position": [0.5, 0.5], "direction": "east"},
                {"name": "inserter", "position": [0.5, 1.5], "direction": "south",
                 "pickup": [0.0, 1.0], "drop": [dx, -0.796875]},
            ])
        };
        let w = build(&scene(ents(-0.5)), Prototypes::bundled()).unwrap();
        assert_eq!(w.belts[0].drop_lane_pos(w.inserters[0].drop_point), (1, 255));
        let w = build(&scene(ents(-0.49609375)), Prototypes::bundled()).unwrap();
        assert_eq!(w.belts[0].drop_lane_pos(w.inserters[0].drop_point), (1, 255));
        let w = build(&scene(ents(0.49609375)), Prototypes::bundled()).unwrap();
        assert_eq!(w.belts[0].drop_lane_pos(w.inserters[0].drop_point), (1, 1));
    }

    /// Начальная нарезка: цепочка одной скорости — одна линия на полосу; стык скоростей —
    /// граница линий; сразу после постройки (прогрев 0) — по линии на тайл.
    #[test]
    fn initial_lines() {
        let belt = |name: &str, x: f64| serde_json::json!({"name": name, "position": [x, 0.5], "direction": "east"});
        let ents = serde_json::json!([
            belt("transport-belt", 0.5), belt("transport-belt", 1.5), belt("fast-transport-belt", 2.5), belt("fast-transport-belt", 3.5),
        ]);
        let w = build(&scene(ents.clone()), Prototypes::bundled()).unwrap();
        let alive: Vec<_> = w.lines.iter().filter(|l| l.alive).map(|l| (l.lane, l.tiles.clone())).collect();
        assert_eq!(alive.len(), 4);
        assert!(alive.contains(&(0, vec![3, 2])) && alive.contains(&(1, vec![1, 0])));
        assert_eq!((w.belts[0].lanes[0].start, w.belts[1].lanes[0].start), (256, 0));
        assert!(w.remerges.is_empty() && w.line_groups.iter().all(|g| g.active.is_empty()));
        let mut sc = scene(ents);
        sc.warmup = Some(0);
        let w = build(&sc, Prototypes::bundled()).unwrap();
        assert_eq!(w.lines.iter().filter(|l| l.alive).count(), 8);
        assert!(!w.remerges.is_empty());
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
        // повороты в M0 не поддержаны — кольцо отсекается при сборке
        assert!(build(&s, Prototypes::bundled()).is_err());
    }
}
