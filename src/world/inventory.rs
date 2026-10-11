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
    /// Спящие манипуляторы (индексы в `World::inserters`): ждут предмета (берут из сундука) и
    /// ждут места (кладут в сундук). Порядок — порядок засыпания (см. `wake`).
    pub out_list: Vec<u32>,
    pub in_list: Vec<u32>,
}

impl Chest {
    pub fn new(pos: (Fx, Fx), inv: Inventory) -> Chest {
        Chest { pos, inv, out_list: Vec::new(), in_list: Vec::new() }
    }
}

impl Inventory {
    /// Все слоты заняты полными стопками (`stack_size` — размер стопки по индексу предмета).
    /// Манипулятор, который кладёт в такой сундук, вообще не выбирает предмет: встаёт в точку
    /// захвата и ждёт места.
    pub fn is_full(&self, stack_size: &[u32]) -> bool {
        self.slots.iter().all(|s| s.is_some_and(|st| st.count >= stack_size[st.item as usize]))
    }
}
