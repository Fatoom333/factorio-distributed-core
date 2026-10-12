//! Сон и пробуждение манипуляторов: список активных и списки спящих.
//!
//! Правила — docs/mechanics/inserters.md, раздел «Порядок, сон и пробуждение» (выведено из
//! наблюдений за Factorio 2.0.77 и сверено с эталонами). Коротко:
//!
//! - Обновляются только **активные** манипуляторы, в порядке списка активных. Разбуженный
//!   встаёт в **начало** списка; если его разбудили во время прохода по списку, в этом тике он
//!   уже не обновляется.
//! - Спящий манипулятор записан в списки тех, кто может его разбудить: у сундука — «ждут
//!   предмета» и «ждут места», у линии ленты — то же. Запись — в конец списка.
//! - Будильник списка будит **с конца** и выписывает разбуженного из всех его списков. Каждый
//!   встаёт в начало списка активных, поэтому спящие одного списка после будильника идут в
//!   порядке засыпания и все — впереди тех, кто не спал.
//!
//! Сон ничего не меняет в движении руки — он меняет только порядок обновления: кто из двух
//! манипуляторов первым возьмёт последний предмет или займёт последнее место.
//!
//! Список активных — двусвязный список на индексах (`Inserter::act_prev` / `act_next`,
//! `World::ins_head`): вставка в начало и удаление за O(1), состояние остаётся плоским.

use serde::Serialize;

use super::inserter::Target;
use super::World;

/// «Нет элемента» в индексных списках.
pub const NONE: u32 = u32::MAX;

/// Список спящих: у кого и чего ждут.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum WakeRef {
    /// Ждут предмета на линии ленты (индекс в `World::lines`).
    Pick(u32),
    /// Ждут места на линии ленты.
    Drop(u32),
    /// Ждут предмета в сундуке (индекс в `World::chests`).
    ChestOut(u32),
    /// Ждут места в сундуке.
    ChestIn(u32),
}

impl World {
    pub(super) fn wake_list(&self, r: WakeRef) -> &Vec<u32> {
        match r {
            WakeRef::Pick(l) => &self.lines[l as usize].pick,
            WakeRef::Drop(l) => &self.lines[l as usize].drop,
            WakeRef::ChestOut(c) => &self.chests[c as usize].out_list,
            WakeRef::ChestIn(c) => &self.chests[c as usize].in_list,
        }
    }

    pub(super) fn wake_list_mut(&mut self, r: WakeRef) -> &mut Vec<u32> {
        match r {
            WakeRef::Pick(l) => &mut self.lines[l as usize].pick,
            WakeRef::Drop(l) => &mut self.lines[l as usize].drop,
            WakeRef::ChestOut(c) => &mut self.chests[c as usize].out_list,
            WakeRef::ChestIn(c) => &mut self.chests[c as usize].in_list,
        }
    }

    /// Активные манипуляторы в порядке обновления (для отладки и тестов).
    pub fn active_inserters(&self) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = self.ins_head;
        while cur != NONE {
            out.push(cur);
            cur = self.inserters[cur as usize].act_next;
        }
        out
    }

    /// Разбудить: в начало списка активных. Уже активный остаётся на своём месте.
    pub(super) fn wake(&mut self, i: usize) {
        if self.inserters[i].awake {
            return;
        }
        let head = self.ins_head;
        let ins = &mut self.inserters[i];
        ins.awake = true;
        ins.act_prev = NONE;
        ins.act_next = head;
        if head != NONE {
            self.inserters[head as usize].act_prev = i as u32;
        }
        self.ins_head = i as u32;
    }

    /// Усыпить: убрать из списка активных (записи в списках спящих делает вызывающий).
    pub(super) fn sleep(&mut self, i: usize) {
        let ins = &mut self.inserters[i];
        if !ins.awake {
            return;
        }
        ins.awake = false;
        let (p, n) = (ins.act_prev, ins.act_next);
        ins.act_prev = NONE;
        ins.act_next = NONE;
        if p != NONE {
            self.inserters[p as usize].act_next = n;
        } else {
            self.ins_head = n;
        }
        if n != NONE {
            self.inserters[n as usize].act_prev = p;
        }
    }

    /// Записать спящего в конец списка (повторно в тот же список не записывается).
    pub(super) fn register(&mut self, i: usize, r: WakeRef) {
        if self.inserters[i].regs.contains(&r) {
            return;
        }
        self.wake_list_mut(r).push(i as u32);
        self.inserters[i].regs.push(r);
    }

    /// Выписать манипулятор из всех его списков; из списка `popped` он уже вынут.
    pub(super) fn unregister_all(&mut self, i: usize, popped: Option<WakeRef>) {
        let mut regs = std::mem::take(&mut self.inserters[i].regs);
        for &r in &regs {
            if Some(r) != popped {
                self.unlist(i, r);
            }
        }
        regs.clear();
        self.inserters[i].regs = regs; // память списка переиспользуется
    }

    fn unlist(&mut self, i: usize, r: WakeRef) {
        let list = self.wake_list_mut(r);
        // недавно уснувшие — в конце списка
        if let Some(k) = list.iter().rposition(|&x| x as usize == i) {
            list.remove(k);
        }
    }

    /// Будильник списка: будит с конца; разбуженный выписывается из всех своих списков и встаёт
    /// в начало списка активных.
    pub(super) fn alarm(&mut self, r: WakeRef) {
        while let Some(i) = self.wake_list_mut(r).pop() {
            self.unregister_all(i as usize, Some(r));
            self.wake(i as usize);
        }
    }

    /// Спящий манипулятор заново записывается на линии лент после того, как линию под его
    /// тайлом разрезали: записи на лентах снимаются и ставятся в конец списков линий, к которым
    /// его тайлы относятся теперь (ждал предмета — обе полосы тайла захвата, левая, потом правая;
    /// ждал места — линия точки выгрузки). Записи у сундуков не меняются; манипулятор не
    /// просыпается. Порядок записи в списки разных линий на исход не влияет: каждый список
    /// хранит свой порядок засыпания.
    pub(super) fn reregister_belt(&mut self, i: usize) {
        let mut regs = std::mem::take(&mut self.inserters[i].regs);
        let (mut had_pick, mut had_drop) = (false, false);
        let mut k = 0;
        while k < regs.len() {
            let r = regs[k];
            match r {
                WakeRef::Pick(_) => had_pick = true,
                WakeRef::Drop(_) => had_drop = true,
                _ => {
                    k += 1;
                    continue;
                }
            }
            self.unlist(i, r);
            regs.remove(k);
        }
        self.inserters[i].regs = regs;
        if had_pick {
            if let Target::Belt(b) = self.inserters[i].pickup {
                for lane in 0..2 {
                    let li = self.belts[b as usize].lanes[lane].line;
                    self.register(i, WakeRef::Pick(li));
                }
            }
        }
        if had_drop {
            if let Target::Belt(b) = self.inserters[i].drop {
                let (lane, _) = self.belts[b as usize].drop_lane_pos(self.inserters[i].drop_point);
                let li = self.belts[b as usize].lanes[lane].line;
                self.register(i, WakeRef::Drop(li));
            }
        }
    }

    /// Попытка уснуть: `room` — ждать места у цели выгрузки, `item` — ждать предмета у источника.
    ///
    /// Манипулятор засыпает, только если разбудить его может одно из событий, на которые он
    /// записывается, и ни одно из них не идёт прямо сейчас:
    /// - берёт с ленты — обе линии под тайлом захвата (левая и правая полосы) неактивны: в этом
    ///   тике на них ничего не двигалось и их не трогал другой манипулятор;
    /// - кладёт на ленту (ждёт места) — линия точки выгрузки неактивна или на ней нет места;
    /// - сундук будит сам: любое пополнение — ждущих предмета, любое изъятие — ждущих места.
    pub(super) fn try_sleep(&mut self, i: usize, room: bool, item: bool) {
        let ins = &self.inserters[i];
        let (pickup, drop, held) = (ins.pickup, ins.drop, ins.held.is_some());
        // не больше трёх записей: цель выгрузки и две полосы
        let mut lists: [Option<WakeRef>; 3] = [None; 3];
        if item {
            match pickup {
                Target::Belt(b) => {
                    if !held {
                        let l = &self.belts[b as usize].lanes;
                        let (l0, l1) = (l[0].line, l[1].line);
                        if self.lines[l0 as usize].is_empty() && self.lines[l1 as usize].is_empty() {
                            if self.lines[l0 as usize].active || self.lines[l1 as usize].active {
                                return;
                            }
                            for lane in 0..2 {
                                let li = self.belts[b as usize].lanes[lane].line;
                                self.register(i, WakeRef::Pick(li));
                            }
                            self.sleep(i);
                            return;
                        }
                    }
                }
                Target::Chest(c) => {
                    if !held && !room {
                        self.register(i, WakeRef::ChestOut(c));
                        self.sleep(i);
                        return;
                    }
                }
                Target::None | Target::Ground | Target::Train => return,
            }
        }
        if room {
            match drop {
                Target::Belt(b) => {
                    let (lane, _) = self.belts[b as usize].drop_lane_pos(self.inserters[i].drop_point);
                    let li = self.belts[b as usize].lanes[lane].line;
                    if self.lines[li as usize].active && self.line_has_room(li) {
                        return;
                    }
                    lists[0] = Some(WakeRef::Drop(li));
                }
                Target::Chest(c) => {
                    lists[0] = Some(WakeRef::ChestIn(c));
                }
                // ждать места на земле нечем: манипулятор не спит и пробует каждый тик
                Target::None | Target::Ground | Target::Train => return,
            }
        }
        if item {
            match pickup {
                Target::Belt(b) => {
                    for lane in 0..2 {
                        let li = self.belts[b as usize].lanes[lane].line;
                        if self.lines[li as usize].active {
                            return;
                        }
                        lists[1 + lane] = Some(WakeRef::Pick(li));
                    }
                }
                Target::Chest(c) => {
                    lists[1] = Some(WakeRef::ChestOut(c));
                }
                Target::None | Target::Ground | Target::Train => return,
            }
        }
        for r in lists.into_iter().flatten() {
            self.register(i, r);
        }
        self.sleep(i);
    }
}
