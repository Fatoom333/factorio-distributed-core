//! Сундуки и их инвентари.

use std::collections::BTreeMap;

use serde::Serialize;

use super::belt::ItemId;
use super::fixed::Fx;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Stack {
    pub item: ItemId,
    pub count: u32,
}

/// Инвентарь из `slots` слотов.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Inventory {
    pub slots: Vec<Option<Stack>>,
}

impl Inventory {
    pub fn new(slots: u32) -> Self {
        Inventory { slots: vec![None; slots as usize] }
    }

    /// Сколько `item` влезет (с учётом неполных стопок и пустых слотов).
    pub fn room_for(&self, item: ItemId, stack_size: u32) -> u32 {
        self.slots
            .iter()
            .map(|s| match s {
                None => stack_size,
                Some(st) if st.item == item => stack_size.saturating_sub(st.count),
                Some(_) => 0,
            })
            .sum()
    }

    /// Вставка: сначала доливаем неполные стопки того же предмета, потом первые пустые
    /// слоты (как `LuaInventory.insert`). Возвращает, сколько вставлено.
    pub fn insert(&mut self, item: ItemId, mut count: u32, stack_size: u32) -> u32 {
        let want = count;
        for st in self.slots.iter_mut().flatten() {
            if count == 0 {
                break;
            }
            if st.item == item && st.count < stack_size {
                let n = (stack_size - st.count).min(count);
                st.count += n;
                count -= n;
            }
        }
        for slot in self.slots.iter_mut() {
            if count == 0 {
                break;
            }
            if slot.is_none() {
                let n = stack_size.min(count);
                *slot = Some(Stack { item, count: n });
                count -= n;
            }
        }
        want - count
    }

    /// Сколько всего `item` в инвентаре.
    pub fn count(&self, item: ItemId) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    /// Забрать до `count` штук `item` с конца инвентаря (из последних слотов), как
    /// манипулятор (проверено по эталону: поле `slots` во всех сценах — из сундука
    /// 100 + 100 манипулятор берёт из второго слота). Возвращает, сколько забрано.
    pub fn remove(&mut self, item: ItemId, mut count: u32) -> u32 {
        let want = count;
        for slot in self.slots.iter_mut().rev() {
            if count == 0 {
                break;
            }
            if let Some(st) = slot {
                if st.item == item {
                    let n = st.count.min(count);
                    st.count -= n;
                    count -= n;
                    if st.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        want - count
    }

    /// Суммы по предметам.
    pub fn totals(&self) -> BTreeMap<ItemId, u32> {
        let mut m = BTreeMap::new();
        for st in self.slots.iter().flatten() {
            *m.entry(st.item).or_insert(0) += st.count;
        }
        m
    }
}

/// Сундук.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Chest {
    /// Центр, 1/256 тайла.
    pub pos: (Fx, Fx),
    pub inv: Inventory,
    /// Сколько положено манипуляторами в текущем тике, по типу предмета (их ещё нельзя
    /// взять). Обычно 0–1 записей; очищается в начале тика.
    pub fresh: Vec<(ItemId, u32)>,
}

impl Chest {
    pub fn add_fresh(&mut self, item: ItemId, n: u32) {
        match self.fresh.iter_mut().find(|(i, _)| *i == item) {
            Some((_, c)) => *c += n,
            None => self.fresh.push((item, n)),
        }
    }

    fn fresh_of(&self, item: ItemId) -> u32 {
        self.fresh.iter().find(|(i, _)| *i == item).map_or(0, |(_, c)| *c)
    }

    /// Предмет, который возьмёт манипулятор: последний непустой слот, чей предмет есть в
    /// сундуке сверх положенного в этом тике (манипулятор берёт с конца инвентаря —
    /// проверено по эталону, поле `slots`; сундук с двумя типами — probes/p13-twoitems).
    pub fn takeable(&self) -> Option<ItemId> {
        self.inv
            .slots
            .iter()
            .rev()
            .flatten()
            .map(|s| s.item)
            .find(|&item| self.inv.count(item) > self.fresh_of(item))
    }
}
