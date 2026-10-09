//! Транспортные ленты: линии, полосы, предметы.
//!
//! Подряд стоящие прямые ленты одного типа и направления — одна транспортная линия
//! (как в игре). Позиция предмета `pos` — расстояние от **конца** линии (по ходу ленты)
//! до центра предмета в 1/256 тайла. Тайл с индексом `k` (0 — последний по ходу, «голова»)
//! занимает `pos ∈ [256·k, 256·(k+1))`; в эталоне `p = pos − 256·k` — расстояние до края
//! тайла по ходу ленты (проверено по эталону: `p` падает на скорость ленты за тик и при
//! переходе на следующий тайл прибавляет 256).
//!
//! Полоса 0 — левая по ходу ленты (`get_transport_line(1)`), полоса 1 — правая.

use serde::Serialize;

use super::fixed::Fx;
use super::proto::{ITEM_SPACING, TILE_LANE_LEN};

/// Индекс предмета в таблице имён мира.
pub type ItemId = u16;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct BeltItem {
    /// Расстояние до конца линии, 1/256 тайла.
    pub pos: Fx,
    pub item: ItemId,
    /// Номер предмета в мире (для «захвата цели» манипулятором), не меняется.
    /// `u64`: счётчик не переполнится за время жизни сервера (2^64 кладок).
    pub uid: u64,
}

/// Полоса: предметы по возрастанию `pos` (первый — ближе всех к концу линии).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Lane {
    pub items: Vec<BeltItem>,
}

/// Транспортная линия.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct BeltLine {
    /// Индексы лент (в `World::belts`) от головы (k = 0) к хвосту.
    pub tiles: Vec<u32>,
    /// Скорость, 1/256 тайла за тик.
    pub speed: Fx,
    pub lanes: [Lane; 2],
    /// Линия, в которую эта линия выводит предметы (лента другого типа впереди,
    /// то же направление). Сверено с игрой пробой p12-mixed (docs/mechanics/belts.md).
    pub next: Option<u32>,
}

impl BeltLine {
    pub fn len(&self) -> Fx {
        self.tiles.len() as Fx * TILE_LANE_LEN
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

impl Lane {
    /// Сдвиг полосы за тик. `limit` — ближе к концу линии, чем `limit`, первый предмет
    /// встать не может (0 — конец линии без продолжения).
    ///
    /// Предметы идут от головы к хвосту; каждый сдвигается на `speed`, но не ближе
    /// `ITEM_SPACING` к **новой** позиции предыдущего (проверено по эталону: плотная
    /// полоса едет целиком, упёршиеся стоят с шагом 64).
    pub fn advance(&mut self, speed: Fx, mut limit: Fx) {
        for it in self.items.iter_mut() {
            let np = (it.pos - speed).max(limit);
            if np < it.pos {
                it.pos = np;
            }
            limit = it.pos + ITEM_SPACING;
        }
    }

    /// Индексы предметов с `pos ∈ [lo, hi)` (предметы отсортированы по `pos` —
    /// двоичный поиск, без прохода по всей полосе).
    pub fn range(&self, lo: Fx, hi: Fx) -> impl DoubleEndedIterator<Item = (usize, &BeltItem)> {
        let a = self.items.partition_point(|it| it.pos < lo);
        let b = self.items.partition_point(|it| it.pos < hi).max(a);
        self.items[a..b].iter().enumerate().map(move |(i, it)| (a + i, it))
    }

    /// Вставка предмета манипулятором в точку `drop` (расстояние до конца линии).
    ///
    /// Манипуляторы в модели обновляются после лент (см. `World::step`), а в игре
    /// положенный предмет в том же тике уже сдвинут лентой (проверено по эталону:
    /// кладка на 77 видна как 69 / 61 / 53 для скоростей 8 / 16 / 24).
    ///
    /// Правило (проверено по эталону — пробы `tests/fidelity/probes/t*`, docs/mechanics/belts.md
    /// «Кладка рядом с другим предметом»): предметы полосы делятся точкой `drop` — с `pos < drop`
    /// «впереди», с `pos >= drop` «позади» (позиции уже после сдвига лент этого тика).
    /// 1. Точка вставки `at = max(drop, передний + ITEM_SPACING)` (и не ближе `end_limit`) —
    ///    передний отодвигает кладку назад (тупик p1: при [0, 64] кладка встаёт на 128).
    /// 2. Вставка, только если задний строго дальше точки вставки: `задний > at`
    ///    (p1: при [0, 64, 128] ждёт; пробы t3/t4/t5: задний ровно на `drop` — ждёт, на `drop + 1` —
    ///    кладёт, на всех трёх скоростях; t6: в тупике задний на `at + 2` — кладёт). Шаг 64 до
    ///    заднего **не** требуется: задний может оказаться ближе 64, тогда он стоит, пока зазор
    ///    не станет 64 (это делает `advance`).
    /// 3. Новый предмет участвует в сдвиге этого тика: `pos = max(drop − speed, передний +
    ///    ITEM_SPACING)` (и не ближе `end_limit`).
    ///
    /// 4. Хвост линии (`len` — длина линии). Предмет не бывает дальше `len − 1`, и зазор 64 до
    ///    переднего не нарушается нигде, кроме одного сверенного случая: передний стоит ровно на
    ///    `len − 64`, тогда по правилу место было бы на `len`, а игра ставит предмет на `len − 1`
    ///    (зазор 63), если в линию ничего не впадает (`fed == false`). Если сзади впадает другая
    ///    линия, манипулятор ждёт. Во всех остальных случаях, когда места нет
    ///    (`передний + 64 > len`), манипулятор тоже ждёт: поведение игры там не измерено.
    ///    Проверено по эталону: фаззер, seed 101 (сцена s101-00774, воспроизводится
    ///    `fuzz_gen --seed 101`: лента в 1 тайл, очередь [0, 64, 128, 192], кладка на 194 → 255)
    ///    и пробы `tests/fidelity/probes/tail-*`: без ленты сзади — 255 на скоростях 8/16/24/32
    ///    при кладке на 200/230/250; с обычной лентой сзади — ждёт до конца съёмки.
    ///    Точка кладки на хвосте линии (`drop >= len`) — не сверено с игрой: такие сцены
    ///    отсекает `layout::build`, а здесь манипулятор просто ждёт.
    ///
    /// Прежнее правило «до заднего не меньше 64» расходилось с игрой, когда два источника кладут
    /// рядом в одном тике (фаззер, seed 1: 12 сцен).
    #[allow(clippy::too_many_arguments)]
    pub fn try_insert(&mut self, drop: Fx, speed: Fx, end_limit: Fx, len: Fx, fed: bool, item: ItemId, uid: u64) -> bool {
        if drop >= len {
            return false;
        }
        // первый предмет с pos >= drop — «позади» точки кладки
        let idx = self.items.partition_point(|it| it.pos < drop);
        // ближе всего к концу линии, где может стоять новый предмет (передний и конец линии)
        let mut ahead_lim = end_limit;
        if idx > 0 {
            ahead_lim = ahead_lim.max(self.items[idx - 1].pos + ITEM_SPACING);
        }
        // места нет: за хвостом, а у хвоста — только сверенный случай ahead_lim == len без впадающей линии
        if ahead_lim > len || (fed && ahead_lim >= len) {
            return false;
        }
        let ahead_lim = ahead_lim.min(len - 1);
        let at = drop.max(ahead_lim);
        if let Some(behind) = self.items.get(idx) {
            if behind.pos <= at {
                return false;
            }
        }
        let pos = (drop - speed).max(ahead_lim);
        self.items.insert(idx, BeltItem { pos, item, uid });
        debug_assert!(
            self.items.windows(2).all(|w| w[0].pos < w[1].pos) && self.items.iter().all(|it| it.pos >= 0 && it.pos < len),
            "полоса не отсортирована или предмет за пределами линии: {:?}",
            self.items.iter().map(|it| it.pos).collect::<Vec<_>>()
        );
        true
    }
}

/// Сдвиг полосы `lane`, у которой впереди полоса `next` другой линии (лента другого
/// типа, то же направление). `next` уже сдвинута в этом тике.
///
/// Проверено по эталону (пробная сцена «обычная → быстрая → обычная»): предмет,
/// переезжающий стык, тратит остаток хода пропорционально скоростям — с обычной на
/// быструю p=5 → 250 (5 по обычной, ещё 3/8 тика по быстрой = 6), с быстрой на обычную
/// p=10 → 253. Округление дробного остатка — вниз (не проверено на дробных случаях).
pub fn advance_into(lane: &mut Lane, speed: Fx, next: &mut Lane, next_speed: Fx, next_len: Fx) {
    let mut prev_new: Option<Fx> = None; // новая позиция предыдущего (в координатах этой линии)
    let mut cross: Option<usize> = None;
    for i in 0..lane.items.len() {
        let pos = lane.items[i].pos;
        let np = match prev_new {
            Some(pn) => (pos - speed).max(pn + ITEM_SPACING).min(pos),
            None => {
                // где может встать въезжающий в `next` предмет (не ближе шага к последнему)
                let tail_min = next.items.last().map(|l| l.pos + ITEM_SPACING);
                let want = pos - speed;
                if want >= 0 {
                    let lim = tail_min.map(|m| m - next_len).unwrap_or(0).max(0);
                    want.max(lim).min(pos)
                } else {
                    let over = speed - pos; // ход за концом линии, в единицах этой линии
                    let rem = over * next_speed / speed;
                    let mut bp = next_len - rem;
                    if let Some(m) = tail_min {
                        bp = bp.max(m);
                    }
                    if bp >= next_len {
                        // места нет — стоим в конце (или не доезжаем до него)
                        let lim = tail_min.map(|m| m - next_len).unwrap_or(0).max(0);
                        lim.min(pos)
                    } else {
                        cross = Some(i);
                        bp - next_len
                    }
                }
            }
        };
        lane.items[i].pos = np;
        prev_new = Some(np);
    }
    if let Some(i) = cross {
        let mut it = lane.items.remove(i);
        it.pos += next_len;
        next.items.push(it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(ps: &[Fx]) -> Lane {
        Lane { items: ps.iter().enumerate().map(|(i, &pos)| BeltItem { pos, item: 0, uid: i as u64 }).collect() }
    }

    fn pos(l: &Lane) -> Vec<Fx> {
        l.items.iter().map(|it| it.pos).collect()
    }

    /// Числа — из эталонов проб `tests/fidelity/probes/t*` и `p1-deadend`.
    #[test]
    fn insert_next_to_other_item() {
        // t3a: задний на d + 1 — кладёт, зазор 9 < 64
        let mut l = lane(&[136]);
        assert!(l.try_insert(135, 8, 0, 1024, false, 0, 9));
        assert_eq!(pos(&l), vec![127, 136]);
        // t3b: задний ровно на d — ждёт
        let mut l = lane(&[136]);
        assert!(!l.try_insert(136, 8, 0, 1024, false, 0, 9));
        // t1a: передний положен в этом тике (53), кладка на 128 → 104, зазор 51
        let mut l = lane(&[104]);
        assert!(l.try_insert(77, 24, 0, 1024, false, 0, 9));
        assert_eq!(pos(&l), vec![53, 104]);
        // t1b: передний на 53 отодвигает кладку: 128 − 24 = 104 → 53 + 64 = 117
        let mut l = lane(&[53]);
        assert!(l.try_insert(128, 24, 0, 1024, false, 0, 9));
        assert_eq!(pos(&l), vec![53, 117]);
        // t6b: тупик [0, 64], кладка отодвинута на 128, задний на 130 — кладёт
        let mut l = lane(&[0, 64, 130]);
        assert!(l.try_insert(77, 16, 0, 1024, false, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 130]);
        // p1 / t6a: задний ровно на 128 — ждёт
        let mut l = lane(&[0, 64, 128]);
        assert!(!l.try_insert(77, 8, 0, 1024, false, 0, 9));
    }

    /// Хвост линии из 1 тайла (len = 256): зазор 64 не нарушается, кроме сверенного случая.
    #[test]
    fn insert_at_tail_keeps_spacing() {
        // сверено (tail-b8-p200, s101-00774): передний на 192, место по правилу на 256 → 255
        let mut l = lane(&[0, 64, 128, 192]);
        assert!(l.try_insert(200, 8, 0, 256, false, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 192, 255]);
        // то же, но сзади впадает другая линия — ждёт
        let mut l = lane(&[0, 64, 128, 192]);
        assert!(!l.try_insert(200, 8, 0, 256, true, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 192]);
        // не сверено: передний на 250 — места нет, раньше предмет вставал на 255 с зазором 5
        let mut l = lane(&[250]);
        assert!(!l.try_insert(255, 8, 0, 256, false, 0, 9));
        assert_eq!(pos(&l), vec![250]);
        // не сверено: передний на 193 (зазор был бы 62) — ждёт
        let mut l = lane(&[0, 64, 128, 193]);
        assert!(!l.try_insert(200, 8, 0, 256, false, 0, 9));
        // не сверено: точка кладки ровно на хвосте (drop == len) — ждёт, на полосе нет наложений
        let mut l = lane(&[0, 64, 128, 192]);
        assert!(!l.try_insert(256, 8, 0, 256, false, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 192]);
        // а на свободной полосе место есть
        let mut l = lane(&[]);
        assert!(l.try_insert(255, 8, 0, 256, false, 0, 9));
        assert_eq!(pos(&l), vec![247]);
    }
}
