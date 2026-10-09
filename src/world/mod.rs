//! Модель мира M0: ленты, сундуки, манипуляторы. Совпадает с движком Factorio потиково
//! (сверка — tests/fidelity.rs, программа `fidelity`, docs/model-fidelity.md).
//!
//! Состояние — плоские данные: `Vec` и индексы, без указателей; `World: Clone`, снимок —
//! это `clone()`. В состоянии нет `f32`/`f64`: позиции в 1/256 тайла, углы — `u64`
//! (доля оборота), вылет руки — `double`, высота руки — `float` в целочисленной эмуляции
//! (`softfloat::Sf`: знак, мантисса, порядок — целые).
//!
//! Порядок внутри тика (проверено по эталону, см. docs/mechanics/*.md):
//! 1. ленты сдвигают предметы;
//! 2. манипуляторы в порядке `ins_order` (последний разбуженный — первым): выбирают цель,
//!    двигают руку, берут/кладут.
//!    Манипулятор видит предметы уже после сдвига лент этого тика.

pub mod belt;
pub mod fixed;
pub mod inserter;
pub mod inventory;
pub mod layout;
pub mod proto;
pub mod softfloat;

use std::collections::BTreeMap;

use serde::Serialize;

use belt::{BeltLine, ItemId};
use fixed::{hypot_f64, orientation_f32, Fx, TILE};
use inserter::{HandTarget, Inserter, Target};
use inventory::Chest;
use layout::{DumpBelt, DumpChest, DumpInserter, DumpItem, LayoutError, Pos, Scene, TickDump};
use proto::{LANE_OFFSET, TILE_LANE_LEN};

/// Направление сущности.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    /// Единичный вектор (ось y карты — вниз).
    pub fn unit(self) -> (i32, i32) {
        match self {
            Dir::North => (0, -1),
            Dir::East => (1, 0),
            Dir::South => (0, 1),
            Dir::West => (-1, 0),
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::North => Dir::South,
            Dir::East => Dir::West,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
        }
    }

    /// Повернуть вектор, заданный для «севера», в это направление (по часовой).
    pub fn rotate(self, v: (i32, i32)) -> (i32, i32) {
        let (x, y) = v;
        match self {
            Dir::North => (x, y),
            Dir::East => (-y, x),
            Dir::South => (-x, -y),
            Dir::West => (y, -x),
        }
    }

    /// Вектор «вправо» от направления движения.
    pub fn right(self) -> (i32, i32) {
        let (x, y) = self.unit();
        (-y, x)
    }
}

/// Тайл ленты.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct BeltTile {
    /// Центр, 1/256 тайла.
    pub pos: (Fx, Fx),
    pub dir: Dir,
    /// Индекс в `World::belt_protos`.
    pub proto: u8,
    /// Линия и номер тайла в ней от головы (0 — последний по ходу).
    pub line: u32,
    pub k: u32,
}

impl BeltTile {
    /// Мировая позиция предмета на полосе `lane` с `p` (до края тайла по ходу).
    pub fn item_pos(&self, lane: usize, p: Fx) -> (Fx, Fx) {
        let (ux, uy) = self.dir.unit();
        let (rx, ry) = self.dir.right();
        let along = TILE / 2 - p;
        let side = if lane == 0 { -LANE_OFFSET } else { LANE_OFFSET };
        (self.pos.0 + ux * along + rx * side, self.pos.1 + uy * along + ry * side)
    }

    /// Куда попадает точка выгрузки `dp` на этом тайле: полоса и расстояние до конца линии.
    ///
    /// Полоса — по стороне точки от оси ленты; на оси — правая (проверено по эталону: кладка
    /// «в лоб» — полоса 1). Расстояние — `256·k + 128 − вдоль`, где `вдоль` — смещение точки от
    /// центра тайла по ходу ленты.
    pub fn drop_lane_pos(&self, dp: (Fx, Fx)) -> (usize, Fx) {
        let rel = (dp.0 - self.pos.0, dp.1 - self.pos.1);
        let (ux, uy) = self.dir.unit();
        let (rx, ry) = self.dir.right();
        let along = rel.0 * ux + rel.1 * uy;
        let lateral = rel.0 * rx + rel.1 * ry;
        let lane = if lateral < 0 { 0 } else { 1 };
        (lane, self.k as Fx * TILE_LANE_LEN + TILE / 2 - along)
    }
}

/// Мир M0.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct World {
    pub tick: u64,
    /// Следующий `uid` предмета на ленте (`u64`, без переполнения — см. `BeltItem::uid`).
    pub next_uid: u64,
    /// Предметы с `uid >= fresh_uid` положены в этом тике — их пока не берут.
    pub fresh_uid: u64,
    pub item_names: Vec<String>,
    pub item_stack: Vec<u32>,
    /// Прототипы лент и манипуляторов, которые есть в сцене (`BeltTile::proto`,
    /// `Inserter::proto` — индексы сюда). Числа — из выжимки `data.raw`, см. `proto`.
    pub belt_protos: Vec<proto::BeltProto>,
    pub ins_protos: Vec<proto::InserterProto>,
    pub belts: Vec<BeltTile>,
    pub lines: Vec<BeltLine>,
    /// Порядок обновления линий: ниже по течению — раньше (у кого `next` — после своего
    /// `next`). Граф линий статичен и ацикличен (проверено при сборке), порядок считается
    /// один раз в `layout::build`.
    pub line_order: Vec<u32>,
    pub inserters: Vec<Inserter>,
    /// Порядок обновления манипуляторов (индексы в `inserters`), см. `layout::inserter_order`.
    pub ins_order: Vec<u32>,
    pub chests: Vec<Chest>,
}

impl World {
    /// Мир из сцены; числа сущностей — из `protos` (обычно [`proto::Prototypes::bundled`]).
    pub fn from_scene(scene: &Scene, protos: &proto::Prototypes) -> Result<World, LayoutError> {
        layout::build(scene, protos)
    }

    /// Один тик.
    pub fn step(&mut self) {
        self.tick += 1;
        self.step_belts();
        // Положенное манипулятором в этом тике другой манипулятор возьмёт только со
        // следующего тика (проверено по эталону: пробные сцены «источник и сток на одной
        // ленте» и «цепочка сундуков» в обоих порядках сцены).
        self.fresh_uid = self.next_uid;
        for c in self.chests.iter_mut() {
            c.fresh.clear();
        }
        // Порядок обновления манипуляторов — `ins_order`: последний разбуженный ходит первым
        // (проверено по эталону: пробы t1*, c1*, c3*; правило и границы — в
        // `layout::inserter_order` и docs/mechanics/inserters.md). Пробуждение во время прогона
        // (манипулятор, берущий с ленты, просыпается, когда на тайл приходит предмет) не
        // смоделировано: конкуренция стоков за один предмет (c2*, c4) — вне M0.
        for j in 0..self.ins_order.len() {
            let i = self.ins_order[j] as usize;
            self.step_inserter(i);
        }
    }

    /// Ближе всего к концу линии `li`, куда может встать вставленный предмет (>= 0):
    /// 0, если впереди нет линии или её хвост не мешает, иначе — сколько нужно
    /// отступить от конца, чтобы держать шаг до последнего предмета следующей линии.
    fn end_limit(&self, li: usize, lane: usize) -> Fx {
        let Some(n) = self.lines[li].next else {
            return 0;
        };
        let nl = &self.lines[n as usize];
        match nl.lanes[lane].items.last() {
            Some(last) => (last.pos + proto::ITEM_SPACING - nl.len()).max(0),
            None => 0,
        }
    }

    fn step_belts(&mut self) {
        for k in 0..self.line_order.len() {
            let li = self.line_order[k] as usize;
            let speed = self.lines[li].speed;
            match self.lines[li].next {
                None => {
                    for lane in 0..2 {
                        self.lines[li].lanes[lane].advance(speed, 0);
                    }
                }
                Some(nx) => {
                    let nx = nx as usize;
                    let (nspeed, nlen) = (self.lines[nx].speed, self.lines[nx].len());
                    for lane in 0..2 {
                        let mut next = std::mem::take(&mut self.lines[nx].lanes[lane]);
                        belt::advance_into(&mut self.lines[li].lanes[lane], speed, &mut next, nspeed, nlen);
                        self.lines[nx].lanes[lane] = next;
                    }
                }
            }
        }
    }

    /// Может ли цель выгрузки принять предмет (для решения «брать ли»).
    fn drop_accepts(&self, ins: &Inserter, item: ItemId) -> bool {
        match ins.drop {
            Target::Chest(c) => {
                self.chests[c as usize].inv.room_for(item, self.item_stack[item as usize]) >= 1
            }
            // На ленту манипулятор берёт всегда и ждёт у ленты (вики; эталон).
            Target::Belt(_) => true,
            // Выгрузка на землю — не M0: такой манипулятор не берёт ничего.
            Target::None => false,
        }
    }

    /// Лежит ли предмет `uid` на тайле ленты `b`.
    fn chase_on_tile(&self, b: usize, uid: u64) -> bool {
        let tile = &self.belts[b];
        let line = &self.lines[tile.line as usize];
        let lo = tile.k as Fx * TILE_LANE_LEN;
        line.lanes.iter().any(|l| l.range(lo, lo + TILE_LANE_LEN).any(|(_, it)| it.uid == uid))
    }

    /// Предмет на ленте, за которым поедет рука: (полоса, индекс в полосе, цель руки).
    pub fn belt_pick_candidate(&self, ins: &Inserter, b: usize) -> Option<(usize, usize, HandTarget)> {
        let tile = &self.belts[b];
        let line = &self.lines[tile.line as usize];
        let lo = tile.k as Fx * TILE_LANE_LEN;
        let hi = lo + TILE_LANE_LEN;
        // Порядок полос: ближняя к манипулятору; при равенстве — левая (полоса 0).
        let mut lanes = [0usize, 1usize];
        let dist2 = |lane: usize| -> i64 {
            let c = tile.item_pos(lane, TILE / 2);
            let dx = (c.0 - ins.pos.0) as i64;
            let dy = (c.1 - ins.pos.1) as i64;
            dx * dx + dy * dy
        };
        if dist2(1) < dist2(0) {
            lanes = [1, 0];
        }
        let target_of = |lane: usize, it: &belt::BeltItem| -> HandTarget {
            let wp = tile.item_pos(lane, it.pos - lo);
            let dx = (wp.0 - ins.pos.0) as i64;
            let dy = (wp.1 - ins.pos.1) as i64;
            HandTarget { o: orientation_f32(dx, dy), ext: hypot_f64(dx, dy) }
        };
        // Уже выбранный предмет, если он ещё на тайле и цель выгрузки его примет (то же
        // условие, что при выборе нового предмета ниже). Как игра ведёт себя, если приёмник
        // заполнился во время погони, не сверено: пробы c4-chase-full / c4b-chase-full не
        // создают такой ситуации в игре (там рука всегда обновляется после заполнившего).
        if let Some(uid) = ins.chase {
            for lane in lanes {
                if let Some((idx, it)) = line.lanes[lane].range(lo, hi).find(|(_, it)| it.uid == uid) {
                    if self.drop_accepts(ins, it.item) {
                        return Some((lane, idx, target_of(lane, it)));
                    }
                }
            }
        }
        for lane in lanes {
            // В полосе — предмет, дальше всех от конца линии (последний вошедший на тайл);
            // проверено по эталону two-lanes, тик 553: из 4 упёршихся берёт p = 192.
            for (idx, it) in line.lanes[lane].range(lo, hi).rev() {
                if it.uid >= self.fresh_uid || !self.drop_accepts(ins, it.item) {
                    continue;
                }
                return Some((lane, idx, target_of(lane, it)));
            }
        }
        None
    }

    fn step_inserter(&mut self, i: usize) {
        let ins = &self.inserters[i];
        let (rot_speed, ext_speed) = {
            let p = &self.ins_protos[ins.proto as usize];
            (p.rotation_speed, p.extension_speed)
        };
        // 1. цель
        let mut belt_pick: Option<(usize, usize, usize)> = None; // (тайл, полоса, индекс)
        let mut chase = None;
        // Предмет, за которым ехала рука, ушёл с тайла (или его забрали). Если рука в этот
        // момент над тайлом ленты, она стоит один тик (FFF #304: пауза после неудачного
        // захвата над лентой; проверено по эталону: пробная сцена p14, тик 156 — пауза;
        // p14 тик 124, p15 тик 105, p16 тик 180 — рука вне тайла, без паузы).
        let mut lost = false;
        if ins.held.is_none() {
            if let (Some(uid), Target::Belt(b)) = (ins.chase, ins.pickup) {
                lost = !self.chase_on_tile(b as usize, uid);
            }
        }
        let ins = &self.inserters[i];
        let target = if ins.held.is_some() {
            ins.at_drop
        } else {
            match ins.pickup {
                Target::Belt(b) => match self.belt_pick_candidate(ins, b as usize) {
                    Some((lane, idx, t)) => {
                        belt_pick = Some((b as usize, lane, idx));
                        let li = self.belts[b as usize].line as usize;
                        chase = Some(self.lines[li].lanes[lane].items[idx].uid);
                        t
                    }
                    None => ins.at_pickup,
                },
                _ => ins.at_pickup,
            }
        };
        // 2. рука
        let ins = &mut self.inserters[i];
        if lost {
            if let Target::Belt(b) = ins.pickup {
                // «Над тайлом» — по плоскому положению руки (угол и вылет), без вычета высоты `h`.
                // Пробы p14–p16 (рука над тайлом — пауза, вне тайла — без паузы) с этим сходятся;
                // они проходят и с проверкой по `hand.offset()` (с высотой), так что выбор между
                // двумя вариантами пробами не различён — плоский взят как более простой.
                let tile = self.belts[b as usize].pos;
                let (hx, hy) = fixed::hand_offset(ins.hand.o, ins.hand.ext);
                let (ax, ay) = (ins.pos.0 as i64 + hx, ins.pos.1 as i64 + hy);
                let half = (TILE / 2) as i64;
                let over = (ax - tile.0 as i64).abs() <= half && (ay - tile.1 as i64).abs() <= half;
                if over {
                    ins.chase = None;
                    return;
                }
            }
        }
        ins.chase = chase;
        let arrived = ins.hand.step(target, rot_speed, ext_speed);
        if !arrived {
            return;
        }
        // 3. действие
        if let Some(item) = self.inserters[i].held {
            let ok = match self.inserters[i].drop {
                Target::Chest(c) => {
                    let ss = self.item_stack[item as usize];
                    let n = self.inserters[i].held_count;
                    let chest = &mut self.chests[c as usize];
                    if chest.inv.room_for(item, ss) >= n {
                        chest.inv.insert(item, n, ss);
                        chest.add_fresh(item, n);
                        true
                    } else {
                        false
                    }
                }
                Target::Belt(b) => {
                    let tile = &self.belts[b as usize];
                    let (lane, drop_pos) = tile.drop_lane_pos(self.inserters[i].drop_point);
                    let li = tile.line as usize;
                    let limit = self.end_limit(li, lane);
                    let speed = self.lines[li].speed;
                    let uid = self.next_uid;
                    let len = self.lines[li].len();
                    // впадает ли в линию сзади другая линия (тогда кладка за хвост не делается)
                    let fed = self.lines.iter().any(|l| l.next == Some(li as u32));
                    let ok = self.lines[li].lanes[lane].try_insert(drop_pos, speed, limit, len, fed, item, uid);
                    if ok {
                        self.next_uid = self.next_uid.checked_add(1).expect("uid предмета: переполнение u64");
                    }
                    ok
                }
                Target::None => false,
            };
            if ok {
                let ins = &mut self.inserters[i];
                ins.held = None;
                ins.held_count = 0;
            }
            return;
        }
        match self.inserters[i].pickup {
            Target::Chest(c) => {
                // Последний слот с предметом, которого в сундуке больше, чем положено в
                // этом тике (положенное в этом тике не берут — по типу предмета).
                let Some(item) = self.chests[c as usize].takeable() else {
                    return;
                };
                if !self.drop_accepts(&self.inserters[i], item) {
                    return;
                }
                self.chests[c as usize].inv.remove(item, 1);
                let ins = &mut self.inserters[i];
                ins.held = Some(item);
                ins.held_count = 1;
            }
            Target::Belt(_) => {
                if let Some((b, lane, idx)) = belt_pick {
                    let li = self.belts[b].line as usize;
                    let it = self.lines[li].lanes[lane].items.remove(idx);
                    let ins = &mut self.inserters[i];
                    ins.held = Some(it.item);
                    ins.held_count = 1;
                    ins.chase = None;
                }
            }
            Target::None => {}
        }
    }

    /// Состояние в формате строки эталона.
    pub fn dump(&self) -> TickDump {
        let name = |id: ItemId| self.item_names[id as usize].clone();
        let belts = self
            .belts
            .iter()
            .map(|t| {
                let line = &self.lines[t.line as usize];
                let lo = t.k as Fx * TILE_LANE_LEN;
                let lane_items = |lane: usize| -> Vec<DumpItem> {
                    let mut v: Vec<DumpItem> = line.lanes[lane]
                        .range(lo, lo + TILE_LANE_LEN)
                        .map(|(_, it)| DumpItem { item: name(it.item), p: it.pos - lo })
                        .collect();
                    v.sort_by_key(|d| d.p);
                    v
                };
                DumpBelt { pos: Pos(t.pos.0, t.pos.1), lines: [lane_items(0), lane_items(1)] }
            })
            .collect();
        let inserters = self
            .inserters
            .iter()
            .map(|ins| {
                let (hx, hy) = ins.hand.offset();
                DumpInserter {
                    pos: Pos(ins.pos.0, ins.pos.1),
                    held: ins.held.map(name),
                    held_count: ins.held_count,
                    hand: [ins.pos.0 + hx, ins.pos.1 + hy],
                }
            })
            .collect();
        let chests = self
            .chests
            .iter()
            .map(|c| {
                let items: BTreeMap<String, u32> =
                    c.inv.totals().into_iter().map(|(id, n)| (name(id), n)).collect();
                let mut slots: Vec<Option<(String, u32)>> =
                    c.inv.slots.iter().map(|s| s.map(|st| (name(st.item), st.count))).collect();
                while slots.last().is_some_and(|s| s.is_none()) {
                    slots.pop();
                }
                DumpChest { pos: Pos(c.pos.0, c.pos.1), items, slots }
            })
            .collect();
        TickDump { t: self.tick, belts, inserters, chests }
    }
}
