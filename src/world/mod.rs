//! Модель мира M0: ленты, сундуки, манипуляторы. Совпадает с движком Factorio потиково
//! (сверка — tests/fidelity.rs, программа `fidelity`, docs/model-fidelity.md).
//!
//! Состояние — плоские данные: `Vec` и индексы, без указателей; `World: Clone`, снимок —
//! это `clone()`. В состоянии нет `f32`/`f64`: позиции в 1/256 тайла, углы — `u64`
//! (доля оборота), вылет руки — `double`, высота руки — `float` в целочисленной эмуляции
//! (`softfloat::Sf`: знак, мантисса, порядок — целые).
//!
//! Порядок внутри тика (проверено по эталону, см. docs/mechanics/*.md):
//! 1. ленты: заявки на перенарезку линий, затем активные линии сдвигают предметы, затем
//!    будильники манипуляторов, спавших на сдвинувшихся линиях (`World::step_belts` в `belt.rs`);
//! 2. активные манипуляторы в порядке списка активных (разбуженный встаёт в его начало):
//!    выбирают цель, двигают руку, берут/кладут, засыпают (`wake.rs`).
//!    Манипулятор видит предметы уже после сдвига лент этого тика.

pub mod belt;
pub mod fixed;
pub mod inserter;
pub mod inventory;
pub mod layout;
pub mod proto;
pub mod softfloat;
pub mod wake;

use std::collections::BTreeMap;

use serde::Serialize;

use belt::{ItemId, Line, LineGroup, Remerge, TileLane};
use fixed::{hypot_f64, orientation_f32, Fx, TILE};
use inserter::{HandTarget, Inserter, Target};
use inventory::Chest;
use layout::{DumpBelt, DumpChest, DumpInserter, DumpItem, LayoutError, Pos, Scene, TickDump};
use proto::{LANE_OFFSET, TILE_LANE_LEN};
use wake::{WakeRef, NONE};

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
    /// Полосы тайла (0 — левая, 1 — правая): линия, место на ней, отметка взаимодействия.
    pub lanes: [TileLane; 2],
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

    /// Куда попадает точка выгрузки `dp` на этом тайле: полоса и расстояние до переднего края
    /// тайла (0..=255).
    ///
    /// Полоса — по стороне точки от оси ленты; на оси — правая (проверено по эталону: кладка
    /// «в лоб» — полоса 1). Расстояние — `128 − вдоль`, где `вдоль` — смещение точки от центра
    /// тайла по ходу ленты, зажатое в `[0, 255]`: точка ровно на заднем (входном) крае тайла даёт
    /// 255 (проверено по эталону: пробы `edge-*`, docs/mechanics/belts.md, «Кладка на край тайла»).
    pub fn drop_lane_pos(&self, dp: (Fx, Fx)) -> (usize, Fx) {
        let rel = (dp.0 - self.pos.0, dp.1 - self.pos.1);
        let (ux, uy) = self.dir.unit();
        let (rx, ry) = self.dir.right();
        let along = rel.0 * ux + rel.1 * uy;
        let lateral = rel.0 * rx + rel.1 * ry;
        let lane = if lateral < 0 { 0 } else { 1 };
        (lane, (TILE / 2 - along).clamp(0, TILE_LANE_LEN - 1))
    }
}

/// Мир M0.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct World {
    pub tick: u64,
    /// Следующий `uid` предмета на ленте (`u64`, без переполнения — см. `LineItem::uid`).
    pub next_uid: u64,
    pub item_names: Vec<String>,
    pub item_stack: Vec<u32>,
    /// Прототипы лент и манипуляторов, которые есть в сцене (`BeltTile::proto`,
    /// `Inserter::proto` — индексы сюда). Числа — из выжимки `data.raw`, см. `proto`.
    pub belt_protos: Vec<proto::BeltProto>,
    pub ins_protos: Vec<proto::InserterProto>,
    pub belts: Vec<BeltTile>,
    /// Транспортные линии (куски полос, см. `belt`). Линии режутся и сливаются на ходу; ячейки
    /// влитых линий (`alive == false`) ждут в `free_lines` и занимаются заново.
    pub lines: Vec<Line>,
    pub free_lines: Vec<u32>,
    /// Группы линий: полоса цепочки лент. У каждой — свой список активных линий.
    pub line_groups: Vec<LineGroup>,
    /// Группы, в которых есть активные линии, в порядке активации (один список на весь мир):
    /// задаёт порядок будильников из обновления лент (`World::step_belts`).
    pub active_groups: Vec<u32>,
    /// Будильники, отложенные до конца обновления лент: (группа, список). Между тиками пуст.
    pub line_alarms: Vec<(u32, WakeRef)>,
    /// Рабочий список групп для `step_belts`. Между тиками пуст.
    pub group_scratch: Vec<u32>,
    /// Рабочий список линий для обновления цепочки (`update_line`). Между тиками пуст.
    pub chain_scratch: Vec<u32>,
    /// Курсор прохода по списку активных (группа, индекс следующей линии) — только во время
    /// обновления лент; между тиками `None`.
    pub line_cursor: Option<(u32, u32)>,
    /// Заявки на перенарезку цепочек и счётчик их номеров.
    pub remerges: Vec<Remerge>,
    pub remerge_seq: u64,
    pub inserters: Vec<Inserter>,
    /// Первый манипулятор списка активных (`NONE` — все спят); порядок списка — порядок
    /// обновления (`Inserter::act_next`, см. `wake`).
    pub ins_head: u32,
    pub chests: Vec<Chest>,
}

impl World {
    /// Мир из сцены; числа сущностей — из `protos` (обычно [`proto::Prototypes::bundled`]).
    pub fn from_scene(scene: &Scene, protos: &proto::Prototypes) -> Result<World, LayoutError> {
        layout::build(scene, protos)
    }

    /// Один тик.
    pub fn step(&mut self) {
        // Номер обновления: обновление `now` превращает снимок `now` в снимок `now + 1`.
        let now = self.tick as i64;
        self.tick += 1;
        self.step_belts(now);
        self.step_inserters(now);
    }

    /// Обновление манипуляторов: проход по списку активных от начала к концу.
    ///
    /// Разбуженный во время прохода встаёт в начало списка — позади курсора, поэтому в этом
    /// тике он не обновляется. Отсюда «положенное в сундук берут со следующего тика», когда
    /// берущий спал; если берущий не спал и стоит в списке после положившего, он берёт в том же
    /// тике (проверено по эталону: пробы `ins-sametick-belt`, `ins-sametick-chest`).
    pub(crate) fn step_inserters(&mut self, now: i64) {
        let mut cur = self.ins_head;
        while cur != NONE {
            // следующий запоминается до обновления: текущий может уснуть и выйти из списка
            let next = self.inserters[cur as usize].act_next;
            self.update_inserter(cur as usize, now);
            cur = next;
        }
    }

    /// Положить предметы в сундук снаружи (не манипулятором): как любое пополнение, будит
    /// манипуляторы, ждущие в этом сундуке предмета. Возвращает, сколько влезло.
    pub fn chest_insert(&mut self, c: usize, item: ItemId, count: u32) -> u32 {
        let n = self.chests[c].inv.insert(item, count, self.item_stack[item as usize]);
        if n > 0 {
            self.alarm(WakeRef::ChestOut(c as u32));
        }
        n
    }

    /// Забрать предметы из сундука снаружи: будит манипуляторы, ждущие в этом сундуке места.
    pub fn chest_remove(&mut self, c: usize, item: ItemId, count: u32) -> u32 {
        let n = self.chests[c].inv.remove(item, count);
        if n > 0 {
            self.alarm(WakeRef::ChestIn(c as u32));
        }
        n
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

    /// Отрезок полосы `lane` тайла `b` на его линии: (линия, начало, конец). У заднего тайла
    /// линии конец открыт: предмет, отодвинутый за задний конец линии, числится на нём.
    pub fn lane_span(&self, b: usize, lane: usize) -> (u32, Fx, Fx) {
        let tl = &self.belts[b].lanes[lane];
        let hi = tl.start + TILE_LANE_LEN;
        let hi = if hi >= self.lines[tl.line as usize].len { Fx::MAX } else { hi };
        (tl.line, tl.start, hi)
    }

    /// Где на тайле ленты `b` лежит предмет `uid`: (полоса, индекс в линии, место в тайле).
    fn find_on_tile(&self, b: usize, uid: u64) -> Option<(usize, usize, Fx)> {
        (0..2).find_map(|lane| {
            let (li, lo, hi) = self.lane_span(b, lane);
            self.lines[li as usize].range(lo, hi).find(|(_, _, it)| it.uid == uid).map(|(idx, p, _)| (lane, idx, p - lo))
        })
    }

    /// Порядок полос при выборе предмета манипулятором `i` на тайле ленты `b`.
    ///
    /// По умолчанию — левая полоса (0), потом правая (1). Наоборот — только когда лента едет
    /// **вправо, если смотреть от манипулятора на ленту**: направление «центр манипулятора →
    /// центр тайла ленты», округлённое до 1/16 оборота, плюс четверть оборота равно направлению
    /// ленты. Манипулятор прямо напротив ленты (отклонение меньше 11.25°) поэтому берёт с
    /// ближней полосы, а стоящий наискосок — всегда с левой, даже если она дальняя. В правиле
    /// участвуют центры сущностей, а не точка захвата.
    ///
    /// Проверено по эталону: пробы `ins-lane-*` (диагонали, 9.5°, 11.3°, 26.6°), фаззер
    /// (docs/mechanics/inserters.md, «Порядок полос»).
    pub fn pick_lanes(&self, i: usize, b: usize) -> [usize; 2] {
        let (ins, tile) = (&self.inserters[i], &self.belts[b]);
        let dx = (tile.pos.0 - ins.pos.0) as i64;
        let dy = (tile.pos.1 - ins.pos.1) as i64;
        // направление в 1/16 оборота, округление к ближайшему (угол — на сетке float)
        let d16 = (((orientation_f32(dx, dy) >> 59) + 1) >> 1) & 15;
        let belt16 = match tile.dir {
            Dir::North => 0,
            Dir::East => 4,
            Dir::South => 8,
            Dir::West => 12,
        };
        if (d16 + 4) & 15 == belt16 {
            [1, 0]
        } else {
            [0, 1]
        }
    }

    /// Цель руки над предметом, лежащим на полосе `lane` тайла `b` в `p` от переднего края.
    fn chase_target(&self, i: usize, b: usize, lane: usize, p: Fx) -> HandTarget {
        let ins = &self.inserters[i];
        let wp = self.belts[b].item_pos(lane, p);
        let dx = (wp.0 - ins.pos.0) as i64;
        let dy = (wp.1 - ins.pos.1) as i64;
        HandTarget { o: orientation_f32(dx, dy), ext: hypot_f64(dx, dy) }
    }

    /// Выбор предмета на тайле ленты `b` манипулятором `i`.
    ///
    /// Возвращает: выбранный предмет (полоса, индекс в линии, место в тайле); были ли на тайле
    /// предметы вообще; какие полосы манипулятор просмотрел (взаимодействие с полосой — см.
    /// `World::touch`): по порядку выбора, каждая полоса, чья **линия** не пуста, пока предмет не
    /// найден.
    ///
    /// В полосе берётся предмет, дальше всех от конца линии (последний вошедший на тайл), который
    /// примет цель выгрузки (проверено по эталону two-lanes, тик 553: из 4 упёршихся берёт
    /// p = 192). Правила «только что положенный предмет не берут» нет.
    #[allow(clippy::type_complexity)]
    pub fn select_belt_item(&self, i: usize, b: usize) -> (Option<(usize, usize, Fx)>, bool, [bool; 2]) {
        let ins = &self.inserters[i];
        let mut seen = [false; 2];
        let mut had_items = false;
        for lane in self.pick_lanes(i, b) {
            let (li, lo, hi) = self.lane_span(b, lane);
            let line = &self.lines[li as usize];
            if line.is_empty() {
                continue;
            }
            seen[lane] = true;
            let mut pick = None;
            for (idx, p, it) in line.range(lo, hi) {
                had_items = true;
                if self.drop_accepts(ins, it.item) {
                    pick = Some((lane, idx, p - lo));
                }
            }
            if pick.is_some() {
                return (pick, had_items, seen);
            }
        }
        (None, had_items, seen)
    }

    /// Шаг одного активного манипулятора (электрический, рука на 1 предмет, энергия полная).
    ///
    /// Две фазы. **Выгрузка** (в руке предмет): рука едет в точку выгрузки и кладёт; не вышло —
    /// засыпает с предметом в руке (на ленте — только если линия стоит). **Захват** (рука пуста):
    /// - сундук-приёмник полон — предмет не выбирается, рука едет в точку захвата и ждёт места;
    /// - из сундука — рука в точке захвата, слоты с конца, первый предмет, который примет цель;
    /// - с ленты — выбор предмета и погоня за ним; пока выбранный предмет на тайле, рука едет к
    ///   нему, **не проверяя приёмник**: проверка — при выборе и когда рука дошла (проба
    ///   `ins-chase-full`, c4-chase-full). Не примет — цель сбрасывается, взятия нет.
    ///
    /// Пауза после потери цели: если выбранный предмет пропал с тайла (уехал или его забрали), а
    /// рука в этот момент над тайлом ленты, манипулятор стоит один тик (FFF #304; пробы p14–p16).
    /// «Была цель» — отдельный признак (`has_target`): он ставится при удачном выборе, снимается
    /// паузой и отказом приёмника, но не взятием предмета.
    fn update_inserter(&mut self, i: usize, now: i64) {
        let ins = &self.inserters[i];
        let (rot_speed, ext_speed) = {
            let p = &self.ins_protos[ins.proto as usize];
            (p.rotation_speed, p.extension_speed)
        };
        let (pickup, drop) = (ins.pickup, ins.drop);
        // Нет источника или цели выгрузки (захват с земли, выгрузка на землю) — не M0: рука
        // встаёт в точку захвата, манипулятор выбывает из списка активных навсегда.
        if pickup == Target::None || drop == Target::None {
            let ins = &mut self.inserters[i];
            let t = ins.at_pickup;
            if ins.hand.step(t, rot_speed, ext_speed) {
                self.sleep(i);
            }
            return;
        }
        // ---- выгрузка ----
        if !ins.pickup_mode || ins.held.is_some() {
            let ins = &mut self.inserters[i];
            ins.pickup_mode = false;
            ins.chase = None;
            let t = ins.at_drop;
            if !ins.hand.step(t, rot_speed, ext_speed) {
                return;
            }
            let Some(item) = ins.held else {
                ins.pickup_mode = true;
                return;
            };
            let n = ins.held_count;
            match drop {
                Target::Chest(c) => {
                    let ss = self.item_stack[item as usize];
                    let chest = &mut self.chests[c as usize];
                    if chest.inv.room_for(item, ss) >= n {
                        chest.inv.insert(item, n, ss);
                        self.released(i);
                        self.alarm(WakeRef::ChestOut(c));
                    } else {
                        self.register(i, WakeRef::ChestIn(c));
                        self.sleep(i);
                    }
                }
                Target::Belt(b) => {
                    let (lane, in_tile) = self.belts[b as usize].drop_lane_pos(self.inserters[i].drop_point);
                    let uid = self.next_uid;
                    if self.belt_insert(b as usize, lane, in_tile, item, uid, now) {
                        self.next_uid = self.next_uid.checked_add(1).expect("uid предмета: переполнение u64");
                        self.released(i);
                    } else {
                        // Очередь перед точкой выгрузки ещё движется — пробует каждый тик; линия
                        // стоит — спит, пока на ней не появится место.
                        let li = self.belts[b as usize].lanes[lane].line;
                        if !self.lines[li as usize].active {
                            self.register(i, WakeRef::Drop(li));
                            self.sleep(i);
                        }
                    }
                }
                Target::None => {}
            }
            return;
        }
        // ---- захват ----
        if let Target::Chest(dc) = drop {
            if self.chests[dc as usize].inv.is_full(&self.item_stack) {
                let ins = &mut self.inserters[i];
                let t = ins.at_pickup;
                if ins.hand.step(t, rot_speed, ext_speed) {
                    self.try_sleep(i, true, false);
                }
                return;
            }
        }
        match pickup {
            Target::Chest(c) => {
                let ins = &mut self.inserters[i];
                let t = ins.at_pickup;
                if !ins.hand.step(t, rot_speed, ext_speed) {
                    return;
                }
                // Слоты с конца инвентаря; первый предмет, который примет цель выгрузки (тот же
                // тип второй раз подряд не проверяется). Проверено по эталону: поле `slots`,
                // сундук с двумя типами — probes/p13-twoitems.
                let ins = &self.inserters[i];
                let (mut blocked, mut prev, mut take) = (false, None, None);
                for st in self.chests[c as usize].inv.slots.iter().rev().flatten() {
                    blocked = true;
                    if prev == Some(st.item) {
                        continue;
                    }
                    prev = Some(st.item);
                    if self.drop_accepts(ins, st.item) {
                        take = Some(st.item);
                        break;
                    }
                }
                let Some(item) = take else {
                    // брать нечего — ждёт предмета; есть, но цель не принимает — ждёт и места
                    self.try_sleep(i, blocked, true);
                    return;
                };
                self.chests[c as usize].inv.remove(item, 1);
                self.grabbed(i, item);
                self.alarm(WakeRef::ChestIn(c));
            }
            Target::Belt(b) => {
                let b = b as usize;
                let mut had_items = false;
                let mut sel = ins.chase.and_then(|uid| self.find_on_tile(b, uid));
                if sel.is_none() {
                    if ins.has_target {
                        // «Над тайлом» — по плоскому положению руки (угол и вылет, без высоты):
                        // целые координаты тайла руки равны координатам тайла ленты.
                        let tile = self.belts[b].pos;
                        let (hx, hy) = fixed::hand_offset(ins.hand.o, ins.hand.ext);
                        let (ax, ay) = (ins.pos.0 as i64 + hx, ins.pos.1 as i64 + hy);
                        let t = TILE as i64;
                        if ax.div_euclid(t) == (tile.0 as i64).div_euclid(t) && ay.div_euclid(t) == (tile.1 as i64).div_euclid(t) {
                            let ins = &mut self.inserters[i];
                            ins.has_target = false;
                            ins.chase = None;
                            return;
                        }
                    }
                    let (s2, had, seen) = self.select_belt_item(i, b);
                    sel = s2;
                    had_items = had;
                    // Поиск предмета — взаимодействие с просмотренными полосами тайла захвата
                    // (проба `sk-mid`: сток посреди ленты, до которого предметы не доезжают,
                    // всё равно режет линию). Пока рука едет за выбранным предметом, отметка не
                    // обновляется. Отметки ставятся в порядке просмотра полос (`pick_lanes`): от
                    // него зависит порядок заявок на перенарезку двух полос в одном тике.
                    for lane in self.pick_lanes(i, b) {
                        if seen[lane] {
                            self.touch(b, lane, now);
                        }
                    }
                }
                self.inserters[i].has_target = sel.is_some();
                match sel {
                    Some((lane, idx, p)) => {
                        let li = self.belts[b].lanes[lane].line;
                        let it = &self.lines[li as usize].items[idx];
                        let (uid, item) = (it.uid, it.item);
                        let t = self.chase_target(i, b, lane, p);
                        let ins = &mut self.inserters[i];
                        ins.chase = Some(uid);
                        if !ins.hand.step(t, rot_speed, ext_speed) {
                            return;
                        }
                        ins.chase = None;
                        if !self.drop_accepts(&self.inserters[i], item) {
                            // приёмник заполнился, пока рука ехала: цель сброшена, без паузы
                            self.inserters[i].has_target = false;
                            return;
                        }
                        self.belt_remove(li, idx);
                        self.grabbed(i, item);
                    }
                    None => {
                        let ins = &mut self.inserters[i];
                        ins.chase = None;
                        let t = ins.at_pickup;
                        if !ins.hand.step(t, rot_speed, ext_speed) {
                            return;
                        }
                        self.try_sleep(i, had_items, true);
                    }
                }
            }
            Target::None => {}
        }
    }

    /// Рука взяла предмет: дальше — фаза выгрузки.
    fn grabbed(&mut self, i: usize, item: ItemId) {
        let ins = &mut self.inserters[i];
        ins.held = Some(item);
        ins.held_count = 1;
        ins.pickup_mode = false;
    }

    /// Рука положила предмет: дальше — фаза захвата.
    fn released(&mut self, i: usize) {
        let ins = &mut self.inserters[i];
        ins.held = None;
        ins.held_count = 0;
        ins.pickup_mode = true;
    }

    /// Состояние в формате строки эталона.
    pub fn dump(&self) -> TickDump {
        let name = |id: ItemId| self.item_names[id as usize].clone();
        let belts = self
            .belts
            .iter()
            .enumerate()
            .map(|(b, t)| {
                let lane_items = |lane: usize| -> Vec<DumpItem> {
                    let (li, lo, hi) = self.lane_span(b, lane);
                    let mut v: Vec<DumpItem> = self.lines[li as usize]
                        .range(lo, hi)
                        .map(|(_, p, it)| DumpItem { item: name(it.item), p: p - lo })
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
