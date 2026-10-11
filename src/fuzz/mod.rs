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
}

impl Ent {
    fn plain(name: &'static str, t: Tile, dir: Option<usize>) -> Ent {
        Ent { name, x: t.0, y: t.1, dir, items: vec![], pickup: None, drop: None }
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
                o
            })
            .collect();
        json!({"name": self.name, "ticks": self.ticks, "entities": ents})
    }
}

pub const BELTS: [&str; 3] = ["transport-belt", "fast-transport-belt", "express-transport-belt"];
pub const INSERTERS: [&str; 3] = ["inserter", "fast-inserter", "long-handed-inserter"];
pub const CHESTS: [(&str, u32); 2] = [("wooden-chest", 16), ("iron-chest", 32)];
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
}

impl Default for Opts {
    fn default() -> Self {
        Opts { turns: false, vectors: true, compete: false, compress: false, multi_item: false, all_inserters: false }
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
        self.opts.compress || (i + 1 < len && ((len - 1 - i) as u32) > self.s_total / 4 + 1)
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

fn build_scene(rng: &mut Rng, opts: Opts, name: String) -> Option<Scene> {
    let mut b = Builder { rng, ents: Vec::new(), occ: HashSet::new(), opts, s_total: 0 };
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
    }
}
