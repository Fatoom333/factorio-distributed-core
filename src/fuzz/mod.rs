//! Общий код фаззера M0: генератор случайных маленьких сцен, проверка сцены, разбор дампа.
//! Модуль библиотеки (`factorio_distributed_core::fuzz`), его используют бинари fuzz_gen, fuzz_capture и fuzz_report.
//!
//! Формат сцены — тот же, что у tests/fidelity/*.scene.json, плюс необязательные поля манипулятора
//! "pickup" и "drop": смещения [dx, dy] в тайлах от центра манипулятора (кастомные векторы,
//! работают только с тестовым модом bench/test-mods/fd-test-vectors).

use serde_json::{json, Value};
use std::collections::HashSet;

// ---------- простой ГПСЧ (splitmix64), без зависимостей ----------

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Целое в 0..n (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    /// Целое в lo..=hi.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }
    /// true с вероятностью pct процентов.
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

// ---------- сцена ----------

pub const DIR_NAMES: [&str; 4] = ["north", "east", "south", "west"];
/// Единичные векторы сторон: 0 = север (0,-1), 1 = восток, 2 = юг, 3 = запад.
pub const DIR_VEC: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

pub fn opposite(d: usize) -> usize {
    (d + 2) % 4
}

type Tile = (i32, i32);

#[derive(Clone, Debug)]
pub struct Ent {
    pub name: &'static str,
    /// Тайл (целые координаты); центр сущности — (x + 0.5, y + 0.5).
    pub x: i32,
    pub y: i32,
    pub dir: Option<usize>,
    pub items: Vec<(&'static str, u32)>,
    /// Кастомные векторы манипулятора: смещение от его центра, тайлы.
    pub pickup: Option<(f64, f64)>,
    pub drop: Option<(f64, f64)>,
    /// Вид подземной ленты: "input" / "output" (поле "type" сцены); у остальных — `None`.
    pub ug: Option<&'static str>,
}

impl Ent {
    fn plain(name: &'static str, t: Tile, dir: Option<usize>) -> Ent {
        Ent { name, x: t.0, y: t.1, dir, items: vec![], pickup: None, drop: None, ug: None }
    }
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub name: String,
    pub ticks: u32,
    pub entities: Vec<Ent>,
}

impl Scene {
    pub fn to_json(&self) -> Value {
        let ents: Vec<Value> = self
            .entities
            .iter()
            .map(|e| {
                let mut o = json!({"name": e.name, "position": [e.x as f64 + 0.5, e.y as f64 + 0.5]});
                if let Some(d) = e.dir {
                    o["direction"] = json!(DIR_NAMES[d]);
                }
                if !e.items.is_empty() {
                    let mut m = serde_json::Map::new();
                    for (n, c) in &e.items {
                        m.insert((*n).to_string(), json!(c));
                    }
                    o["items"] = Value::Object(m);
                }
                if let Some((dx, dy)) = e.pickup {
                    o["pickup"] = json!([dx, dy]);
                }
                if let Some((dx, dy)) = e.drop {
                    o["drop"] = json!([dx, dy]);
                }
                if let Some(t) = e.ug {
                    o["type"] = json!(t);
                }
                o
            })
            .collect();
        json!({"name": self.name, "ticks": self.ticks, "entities": ents})
    }
}

pub const BELTS: [&str; 3] = ["transport-belt", "fast-transport-belt", "express-transport-belt"];
pub const INSERTERS: [&str; 3] = ["inserter", "fast-inserter", "long-handed-inserter"];
pub const CHESTS: [(&str, u32); 2] = [("wooden-chest", 16), ("iron-chest", 32)];
/// Ярусы лент для сцен с подземками: (лента, подземная лента, `max_distance` подземки —
/// наибольшее расстояние между центрами входа и выхода в тайлах).
pub const TIERS: [(&str, &str, i32); 4] = [
    ("transport-belt", "underground-belt", 5),
    ("fast-transport-belt", "fast-underground-belt", 7),
    ("express-transport-belt", "express-underground-belt", 9),
    ("turbo-transport-belt", "turbo-underground-belt", 11),
];
/// Предметы и размеры стака (для проверки вместимости сундука).
pub const ITEMS: [(&str, u32); 9] = [
    ("iron-plate", 100),
    ("copper-plate", 100),
    ("coal", 50),
    ("stone", 50),
    ("iron-ore", 50),
    ("iron-gear-wheel", 100),
    ("electronic-circuit", 200),
    ("stone-brick", 100),
    ("copper-cable", 200),
];

/// Предел расстояния кастомного вектора (в тайлах по каждой оси). Проверено в игре, см.
/// docs/mechanics/inserters.md, «Кастомные векторы».
pub const MAX_CUSTOM_REACH: i32 = 3;
pub const MAX_COORD: i32 = 64;
pub const MAX_ENTITIES: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct Opts {
    /// Повороты лент и боковая загрузка (M0.5). По умолчанию выключено.
    pub turns: bool,
    /// Кастомные векторы манипулятора (нужен тестовый мод). По умолчанию включено.
    pub vectors: bool,
    /// Сцены, которые раньше были известными расхождениями модели (с 11.10 модель их считает;
    /// по умолчанию выключены, чтобы прогоны по умолчанию были сравнимы со старыми):
    /// несколько манипуляторов-стоков с одной ленты (конкуренция стоков: сон и порядок хода);
    pub compete: bool,
    /// сток на конце ленты и вообще очередь, которую разбирает сток (перезапуск подпора);
    pub compress: bool,
    /// несколько видов предметов в сундуке-источнике;
    pub multi_item: bool,
    /// манипуляторы быстрый и длиннорукий (с 09.10 модель их знает: числа — из выжимки
    /// data.raw; флаг оставлен, чтобы прогоны по умолчанию были сравнимы со старыми).
    pub all_inserters: bool,
    /// Подземные ленты (M0.5): пары разной длины и ярусов, стыки с лентами, боковые входы во
    /// вход и в выход, манипуляторы у входа и выхода, подземки без пары. Сцены строит отдельная
    /// ветка генератора (`build_scene_ug`); без флага сцены прежние, бит в бит.
    pub underground: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { turns: false, vectors: true, compete: false, compress: false, multi_item: false, all_inserters: false, underground: false }
    }
}

// ---------- проверка ----------

/// Проверка сцены: границы, пересечения (все сущности 1x1, один тайл на сущность),
/// вместимость сундуков.
pub fn check_scene(s: &Scene) -> Result<(), String> {
    if s.entities.is_empty() || s.entities.len() > MAX_ENTITIES {
        return Err("число сущностей".into());
    }
    if !(1..=7200).contains(&s.ticks) {
        return Err("ticks".into());
    }
    let mut seen = HashSet::new();
    for e in &s.entities {
        if e.x.abs() > MAX_COORD || e.y.abs() > MAX_COORD {
            return Err(format!("вне границ: {},{}", e.x, e.y));
        }
        if !seen.insert((e.x, e.y)) {
            return Err(format!("пересечение в {},{}", e.x, e.y));
        }
        if !e.items.is_empty() {
            let slots = CHESTS.iter().find(|c| c.0 == e.name).map(|c| c.1).ok_or("items не у сундука")?;
            let mut used = 0;
            for (n, c) in &e.items {
                let stack = ITEMS.iter().find(|i| i.0 == *n).map(|i| i.1).ok_or("неизвестный предмет")?;
                used += c.div_ceil(stack);
            }
            if used > slots {
                return Err(format!("сундук {} не вместит предметы ({used} > {slots} слотов)", e.name));
            }
        }
        if (e.pickup.is_some() || e.drop.is_some()) && !INSERTERS.contains(&e.name) {
            return Err("pickup/drop не у манипулятора".into());
        }
        let is_ug = TIERS.iter().any(|t| t.1 == e.name);
        if is_ug != e.ug.is_some() || e.ug.is_some_and(|t| t != "input" && t != "output") {
            return Err("type (input/output) — у подземной ленты и только у неё".into());
        }
    }
    ug_facing_conflict(s)
}

/// Две подземки одного прототипа, которые смотрят «навесами» друг на друга в пределах дальности,
/// игра при постройке делает парой: если по сцене у них разное направление движения (два входа
/// или два выхода навстречу), построенная позже получает другой вид и направление, чем записано
/// (проверено в игре: мод на такую сцену отвечает ошибкой «встала не так»). Такие сцены
/// генератор не выдаёт.
fn ug_facing_conflict(s: &Scene) -> Result<(), String> {
    // навес смотрит по ходу у входа и против хода у выхода
    let hood = |e: &Ent| e.dir.map(|d| if e.ug == Some("input") { d } else { opposite(d) });
    for a in s.entities.iter().filter(|e| e.ug.is_some()) {
        let (Some(h), Some(tier)) = (hood(a), TIERS.iter().find(|t| t.1 == a.name)) else { continue };
        for k in 1..=tier.2 {
            let t = step((a.x, a.y), h, k);
            let facing = s.entities.iter().find(|b| (b.x, b.y) == t && b.name == a.name && hood(b) == Some(opposite(h)));
            if facing.is_some_and(|b| b.dir != a.dir) {
                return Err(format!("подземки {},{} и {},{} навстречу с разным направлением движения", a.x, a.y, t.0, t.1));
            }
        }
    }
    Ok(())
}

// ---------- генератор ----------

struct Builder<'a> {
    rng: &'a mut Rng,
    ents: Vec<Ent>,
    occ: HashSet<Tile>,
    opts: Opts,
    /// Сумма предметов во всех сундуках-источниках.
    s_total: u32,
    /// Стоки можно ставить где угодно (очередь доходит до стока): `opts.compress`, а в сценах с
    /// подземками — ещё и по жребию на сцену.
    compress: bool,
}

fn step(v: Tile, d: usize, k: i32) -> Tile {
    (v.0 + DIR_VEC[d].0 * k, v.1 + DIR_VEC[d].1 * k)
}

type Chain = Vec<(Tile, usize)>;

impl<'a> Builder<'a> {
    fn free(&self, t: Tile) -> bool {
        !self.occ.contains(&t) && t.0.abs() <= 40 && t.1.abs() <= 40
    }

    fn place(&mut self, e: Ent) -> bool {
        if !self.free((e.x, e.y)) {
            return false;
        }
        self.occ.insert((e.x, e.y));
        self.ents.push(e);
        true
    }

    fn belt_len(&mut self) -> i32 {
        if self.rng.chance(25) {
            self.rng.range(1, 3)
        } else {
            self.rng.range(4, 20)
        }
    }

    /// Цепочка лент. Возвращает (тайл, направление) в порядке движения.
    fn build_chain(&mut self) -> Option<Chain> {
        let d = self.rng.below(4) as usize;
        let start = (self.rng.range(-4, 4), self.rng.range(-4, 4));
        let two = self.rng.chance(25);
        let (l1, l2) = if two {
            let l1 = self.rng.range(1, 12);
            (l1, self.rng.range(1, (20 - l1).min(12)))
        } else {
            (self.belt_len(), 0)
        };
        let t1 = self.rng.pick(&BELTS);
        let t2 = if self.rng.chance(70) { self.rng.pick(&BELTS) } else { t1 };
        let mut parts = Vec::new();
        let mut cur = start;
        for _ in 0..l1 {
            parts.push((cur, d, t1));
            cur = step(cur, d, 1);
        }
        if two {
            let mut d2 = d;
            if self.opts.turns {
                // поворот: вторая лента смотрит вбок, её первая клетка — угол
                d2 = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
            }
            for _ in 0..l2 {
                parts.push((cur, d2, t2));
                cur = step(cur, d2, 1);
            }
        }
        let mut out = Vec::new();
        for (t, dd, name) in parts {
            if !self.place(Ent::plain(name, t, Some(dd))) {
                return None;
            }
            out.push((t, dd));
        }
        Some(out)
    }

    fn make_items(&mut self, slots: u32) -> Vec<(&'static str, u32)> {
        let kinds = if self.opts.multi_item && self.rng.chance(20) { 2 } else { 1 };
        let mut left = slots;
        let mut items: Vec<(&'static str, u32)> = Vec::new();
        for _ in 0..kinds {
            let (name, stack) = self.rng.pick(&ITEMS);
            if items.iter().any(|i| i.0 == name) {
                continue;
            }
            let cap = left * stack;
            if cap == 0 {
                break;
            }
            let want = match self.rng.below(10) {
                0 => self.rng.range(1, 5) as u32,
                1..=3 => self.rng.range(6, 60) as u32,
                _ => self.rng.range(61, 1500) as u32,
            };
            let count = want.min(cap).max(1);
            left -= count.div_ceil(stack);
            self.s_total += count;
            items.push((name, count));
            if left == 0 {
                break;
            }
        }
        items
    }

    /// Манипулятор рядом с лентой: tile — клетка ленты, sdir — сторона от ленты к сундуку.
    /// Источник: сундук -> лента (направление манипулятора = сторона сундука). Сток: лента -> сундук.
    fn add_feed(&mut self, tile: Tile, sdir: usize, is_source: bool) -> bool {
        let ins = self.pick_inserter();
        let r = if ins == "long-handed-inserter" { 2 } else { 1 };
        let ip = step(tile, sdir, r);
        let cp = step(tile, sdir, 2 * r);
        if !self.free(ip) || !self.free(cp) {
            return false;
        }
        let (chest, slots) = self.rng.pick(&CHESTS);
        let items = if is_source { self.make_items(slots) } else { vec![] };
        let idir = if is_source { sdir } else { opposite(sdir) };
        self.place(Ent::plain(ins, ip, Some(idir)))
            && self.place(Ent { items, ..Ent::plain(chest, cp, None) })
    }

    fn pick_inserter(&mut self) -> &'static str {
        if self.opts.all_inserters {
            self.rng.pick(&INSERTERS)
        } else {
            "inserter"
        }
    }

    /// Можно ли ставить сток, берущий с клетки номер i цепочки длиной len. Без флага compress:
    /// не на последней клетке и так, чтобы очередь из всех предметов (до 4 на клетку одной
    /// полосы) не доросла до стока.
    fn sink_ok(&self, i: usize, len: usize) -> bool {
        self.compress || (i + 1 < len && ((len - 1 - i) as u32) > self.s_total / 4 + 1)
    }

    fn jitter(&mut self) -> f64 {
        if self.rng.chance(30) {
            0.0
        } else {
            self.rng.range(-80, 80) as f64 / 256.0
        }
    }

    fn offset(&mut self) -> Tile {
        loop {
            let o = (self.rng.range(-MAX_CUSTOM_REACH, MAX_CUSTOM_REACH), self.rng.range(-MAX_CUSTOM_REACH, MAX_CUSTOM_REACH));
            if o != (0, 0) {
                return o;
            }
        }
    }

    /// Манипулятор с кастомными векторами: берёт и кладёт на произвольных клетках рядом.
    fn add_custom(&mut self, chain: &Chain, is_source: bool) -> bool {
        let bi = self.rng.below(chain.len() as u64) as usize;
        if !is_source && !self.sink_ok(bi, chain.len()) {
            return false;
        }
        let b = chain[bi].0;
        let o1 = self.offset(); // манипулятор -> клетка ленты (с обратным знаком)
        let p = (b.0 + o1.0, b.1 + o1.1);
        let o2 = self.offset(); // манипулятор -> сундук
        let c = (p.0 + o2.0, p.1 + o2.1);
        if c == b || !self.free(p) || !self.free(c) {
            return false;
        }
        let ins = self.pick_inserter();
        let (chest, slots) = self.rng.pick(&CHESTS);
        let items = if is_source { self.make_items(slots) } else { vec![] };
        let vb = (-o1.0 as f64 + self.jitter(), -o1.1 as f64 + self.jitter());
        let vc = (o2.0 as f64 + self.jitter(), o2.1 as f64 + self.jitter());
        let (pickup, drop) = if is_source { (vc, vb) } else { (vb, vc) };
        let dir = self.rng.below(4) as usize;
        self.place(Ent { pickup: Some(pickup), drop: Some(drop), ..Ent::plain(ins, p, Some(dir)) })
            && self.place(Ent { items, ..Ent::plain(chest, c, None) })
    }

    /// Один источник или сток: сбоку (60%) или «в лоб» с края ленты.
    fn add_one(&mut self, chain: &Chain, is_source: bool) -> bool {
        for _ in 0..8 {
            if self.opts.vectors && self.rng.chance(15) {
                if self.add_custom(chain, is_source) {
                    return true;
                }
                continue;
            }
            let ok = if self.rng.chance(60) {
                let ti = self.rng.below(chain.len() as u64) as usize;
                if !is_source && !self.sink_ok(ti, chain.len()) {
                    continue;
                }
                let (t, d) = chain[ti];
                let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
                self.add_feed(t, side, is_source)
            } else if self.rng.chance(50) {
                // с тыла первой ленты
                let (t, d) = chain[0];
                if !is_source && !self.sink_ok(0, chain.len()) {
                    continue;
                }
                self.add_feed(t, opposite(d), is_source)
            } else {
                // с головы последней ленты
                let (t, d) = chain[chain.len() - 1];
                if !is_source && !self.sink_ok(chain.len() - 1, chain.len()) {
                    continue;
                }
                self.add_feed(t, d, is_source)
            };
            if ok {
                return true;
            }
        }
        false
    }

    /// Боковая загрузка (только с opts.turns): короткая лента, впадающая сбоку в основную.
    fn add_feeder(&mut self, chain: &Chain) -> bool {
        if chain.len() < 3 {
            return false;
        }
        let i = self.rng.range(1, chain.len() as i32 - 1) as usize;
        let (t, d) = chain[i];
        if chain[i - 1].1 != d {
            return false;
        }
        let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
        let len = self.rng.range(1, 4);
        let name = self.rng.pick(&BELTS);
        let mut fchain = Vec::new();
        for k in (1..=len).rev() {
            let ft = step(t, side, k);
            if !self.place(Ent::plain(name, ft, Some(opposite(side)))) {
                return false;
            }
            fchain.push((ft, opposite(side)));
        }
        self.add_one(&fchain, true)
    }
}

/// Что стоит на клетке цепочки с подземками.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Belt,
    In,
    Out,
}

impl<'a> Builder<'a> {
    fn place_ug(&mut self, tier: usize, t: Tile, d: usize, kind: &'static str) -> bool {
        self.place(Ent { ug: Some(kind), ..Ent::plain(TIERS[tier].1, t, Some(d)) })
    }

    /// Расстояние между входом и выходом пары яруса `tier`: вплотную, на пределе, за пределом
    /// (пара не образуется) или посередине.
    fn ug_distance(&mut self, tier: usize) -> i32 {
        let max = TIERS[tier].2;
        match self.rng.below(20) {
            0..=2 => 1,
            3..=5 => max,
            6 => max + 1,
            _ => self.rng.range(2, max - 1),
        }
    }

    /// Помеха в промежутке пары (тайлы `gap`): лента поперёк, подземка другого яруса или
    /// подземка того же яруса (она меняет поиск пары).
    fn add_gap_decoy(&mut self, tier: usize, gap: &[Tile], d: usize) {
        if gap.is_empty() {
            return;
        }
        let t = gap[self.rng.below(gap.len() as u64) as usize];
        let kind = if self.rng.chance(50) { "input" } else { "output" };
        match self.rng.below(4) {
            0 => {
                let name = TIERS[self.rng.below(4) as usize].0;
                let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
                self.place(Ent::plain(name, t, Some(side)));
            }
            1 | 2 => {
                let other = (tier + 1 + self.rng.below(3) as usize) % 4;
                let dd = if self.rng.chance(70) { d } else { opposite(d) };
                self.place_ug(other, t, dd, kind);
            }
            _ => {
                let dd = match self.rng.below(4) {
                    0 | 1 => d,
                    2 => opposite(d),
                    _ => (d + 1) % 4,
                };
                self.place_ug(tier, t, dd, kind);
            }
        }
    }

    /// Цепочка с подземками: отрезки лент, пары вход/выход и одиночные подземки вдоль одной
    /// прямой (с `opts.turns` — с поворотами между отрезками). Возвращает надземные клетки в
    /// порядке движения и что на каждой стоит; хотя бы одна пара в цепочке есть.
    fn build_chain_ug(&mut self) -> Option<(Chain, Vec<Cell>)> {
        let mut d = self.rng.below(4) as usize;
        let mut cur = (self.rng.range(-4, 4), self.rng.range(-4, 4));
        let mut tier = self.rng.below(4) as usize;
        let mixed = self.rng.chance(35);
        let elems = self.rng.range(2, 6);
        let mut chain: Chain = Vec::new();
        let mut cells: Vec<Cell> = Vec::new();
        let mut pairs = 0;
        let mut span = 0;
        for k in 0..elems {
            if span > 26 {
                break;
            }
            if mixed && self.rng.chance(50) {
                tier = self.rng.below(4) as usize;
            }
            // первый элемент чаще лента (чтобы было куда класть), дальше чаще пара
            let roll = self.rng.below(10);
            let belt_run = if k == 0 { roll < 6 } else { roll < 3 };
            if belt_run {
                if self.opts.turns && k > 0 && self.rng.chance(25) {
                    d = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
                }
                for _ in 0..self.rng.range(1, 4) {
                    if !self.place(Ent::plain(TIERS[tier].0, cur, Some(d))) {
                        return None;
                    }
                    chain.push((cur, d));
                    cells.push(Cell::Belt);
                    cur = step(cur, d, 1);
                    span += 1;
                }
            } else if roll == 9 {
                // одиночная подземка: вход (тупик) или выход (начало с половины тайла)
                let kind = if self.rng.chance(50) { "input" } else { "output" };
                if !self.place_ug(tier, cur, d, kind) {
                    return None;
                }
                chain.push((cur, d));
                cells.push(if kind == "input" { Cell::In } else { Cell::Out });
                cur = step(cur, d, 1);
                span += 1;
            } else {
                let ut = if self.rng.chance(20) { self.rng.below(4) as usize } else { tier };
                let dist = self.ug_distance(ut);
                let out = step(cur, d, dist);
                if !self.place_ug(ut, cur, d, "input") || !self.place_ug(ut, out, d, "output") {
                    return None;
                }
                chain.push((cur, d));
                cells.push(Cell::In);
                chain.push((out, d));
                cells.push(Cell::Out);
                if dist <= TIERS[ut].2 {
                    pairs += 1;
                }
                if self.rng.chance(25) {
                    let gap: Vec<Tile> = (1..dist).map(|j| step(cur, d, j)).collect();
                    self.add_gap_decoy(ut, &gap, d);
                }
                cur = step(out, d, 1);
                span += dist + 1;
            }
        }
        if pairs == 0 {
            return None;
        }
        // приёмник поперёк за последней клеткой: цепочка впадает в него сбоку
        if self.rng.chance(20) {
            let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
            let name = TIERS[self.rng.below(4) as usize].0;
            for j in -1..=self.rng.range(0, 2) {
                self.place(Ent::plain(name, step(cur, side, j), Some(side)));
            }
        }
        Some((chain, cells))
    }

    /// Источник у начала цепочки (первые две клетки), чтобы предметы прошли её всю.
    fn add_source_at_start(&mut self, chain: &Chain) -> bool {
        for _ in 0..8 {
            let i = self.rng.below(chain.len().min(2) as u64) as usize;
            let (t, d) = chain[i];
            let side = match self.rng.below(3) {
                0 => (d + 1) % 4,
                1 => (d + 3) % 4,
                _ => opposite(d),
            };
            if i > 0 && side == opposite(d) {
                continue;
            }
            if self.add_feed(t, side, true) {
                return true;
            }
        }
        false
    }

    /// Источник или сток прямо у клетки подземки (вход или выход), сбоку.
    fn add_at_ug(&mut self, chain: &Chain, cells: &[Cell], is_source: bool) -> bool {
        let ugs: Vec<usize> = (0..chain.len()).filter(|&i| cells[i] != Cell::Belt).collect();
        for _ in 0..6 {
            let i = ugs[self.rng.below(ugs.len() as u64) as usize];
            if !is_source && !self.sink_ok(i, chain.len()) {
                continue;
            }
            let (t, d) = chain[i];
            let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
            if self.add_feed(t, side, is_source) {
                return true;
            }
        }
        false
    }

    /// Боковой вход в клетку подземки: короткая лента (иногда — выход другой подземки), впадающая
    /// сбоку во вход или в выход, со своим источником.
    fn add_ug_feeder(&mut self, chain: &Chain, cells: &[Cell]) -> bool {
        let ugs: Vec<usize> = (0..chain.len()).filter(|&i| cells[i] != Cell::Belt).collect();
        let i = ugs[self.rng.below(ugs.len() as u64) as usize];
        let (t, d) = chain[i];
        let side = if self.rng.chance(50) { (d + 1) % 4 } else { (d + 3) % 4 };
        let fd = opposite(side);
        let len = self.rng.range(1, 3);
        let tier = self.rng.below(4) as usize;
        let mut fchain = Vec::new();
        if self.rng.chance(20) {
            // пара подземок, выход которой смотрит в бок клетки
            let dist = self.rng.range(1, TIERS[tier].2);
            if !self.place_ug(tier, step(t, side, 1 + dist), fd, "input") || !self.place_ug(tier, step(t, side, 1), fd, "output") {
                return false;
            }
            fchain.push((step(t, side, 1 + dist), fd));
        } else {
            for k in (1..=len).rev() {
                let ft = step(t, side, k);
                if !self.place(Ent::plain(TIERS[tier].0, ft, Some(fd))) {
                    return false;
                }
                fchain.push((ft, fd));
            }
        }
        self.add_one(&fchain, true)
    }
}

/// Сцена с подземными лентами (`opts.underground`).
fn build_scene_ug(rng: &mut Rng, opts: Opts, name: String) -> Option<Scene> {
    let mut b = Builder { rng, ents: Vec::new(), occ: HashSet::new(), opts, s_total: 0, compress: opts.compress };
    let (chain, cells) = b.build_chain_ug()?;
    if !b.opts.compress {
        b.compress = b.rng.chance(50);
    }
    if !b.add_source_at_start(&chain) {
        return None;
    }
    if b.rng.chance(40) {
        if b.rng.chance(50) {
            b.add_at_ug(&chain, &cells, true);
        } else {
            b.add_one(&chain, true);
        }
    }
    let mut nk = match b.rng.below(4) {
        0 => 0,
        1 | 2 => 1,
        _ => 2,
    };
    if !opts.compete {
        nk = nk.min(1);
    }
    for _ in 0..nk {
        if b.rng.chance(40) {
            b.add_at_ug(&chain, &cells, false);
        } else {
            b.add_one(&chain, false);
        }
    }
    match b.rng.below(10) {
        0..=3 => {
            b.add_ug_feeder(&chain, &cells);
        }
        4 | 5 => {
            b.add_feeder(&chain);
        }
        _ => {}
    }
    let ticks = 600 + b.rng.below(601) as u32;
    let scene = Scene { name, ticks, entities: b.ents };
    check_scene(&scene).ok()?;
    Some(scene)
}

fn build_scene(rng: &mut Rng, opts: Opts, name: String) -> Option<Scene> {
    if opts.underground {
        return build_scene_ug(rng, opts, name);
    }
    let mut b = Builder { rng, ents: Vec::new(), occ: HashSet::new(), opts, s_total: 0, compress: opts.compress };
    let chain = b.build_chain()?;
    let ns = if b.rng.chance(30) { 2 } else { 1 };
    for k in 0..ns {
        if !b.add_one(&chain, true) && k == 0 {
            return None;
        }
    }
    let mut nk = match b.rng.below(4) {
        0 => 0,
        1 | 2 => 1,
        _ => 2,
    };
    if !opts.compete {
        nk = nk.min(1);
    }
    for _ in 0..nk {
        b.add_one(&chain, false);
    }
    if opts.turns && b.rng.chance(40) {
        b.add_feeder(&chain);
    }
    let ticks = 600 + b.rng.below(601) as u32;
    let scene = Scene { name, ticks, entities: b.ents };
    check_scene(&scene).ok()?;
    Some(scene)
}

pub fn scene_name(seed: u64, idx: u32) -> String {
    format!("s{seed}-{idx:05}")
}

/// Сцена номер idx прогона seed. Детерминирована: зависит только от (seed, idx, opts).
pub fn generate(seed: u64, idx: u32, opts: Opts) -> Scene {
    for attempt in 0..1000u64 {
        let mut rng = Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ ((idx as u64) << 20) ^ attempt.wrapping_mul(0x9E37));
        // прогрев: близкие зерна дают похожие первые значения
        rng.next_u64();
        rng.next_u64();
        if let Some(s) = build_scene(&mut rng, opts, scene_name(seed, idx)) {
            return s;
        }
    }
    panic!("генератор не смог собрать сцену {idx} за 1000 попыток");
}

/// Эталон: по одной строке JSON на тик.
pub fn read_dump(path: &std::path::Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}

/// Простейший разбор аргументов: `--ключ значение` и `--флаг`.
pub struct Args(std::collections::HashMap<String, String>);

impl Args {
    pub fn parse() -> Args {
        let v: Vec<String> = std::env::args().skip(1).collect();
        let mut m = std::collections::HashMap::new();
        let mut i = 0;
        while i < v.len() {
            if let Some(k) = v[i].strip_prefix("--") {
                if i + 1 < v.len() && !v[i + 1].starts_with("--") {
                    m.insert(k.to_string(), v[i + 1].clone());
                    i += 1;
                } else {
                    m.insert(k.to_string(), String::new());
                }
            }
            i += 1;
        }
        Args(m)
    }
    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(String::as_str)
    }
    pub fn flag(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }
    pub fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> T {
        self.get(k).and_then(|s| s.parse().ok()).unwrap_or(default)
    }
    pub fn opts(&self) -> Opts {
        let bad = self.flag("known-bad");
        Opts {
            turns: self.flag("turns"),
            vectors: !self.flag("no-vectors"),
            compete: bad || self.flag("compete"),
            compress: bad || self.flag("compress"),
            multi_item: bad || self.flag("multi-item"),
            all_inserters: self.flag("all-inserters"),
            underground: self.flag("underground"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::proto::Prototypes;

    /// Таблицы генератора (вместимость сундуков, stack_size) — те же числа, что в выжимке
    /// data.raw, которой пользуется модель; типы сущностей генератора модель знает.
    #[test]
    fn generator_tables_match_prototypes() {
        let p = Prototypes::bundled();
        for (name, ss) in ITEMS {
            assert_eq!(p.stack_size.get(name), Some(&ss), "{name}");
        }
        for (name, slots) in CHESTS {
            assert_eq!(p.chests.get(name).map(|c| c.slots), Some(slots), "{name}");
        }
        for name in INSERTERS {
            assert!(p.inserters.contains_key(name), "{name}");
        }
        for name in BELTS {
            assert!(p.belts.contains_key(name), "{name}");
        }
        for (belt, ug, max) in TIERS {
            assert_eq!(p.undergrounds.get(ug).map(|u| u.max_distance as i32), Some(max), "{ug}");
            assert_eq!(p.belts.get(belt).map(|b| b.speed), p.undergrounds.get(ug).map(|u| u.speed), "{belt}");
        }
    }

    /// Сцены с подземками: детерминированы, проходят проверку, в каждой есть пара вход/выход
    /// в пределах дальности; модель строит мир из каждой и проходит 50 тиков без паники.
    #[test]
    fn underground_scenes() {
        let opts = Opts { underground: true, ..Opts::default() };
        for idx in 0..200 {
            let s = generate(77, idx, opts);
            check_scene(&s).unwrap();
            assert_eq!(s.to_json(), generate(77, idx, opts).to_json());
            let paired = s.entities.iter().filter(|e| e.ug == Some("input")).any(|i| {
                let d = DIR_VEC[i.dir.unwrap()];
                let max = TIERS.iter().find(|t| t.1 == i.name).unwrap().2;
                s.entities.iter().any(|o| {
                    o.ug == Some("output")
                        && o.name == i.name
                        && o.dir == i.dir
                        && (1..=max).any(|k| (o.x, o.y) == (i.x + d.0 * k, i.y + d.1 * k))
                })
            });
            assert!(paired, "{}", s.name);
            let sc: crate::world::layout::Scene = serde_json::from_value(s.to_json()).unwrap();
            let mut w = crate::world::World::from_scene(&sc, Prototypes::bundled()).unwrap_or_else(|e| panic!("{}: {e}", s.name));
            for _ in 0..50 {
                w.step();
            }
        }
    }
}
