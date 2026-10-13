//! Формат сцены и эталона (контракт мода и модели, см. tests/fidelity/README.md)
//! и сборка мира из сцены.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::belt::{ItemId, Line, LineGroup, SideOut, TileLane};
use super::fixed::{fx_from_f64, hypot_f64, orientation_f32, Fx, TILE};
use super::inserter::{Hand, HandTarget, Inserter, Target};
use super::inventory::{Chest, Inventory};
use super::proto::{
    BeltProto, InserterProto, Prototypes, SIDE_FAR, SIDE_NEAR, TILE_LANE_LEN, TURN_INNER_LEN, TURN_OUTER_LEN, UG_SIDE_IN,
    UG_SIDE_OUT, UG_SURFACE_LEN,
};
use super::softfloat::Sf;
use super::wake::{WakeRef, NONE};
use super::{BeltTile, Dir, TileKind, World};

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
    /// Вид подземной ленты: `"input"` (вход под землю) или `"output"` (выход). Обязательно у
    /// подземной ленты и только у неё; `direction` у обоих — куда едут предметы.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ug_type: Option<String>,
}

/// Вид подземной ленты из поля `type` сцены.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UgType {
    Input,
    Output,
}

impl SceneEntity {
    /// Проверка поля `type`: у подземной ленты (`is_underground`) — обязательное `input` /
    /// `output`, у остальных сущностей поля быть не должно.
    pub fn underground_type(&self, i: usize, is_underground: bool) -> Result<Option<UgType>, LayoutError> {
        match (is_underground, self.ug_type.as_deref()) {
            (true, Some("input")) => Ok(Some(UgType::Input)),
            (true, Some("output")) => Ok(Some(UgType::Output)),
            (true, Some(t)) => err(format!("сущность {i} {}: плохой type {t:?} (нужно \"input\" или \"output\")", self.name)),
            (true, None) => err(format!("сущность {i} {}: у подземной ленты нужен type (\"input\" или \"output\")", self.name)),
            (false, Some(_)) => err(format!("сущность {i} {}: type только у подземной ленты", self.name)),
            (false, None) => Ok(None),
        }
    }
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
    /// Только у подземной ленты: линии 3 и 4 сущности — подземная часть (у входа с парой —
    /// предметы под землёй, `p` — расстояние до центра тайла выхода; у выхода и у подземки без
    /// пары игра эти линии не использует). У обычной ленты поля нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<[Vec<DumpItem>; 2]>,
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
    /// Один из четырёх тайлов прямого рельса 2×2 (без вагона).
    Rail,
}

/// Куда выходит полоса тайла: прямо в ту же полосу тайла `0` (то же направление, поворот,
/// который она питает, или следующий кусок подземки) или вбок — в полосу `lane` тайла `to`, в
/// точку `off` от начала его полосы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BeltLink {
    Direct(usize),
    Side { to: usize, lane: u8, off: Fx },
}

impl BeltLink {
    fn target(self) -> usize {
        match self {
            BeltLink::Direct(f) | BeltLink::Side { to: f, .. } => f,
        }
    }
}

/// Выходы обеих полос тайла (у «половинной» боковой загрузки в подземку выход есть у одной).
type TileLinks = [Option<BeltLink>; 2];

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
    // подземки: (тайл, предел дальности пары)
    let mut ug_reach: Vec<(usize, i32)> = Vec::new();

    for (i, e) in scene.entities.iter().enumerate() {
        check_range("позиция", i, &e.name, e.position, MAX_COORD_TILES)?;
        if let Some(v) = e.pickup {
            check_range("pickup", i, &e.name, v, MAX_VECTOR_TILES)?;
        }
        if let Some(v) = e.drop {
            check_range("drop", i, &e.name, v, MAX_VECTOR_TILES)?;
        }
        let pos = (fx_from_f64(e.position[0]), fx_from_f64(e.position[1]));
        let ug = e.underground_type(i, protos.undergrounds.contains_key(&e.name))?;
        if e.name == "straight-rail" {
            // Прямой рельс 2×2: центр на границе тайлов, занимает четыре тайла. Вагонов и
            // поездов в модели нет, поэтому рельс — всегда «без вагона» (манипулятор ждёт поезд).
            if pos.0.rem_euclid(TILE) != 0 || pos.1.rem_euclid(TILE) != 0 {
                return err(format!("сущность {i} {}: центр рельса не на границе тайлов", e.name));
            }
            if !matches!(e.direction.as_deref(), None | Some("north") | Some("east")) {
                return err(format!("сущность {i} {}: прямой рельс — только north/east", e.name));
            }
            let (cx, cy) = (pos.0.div_euclid(TILE), pos.1.div_euclid(TILE));
            for t in [(cx - 1, cy - 1), (cx, cy - 1), (cx - 1, cy), (cx, cy)] {
                if at.insert(t, Kind::Rail).is_some() {
                    return err(format!("сущность {i}: тайл {t:?} уже занят"));
                }
            }
            continue;
        }
        if matches!(e.name.as_str(), "cargo-wagon" | "fluid-wagon" | "artillery-wagon" | "locomotive") {
            return err(format!("сущность {i}: {} не поддержан моделью: вагоны и поезда не считаются", e.name));
        }
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
            let lane = |line: u32| TileLane { line, start: 0, len: TILE_LANE_LEN, touched: None, strong: false };
            let b = u32::try_from(belts.len()).ok().and_then(|b| b.checked_mul(2)).ok_or_else(|| LayoutError("слишком много лент".into()))?;
            // каждая лента сначала — своя линия на каждую полосу (как сразу после постройки);
            // форма и длины полос — ниже, когда известны соседи
            belts.push(BeltTile { kind: TileKind::Belt, under: NONE, pos, dir, proto, shape: 0, unit: i as u32, lanes: [lane(b), lane(b + 1)] });
            built.push(i);
            Kind::Belt(belts.len() - 1)
        } else if let (Some(up), Some(ug)) = (protos.undergrounds.get(&e.name), ug) {
            // Подземка: в сцене — надземная половина тайла (полосы по 128); подземная часть
            // добавляется ниже, когда найдены пары.
            let proto = proto_index(&mut belt_protos, &BeltProto { name: up.name.clone(), speed: up.speed });
            let lane = |line: u32| TileLane { line, start: 0, len: UG_SURFACE_LEN, touched: None, strong: false };
            let b = u32::try_from(belts.len()).ok().and_then(|b| b.checked_mul(2)).ok_or_else(|| LayoutError("слишком много лент".into()))?;
            let kind = if ug == UgType::Input { TileKind::UgIn } else { TileKind::UgOut };
            belts.push(BeltTile { kind, under: NONE, pos, dir, proto, shape: 0, unit: i as u32, lanes: [lane(b), lane(b + 1)] });
            built.push(i);
            ug_reach.push((belts.len() - 1, up.max_distance as i32));
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

    // ---- пары подземок ----
    let belt_at = |t: (i32, i32)| -> Option<usize> {
        match at.get(&t) {
            Some(Kind::Belt(b)) => Some(*b),
            _ => None,
        }
    };
    let add = |t: (i32, i32), v: (i32, i32)| (t.0 + v.0, t.1 + v.1);
    // Куда «смотрит» туннель: вход — вперёд по ходу, выход — назад. Пара — первая подземка того
    // же прототипа на этом луче не дальше предела дальности, если она смотрит навстречу; смотрит
    // туда же — пары нет; поперечные, чужие прототипы и прочие сущности луч не видит
    // (docs/mechanics/belts.md, «Подземные ленты»; пробы `ug-r-pair-*`).
    let face = |b: &BeltTile| if b.kind == TileKind::UgIn { b.dir } else { b.dir.opposite() };
    let mut pairs: Vec<(usize, usize, i32)> = Vec::new(); // (вход, выход, расстояние)
    for &(u, reach) in &ug_reach {
        let (ub, f) = (&belts[u], face(&belts[u]));
        let t0 = tile_of(ub.pos.0, ub.pos.1);
        for d in 1..=reach {
            let (fx, fy) = f.unit();
            let Some(o) = belt_at((t0.0 + fx * d, t0.1 + fy * d)) else { continue };
            let ob = &belts[o];
            if ob.kind == TileKind::Belt || ob.proto != ub.proto {
                continue;
            }
            let of = face(ob);
            if of == f {
                break;
            }
            if of != f.opposite() {
                continue;
            }
            if ob.kind == ub.kind {
                // Игра в таком случае сама переделывает поставленный позже конец (меняет вид и
                // направление): такой сцены в игре не бывает, модель её не считает.
                return err(format!(
                    "сущность {}: {} не поддержан моделью: два одинаковых конца подземки смотрят друг на друга (сущность {})",
                    ub.unit, belt_protos[ub.proto as usize].name, ob.unit
                ));
            }
            if ub.kind == TileKind::UgIn {
                pairs.push((u, o, d));
            }
            break;
        }
    }
    // Состояние подземок в первые тики после постройки (кто с кем уже соединён, когда сливаются
    // линии пары) с игрой не сверено: модель считает только сцену после прогрева.
    if !ug_reach.is_empty() && scene.warmup.is_some_and(|w| w < WARMUP_SETTLED) {
        return err(format!(
            "подземные ленты с прогревом меньше {WARMUP_SETTLED} тиков не поддержаны моделью: состояние сразу после постройки не сверено"
        ));
    }
    // подземная часть пары — скрытый тайл длиной D · 256 после всех тайлов сцены
    let mut under_out: HashMap<usize, usize> = HashMap::new();
    for &(i, o, d) in &pairs {
        let h = belts.len();
        let b = u32::try_from(h).ok().and_then(|b| b.checked_mul(2)).ok_or_else(|| LayoutError("слишком много лент".into()))?;
        let lane = |line: u32| TileLane { line, start: 0, len: d * TILE_LANE_LEN, touched: None, strong: false };
        let src = &belts[i];
        let tile = BeltTile { kind: TileKind::Under, under: NONE, pos: src.pos, dir: src.dir, proto: src.proto, shape: 0, unit: src.unit, lanes: [lane(b), lane(b + 1)] };
        belts.push(tile);
        belts[i].under = h as u32;
        // НЕ СВЕРЕНО С ИГРОЙ (допущение): место скрытого тайла в порядке постройки — вместе с
        // концом пары, построенным позже. На результат сейчас не влияет: прогрев короче
        // `WARMUP_SETTLED` с подземками отклонён выше. Понадобится при коротком прогреве и
        // постройке на ходу — тогда снять пробами.
        // Номер постройки скрытого тайла (`unit` — как у входа) сверен наполовину: в кольце,
        // где первым построен выход, голова кольца — не подземная часть (пробы
        // `ug-y-ring-out-first*` расходятся, если дать ей номер выхода); точное значение не различено.
        built.push(built[i].max(built[o]));
        under_out.insert(h, o);
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
    // Форма ленты (выведено из наблюдений, docs/mechanics/belts.md, «Повороты»): лента «питает»
    // соседа, если смотрит в него. Питает сзади — прямая; иначе питает ровно один бок — поворот
    // (вход слева — налево, справа — направо); оба бока или никто — прямая. Выход подземки питает
    // как лента, вход подземки не питает никого (перед ним — он сам). У подземки формы нет.
    let feeds_into = |from: (i32, i32), to: (i32, i32)| -> bool {
        belt_at(from).is_some_and(|x| belts[x].kind != TileKind::UgIn && add(from, belts[x].dir.unit()) == to)
    };
    let shapes: Vec<u8> = belts
        .iter()
        .map(|bt| {
            if bt.kind != TileKind::Belt {
                return 0;
            }
            let (d, t) = (bt.dir, tile_of(bt.pos.0, bt.pos.1));
            let (ux, uy) = d.unit();
            let (rx, ry) = d.right();
            let back = feeds_into((t.0 - ux, t.1 - uy), t);
            let left = feeds_into((t.0 - rx, t.1 - ry), t);
            let right = feeds_into((t.0 + rx, t.1 + ry), t);
            if back || left == right {
                0
            } else if left {
                1
            } else {
                2
            }
        })
        .collect();
    for (b, &shape) in shapes.iter().enumerate() {
        if belts[b].kind != TileKind::Belt {
            continue;
        }
        let lens = match shape {
            1 => [TURN_INNER_LEN, TURN_OUTER_LEN],
            2 => [TURN_OUTER_LEN, TURN_INNER_LEN],
            _ => [TILE_LANE_LEN, TILE_LANE_LEN],
        };
        belts[b].shape = shape;
        for (tl, len) in belts[b].lanes.iter_mut().zip(lens) {
            tl.len = len;
        }
    }
    // Выход каждого тайла. Лента и выход подземки: прямо (в ленту того же направления, в поворот,
    // который они питают, или во вход подземки того же направления), вбок (в бок прямой ленты или
    // подземки) или никуда (впереди пусто, лента навстречу или выход подземки спиной). Вход
    // подземки: в подземную часть пары (нет пары — тупик); подземная часть — в выход.
    let links: Vec<TileLinks> = belts
        .iter()
        .enumerate()
        .map(|(bi, b)| {
            match b.kind {
                TileKind::Under => return [Some(BeltLink::Direct(under_out[&bi])); 2],
                TileKind::UgIn => return [(b.under != NONE).then_some(BeltLink::Direct(b.under as usize)); 2],
                TileKind::Belt | TileKind::UgOut => {}
            }
            let Some(f) = belt_at(front_of(b)) else { return [None; 2] };
            let (fd, fk) = (belts[f].dir, belts[f].kind);
            if fd == b.dir.opposite() {
                return [None; 2];
            }
            if fd == b.dir {
                // за выходом подземки соединения нет: лента упирается в него, как в тупик
                return [(fk != TileKind::UgOut).then_some(BeltLink::Direct(f)); 2];
            }
            if belts[f].shape != 0 {
                return [Some(BeltLink::Direct(f)); 2];
            }
            // источник слева от приёмника — в его левую полосу, справа — в правую
            let (rx, ry) = fd.right();
            let ft = tile_of(belts[f].pos.0, belts[f].pos.1);
            let on_left = tile_of(b.pos.0, b.pos.1) == (ft.0 - rx, ft.1 - ry);
            let tl: u8 = if on_left { 0 } else { 1 };
            // полоса источника, ближняя к переднему краю приёмника
            let near = tl as usize;
            let mut out = [None; 2];
            match fk {
                // лента: ближняя полоса источника — в 67, дальняя — в 188
                TileKind::Belt => {
                    for (lane, o) in out.iter_mut().enumerate() {
                        let off = if lane == near { SIDE_NEAR } else { SIDE_FAR };
                        *o = Some(BeltLink::Side { to: f, lane: tl, off });
                    }
                }
                // Подземка принимает сбоку одну полосу: выход — ближнюю к своему переднему краю,
                // вход — ближнюю к заднему (надземная половина тайла); вторая полоса источника
                // остаётся без выхода (пробы `ug-side-*`, `ug-r-side-*`).
                TileKind::UgOut => out[near] = Some(BeltLink::Side { to: f, lane: tl, off: UG_SIDE_OUT }),
                TileKind::UgIn => out[1 - near] = Some(BeltLink::Side { to: f, lane: tl, off: UG_SIDE_IN }),
                TileKind::Under => unreachable!("подземной части нет на карте"),
            }
            out
        })
        .collect();
    // В полосу тайла напрямую входит не больше одной полосы: иначе цепочка линий не цепочка, и
    // обход её головы не кончится. Правила выше такого не дают; проверка — на случай их правки.
    let mut direct_in = vec![false; belts.len() * 2];
    for (b, l) in links.iter().enumerate() {
        for (lane, link) in l.iter().enumerate() {
            if let Some(BeltLink::Direct(f)) = *link {
                if std::mem::replace(&mut direct_in[f * 2 + lane], true) {
                    return err(format!(
                        "сущность {}: внутренняя ошибка сборки лент: в полосу {lane} тайла {} напрямую входят две ленты",
                        belts[b].unit, belts[f].unit
                    ));
                }
            }
        }
    }
    // Группы линий: связные куски графа «полоса тайла — полоса тайла» по прямым и боковым
    // соединениям (боковое соединение сливает группы источника и приёмника).
    let mut parent: Vec<usize> = (0..belts.len() * 2).collect();
    fn root(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for (b, l) in links.iter().enumerate() {
        for (lane, link) in l.iter().enumerate() {
            let to = match *link {
                Some(BeltLink::Direct(f)) => f * 2 + lane,
                Some(BeltLink::Side { to, lane: tl, .. }) => to * 2 + tl as usize,
                None => continue,
            };
            let (x, y) = (root(&mut parent, b * 2 + lane), root(&mut parent, to));
            parent[x] = y;
        }
    }
    let mut group_id: HashMap<usize, u32> = HashMap::new();
    let mut lines: Vec<Line> = Vec::with_capacity(belts.len() * 2);
    for (b, tile) in belts.iter().enumerate() {
        let speed = belt_protos[tile.proto as usize].speed;
        for lane in 0..2u8 {
            let r = root(&mut parent, b * 2 + lane as usize);
            let next_id = group_id.len() as u32;
            let g = *group_id.entry(r).or_insert(next_id);
            let mut l = Line::new(lane, g, speed, tile.lanes[lane as usize].len);
            l.tiles.push(b as u32);
            lines.push(l);
        }
    }
    let n_groups = group_id.len();

    // ---- цели манипуляторов ----
    for ins in inserters.iter_mut() {
        let pick_tile = tile_of(ins.pickup_point.0, ins.pickup_point.1);
        let drop_tile = tile_of(ins.drop_point.0, ins.drop_point.1);
        ins.pickup = match at.get(&pick_tile) {
            Some(Kind::Chest(c)) => Target::Chest(*c as u32),
            Some(Kind::Belt(b)) => Target::Belt(*b as u32),
            Some(Kind::Rail) => Target::Train,
            _ => Target::None,
        };
        // Тайл выгрузки без сущности — земля; другая сущность (манипулятор) не принимает.
        ins.drop = match at.get(&drop_tile) {
            Some(Kind::Chest(c)) => Target::Chest(*c as u32),
            Some(Kind::Belt(b)) => Target::Belt(*b as u32),
            Some(Kind::Inserter) => Target::None,
            Some(Kind::Rail) => Target::Train,
            None => Target::Ground,
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
        line_groups: vec![LineGroup::default(); n_groups],
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
        ground: Vec::new(),
    };
    settle(&mut world, &links, &built, scene.warmup);
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
fn settle(world: &mut World, links: &[TileLinks], built: &[usize], warmup: Option<u64>) {
    let w = warmup.unwrap_or(WARMUP_SETTLED).min(WARMUP_SETTLED) as i64;
    connect_belts(world, links, built, -w);
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
fn connect_belts(world: &mut World, links: &[TileLinks], built: &[usize], start: i64) {
    let link = |world: &mut World, up: usize| {
        for lane in 0..2 {
            let u = (up * 2 + lane) as u32;
            match links[up][lane] {
                None => continue,
                Some(BeltLink::Direct(down)) => {
                    let d = (down * 2 + lane) as u32;
                    world.lines[u as usize].out = Some(d);
                    world.lines[d as usize].inp = Some(u);
                }
                Some(BeltLink::Side { to: down, lane: tl, off }) => {
                    let d = (down * 2 + tl as usize) as u32;
                    world.lines[u as usize].out = Some(d);
                    world.lines[u as usize].side = Some(SideOut { belt: down as u32, lane: tl, off });
                    let mut sides = std::mem::take(&mut world.lines[d as usize].sides);
                    sides.push(u);
                    sides.sort_by_key(|&x| world.line_key(x));
                    world.lines[d as usize].sides = sides;
                }
            }
            world.schedule_remerge(u, start);
        }
    };
    // по порядку постройки: тайл соединяется с теми соседями, что уже стоят (подземная часть
    // пары появляется вместе с тем концом, что построен позже)
    let target = |l: &TileLinks| l.iter().flatten().next().map(|x| x.target());
    let mut order: Vec<usize> = (0..links.len()).collect();
    order.sort_by_key(|&b| built[b]);
    // кто входит в каждый тайл (сзади и с боков), по порядку постройки
    let mut feeders: Vec<Vec<usize>> = vec![Vec::new(); links.len()];
    for &p in &order {
        if let Some(f) = target(&links[p]) {
            feeders[f].push(p);
        }
    }
    let mut done = vec![false; links.len()];
    for &b in &order {
        if target(&links[b]).is_some_and(|f| done[f]) {
            link(world, b);
        }
        for &p in &feeders[b] {
            if done[p] {
                link(world, p);
            }
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

    /// Подземная лента: формат проверяется (обязательный `type`, только у подземки).
    #[test]
    fn underground_format() {
        let ug = |t: serde_json::Value| {
            let mut e = serde_json::json!({"name": "fast-underground-belt", "position": [0.5, 0.5], "direction": "east"});
            if !t.is_null() {
                e["type"] = t;
            }
            serde_json::json!([e])
        };
        assert!(build(&scene(ug(serde_json::json!("input"))), Prototypes::bundled()).is_ok());
        assert!(build(&scene(ug(serde_json::json!("output"))), Prototypes::bundled()).is_ok());
        assert!(rejected(ug(serde_json::json!("sideways"))).contains("плохой type"));
        assert!(rejected(ug(serde_json::Value::Null)).contains("нужен type"));
        let belt = serde_json::json!([{"name": "transport-belt", "position": [0.5, 0.5], "type": "input"}]);
        assert!(rejected(belt).contains("type только у подземной ленты"));
        // поле `type` сохраняется при записи сцены и не появляется у остальных сущностей
        let sc = scene(ug(serde_json::json!("output")));
        assert_eq!(serde_json::to_value(&sc).unwrap()["entities"][0]["type"], "output");
        let sc = scene(serde_json::json!([{"name": "transport-belt", "position": [0.5, 0.5]}]));
        assert!(serde_json::to_value(&sc).unwrap()["entities"][0].get("type").is_none());
        // эталон: у подземки есть "under", у ленты — нет
        let d: TickDump = serde_json::from_str(
            r#"{"t":0,"belts":[{"pos":[0.5,0.5],"lines":[[],[]]},{"pos":[1.5,0.5],"lines":[[],[]],"ug":"input","under":[[{"item":"coal","p":700}],[]]}],"inserters":[],"chests":[]}"#,
        )
        .unwrap();
        assert!(d.belts[0].under.is_none());
        assert_eq!(d.belts[1].under.as_ref().unwrap()[0][0].p, 700);
    }

    fn ug(name: &str, kind: &str, x: f64, y: f64, dir: &str) -> serde_json::Value {
        serde_json::json!({"name": name, "position": [x, y], "direction": dir, "type": kind})
    }

    /// Пара подземок: первая подземка того же прототипа на луче, не дальше предела; после
    /// прогрева лента, вход, подземная часть, выход и лента за ним — одна линия на полосу.
    #[test]
    fn underground_pairs_and_lines() {
        let belt = |x: f64| serde_json::json!({"name": "transport-belt", "position": [x, 0.5], "direction": "east"});
        let y = "underground-belt";
        let w = build(&scene(serde_json::json!([belt(-0.5), ug(y, "input", 0.5, 0.5, "east"), ug(y, "output", 3.5, 0.5, "east"), belt(4.5)])), Prototypes::bundled()).unwrap();
        assert_eq!(w.belts.len(), 5);
        let h = w.belts[1].under as usize;
        assert_eq!((h, w.belts[h].kind, w.belts[h].lanes[0].len), (4, TileKind::Under, 768));
        assert_eq!((w.belts[1].lanes[0].len, w.belts[2].lanes[1].len, w.belts[2].under), (128, 128, NONE));
        for lane in 0..2 {
            let l = &w.lines[w.belts[3].lanes[lane].line as usize];
            // от переднего тайла к заднему: лента, выход, подземная часть, вход, лента
            assert_eq!((l.tiles.clone(), l.len), (vec![3, 2, 4, 1, 0], 256 + 128 + 768 + 128 + 256));
            assert_eq!((w.belts[2].lanes[lane].start, w.belts[4].lanes[lane].start, w.belts[1].lanes[lane].start), (256, 384, 1152));
        }
        // эталон: у подземки есть `under`, скрытый тайл отдельной лентой не пишется
        let d = w.dump();
        assert_eq!(d.belts.len(), 4);
        assert!(d.belts[0].under.is_none() && d.belts[1].under.is_some() && d.belts[2].under.is_some());
        let paired = |ents: serde_json::Value| -> Vec<bool> {
            let w = build(&scene(ents), Prototypes::bundled()).unwrap();
            w.belts.iter().filter(|b| b.kind == TileKind::UgIn).map(|b| b.under != NONE).collect()
        };
        // предел дальности: 5 — пара, 6 — нет
        assert_eq!(paired(serde_json::json!([ug(y, "input", 0.5, 0.5, "east"), ug(y, "output", 5.5, 0.5, "east")])), [true]);
        assert_eq!(paired(serde_json::json!([ug(y, "input", 0.5, 0.5, "east"), ug(y, "output", 6.5, 0.5, "east")])), [false]);
        // ближний вход того же прототипа перехватывает выход; порядок постройки не важен
        let (a, b, c) = (ug(y, "input", 0.5, 0.5, "east"), ug(y, "input", 2.5, 0.5, "east"), ug(y, "output", 4.5, 0.5, "east"));
        assert_eq!(paired(serde_json::json!([a, b, c])), [false, true]);
        assert_eq!(paired(serde_json::json!([c, b, a])), [true, false]);
        // чужой прототип и поперечная подземка того же прототипа лучу не мешают
        let f = "fast-underground-belt";
        assert_eq!(paired(serde_json::json!([a, ug(f, "input", 2.5, 0.5, "east"), c])), [true, false]);
        assert_eq!(paired(serde_json::json!([a, ug(y, "input", 2.5, 0.5, "south"), c])), [true, false]);
        // выход с ходом навстречу смотрит туда же, куда вход, — пары нет
        assert_eq!(paired(serde_json::json!([a, ug(y, "output", 3.5, 0.5, "west")])), [false]);
        // два входа лицом друг к другу: игра переделывает второй — модель сцену не считает
        assert!(rejected(serde_json::json!([a, ug(y, "input", 3.5, 0.5, "west")])).contains("не поддержан моделью"));
        assert!(rejected(serde_json::json!([ug(y, "output", 0.5, 0.5, "west"), ug(y, "output", 3.5, 0.5, "east")])).contains("не поддержан моделью"));
        // состояние сразу после постройки подземок не сверено
        let mut sc = scene(serde_json::json!([a, c]));
        sc.warmup = Some(60);
        assert!(build(&sc, Prototypes::bundled()).is_err());
    }

    /// Стыки подземки: лента за выходом и лента навстречу выходу не соединяются; сбоку подземка
    /// принимает одну полосу источника (вход — дальнюю от своего переднего края, в 60; выход —
    /// ближнюю, в 67), вторая полоса источника остаётся без выхода.
    #[test]
    fn underground_links() {
        let y = "underground-belt";
        let belt = |x: f64, yy: f64, d: &str| serde_json::json!({"name": "transport-belt", "position": [x, yy], "direction": d});
        let w = build(
            &scene(serde_json::json!([
                ug(y, "input", 0.5, 0.5, "east"),   // 0
                ug(y, "output", 2.5, 0.5, "east"),  // 1
                belt(1.5, 0.5, "east"),             // 2: за выходом — тупик
                belt(0.5, -0.5, "south"),           // 3: слева от входа
                belt(2.5, 1.5, "north"),            // 4: справа от выхода
                belt(3.5, 0.5, "west"),             // 5: навстречу выходу
            ])),
            Prototypes::bundled(),
        )
        .unwrap();
        let line = |b: usize, lane: usize| &w.lines[w.belts[b].lanes[lane].line as usize];
        assert!(line(2, 0).out.is_none() && line(2, 1).out.is_none());
        assert!(line(1, 0).out.is_none() && line(5, 0).out.is_none());
        // слева от входа: правая полоса источника — в левую полосу входа, точка 60
        assert!(line(3, 0).out.is_none());
        let sd = line(3, 1).side.expect("выход вбок");
        assert_eq!((sd.belt, sd.lane, sd.off), (0, 0, 60));
        // справа от выхода: правая полоса источника — в правую полосу выхода, точка 67
        assert!(line(4, 0).out.is_none());
        let sd = line(4, 1).side.expect("выход вбок");
        assert_eq!((sd.belt, sd.lane, sd.off), (1, 1, 67));
        // лента с выходом подземки сбоку и пустым задом — поворот; вход подземки ленту не питает
        let w = build(
            &scene(serde_json::json!([ug(y, "output", 0.5, 0.5, "east"), belt(1.5, 0.5, "south"), ug(y, "input", 2.5, 1.5, "east"), belt(1.5, 1.5, "south")])),
            Prototypes::bundled(),
        )
        .unwrap();
        assert_eq!((w.belts[1].shape, w.belts[3].shape), (2, 0));
    }

    /// Кладка на подземку: точка зажимается в надземную половину тайла (вход — задняя, выход —
    /// передняя); позиция предмета на карте — как на ленте (пробы `ug-r-edge-*`, `ug-r-pick-*`).
    #[test]
    fn underground_drop_and_item_pos() {
        let y = "underground-belt";
        let w = build(&scene(serde_json::json!([ug(y, "input", 0.5, 0.5, "east"), ug(y, "output", 2.5, 0.5, "east")])), Prototypes::bundled()).unwrap();
        let (i, o) = (&w.belts[0], &w.belts[1]);
        // вход: центр тайла (128, 128); позади центра на 30 — 30; на «крыше» — 0; задний край — 127
        assert_eq!(i.drop_lane_pos((98, 100)), (0, 30));
        assert_eq!(i.drop_lane_pos((200, 128)), (1, 0));
        assert_eq!(i.drop_lane_pos((0, 200)), (1, 127));
        // выход: центр (640, 128); у переднего края — 1; впереди центра на 28 — 100; «крыша» — 127
        assert_eq!(o.drop_lane_pos((767, 100)), (0, 1));
        assert_eq!(o.drop_lane_pos((668, 100)), (0, 100));
        assert_eq!(o.drop_lane_pos((520, 200)), (1, 127));
        assert_eq!((i.item_pos(0, 0), i.item_pos(1, 127)), ((128, 68), (1, 188)));
        assert_eq!((o.item_pos(0, 0), o.item_pos(1, 127)), ((768, 68), (641, 188)));
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

    /// Рельс 2×2: любой из четырёх тайлов — цель `Train`; вагоны и поезда — отказ; рельс и
    /// сущность в одном тайле — отказ.
    #[test]
    fn rail_targets() {
        let ins = |x: f64, d: &str| serde_json::json!({"name": "inserter", "position": [x, 0.5], "direction": d});
        let rail = serde_json::json!({"name": "straight-rail", "position": [3.0, 1.0], "direction": "east"});
        // выгрузка (2.7, 0.5) — тайл (2, 0) рельса; захват (3.5, 0.5) — тайл (3, 0) рельса
        let w = build(&scene(serde_json::json!([ins(1.5, "west"), ins(4.5, "west"), rail])), Prototypes::bundled()).unwrap();
        assert_eq!((w.inserters[0].pickup, w.inserters[0].drop), (Target::None, Target::Train));
        assert_eq!(w.inserters[1].pickup, Target::Train);
        let wagon = serde_json::json!({"name": "cargo-wagon", "position": [3.0, 1.0]});
        assert!(build(&scene(serde_json::json!([wagon])), Prototypes::bundled()).is_err());
        let chest = serde_json::json!({"name": "wooden-chest", "position": [2.5, 0.5]});
        assert!(build(&scene(serde_json::json!([chest, rail])), Prototypes::bundled()).is_err());
        let off = serde_json::json!({"name": "straight-rail", "position": [3.5, 1.0]});
        assert!(build(&scene(serde_json::json!([off])), Prototypes::bundled()).is_err());
    }

    /// Земля: тайл без сущности — цель `Ground`, тайл с манипулятором — `None`; несколько
    /// предметов в тайле захвата безисточникового манипулятора — не отказ (выбор — в `World::step`).
    #[test]
    fn ground_targets() {
        let a = serde_json::json!({"name": "inserter", "position": [0.5, 0.5], "direction": "west"});
        let b = serde_json::json!({"name": "inserter", "position": [1.5, 1.5], "direction": "north"});
        let c = |dy: f64| serde_json::json!({"name": "inserter", "position": [0.5, 2.5], "direction": "west",
                                              "drop": [1.19921875, dy]});
        let w = build(&scene(serde_json::json!([a, b])), Prototypes::bundled()).unwrap();
        assert_eq!((w.inserters[0].pickup, w.inserters[0].drop), (Target::None, Target::Ground));
        let dead = serde_json::json!({"name": "inserter", "position": [1.5, 0.5], "direction": "north"});
        let w = build(&scene(serde_json::json!([a, dead])), Prototypes::bundled()).unwrap();
        assert_eq!(w.inserters[0].drop, Target::None);
        // второй источник на ту же землю: и рядом (мешает первому), и далеко — сцена строится
        assert!(build(&scene(serde_json::json!([a, b, c(-2.0 + 70.0 / 256.0)])), Prototypes::bundled()).is_ok());
        assert!(build(&scene(serde_json::json!([a, b, c(-2.0 + 71.0 / 256.0)])), Prototypes::bundled()).is_ok());
    }

    /// Кольцо 2×2 по часовой: все четыре ленты — повороты направо (левая полоса внешняя, 295,
    /// правая внутренняя, 106). Одного типа — одна линия на полосу, замкнутая на себя, голова —
    /// построенная первой; разных типов — по линии на тайл, голова есть у каждой.
    #[test]
    fn rings() {
        let ring = |a: &str, b: &str| {
            scene(serde_json::json!([
                {"name": a, "position": [0.5, 0.5], "direction": "east"},
                {"name": b, "position": [1.5, 0.5], "direction": "south"},
                {"name": a, "position": [1.5, 1.5], "direction": "west"},
                {"name": b, "position": [0.5, 1.5], "direction": "north"},
            ]))
        };
        let w = build(&ring("transport-belt", "transport-belt"), Prototypes::bundled()).unwrap();
        assert!(w.belts.iter().all(|b| b.shape == 2 && b.lanes[0].len == 295 && b.lanes[1].len == 106));
        for lane in 0..2 {
            let li = w.belts[0].lanes[lane].line;
            let l = &w.lines[li as usize];
            assert_eq!((l.out, l.inp, l.len, l.tiles[0]), (Some(li), Some(li), [1180, 424][lane], 0));
            // хвост кольца — тайл сразу ниже по течению переднего тайла головы
            assert_eq!(l.tiles[3], 1);
        }
        let w = build(&ring("transport-belt", "fast-transport-belt"), Prototypes::bundled()).unwrap();
        assert_eq!(w.lines.iter().filter(|l| l.alive).count(), 8);
    }

    /// Боковая загрузка: лента слева от приёмника — обе полосы в его левую полосу, ближняя к
    /// переднему краю — в 67, дальняя — в 188; справа — в правую; приёмник прямой.
    #[test]
    fn side_load_links() {
        let w = build(
            &scene(serde_json::json!([
                {"name": "transport-belt", "position": [0.5, 0.5], "direction": "north"},
                {"name": "transport-belt", "position": [-0.5, 0.5], "direction": "east"},
                {"name": "transport-belt", "position": [1.5, 0.5], "direction": "west"},
            ])),
            Prototypes::bundled(),
        )
        .unwrap();
        assert_eq!(w.belts[0].shape, 0);
        for (src, tl, offs) in [(1usize, 0u8, [67, 188]), (2, 1, [188, 67])] {
            for (lane, off) in offs.into_iter().enumerate() {
                let l = &w.lines[w.belts[src].lanes[lane].line as usize];
                let sd = l.side.expect("выход вбок");
                assert_eq!((sd.belt, sd.lane, sd.off), (0, tl, off));
                assert_eq!(l.out, Some(w.belts[0].lanes[tl as usize].line));
            }
            let t = &w.lines[w.belts[0].lanes[tl as usize].line as usize];
            assert_eq!(t.sides, vec![w.belts[src].lanes[0].line, w.belts[src].lanes[1].line]);
        }
        // одна группа на приёмник и его источник
        let g = |b: usize, l: usize| w.lines[w.belts[b].lanes[l].line as usize].group;
        assert!(g(1, 0) == g(0, 0) && g(1, 1) == g(0, 0) && g(2, 0) == g(0, 1) && g(0, 0) != g(0, 1));
    }
}
