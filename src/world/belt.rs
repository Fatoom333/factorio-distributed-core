//! Транспортные ленты: линии, предметы, порядок обновления.
//!
//! Правила — docs/mechanics/belts.md, раздел «Линии: нарезка, порядок обновления, сон»
//! (выведено из наблюдений за Factorio 2.0.77 и сверено с эталонами). Коротко:
//!
//! - Полоса цепочки лент (левая и правая — отдельно) хранится не целиком, а **линиями**: куски
//!   подряд стоящих тайлов одной скорости. Границы линий меняются на ходу: линия режется через
//!   два тайла ниже по течению от тайла, с которым работал манипулятор (`World::remerge`).
//! - Каждая линия обновляется отдельно. Обновляются только **активные** линии, в порядке списка
//!   активных своей группы; перед своим ходом линия обновляет линию ниже по течению, только если
//!   та активна. Линия, в которой за тик ничего не сдвинулось, засыпает.
//! - Предметы линии хранятся зазорами: `fg` — от переднего конца линии до первого предмета,
//!   `gap` у предмета — до следующего сзади, `bg` — от последнего предмета до заднего конца.
//!   Позиция предмета — расстояние до **переднего** конца линии (куда едут предметы), 1/256 тайла.
//!
//! Полоса 0 — левая по ходу ленты (`get_transport_line(1)`), полоса 1 — правая.

use serde::Serialize;

use super::fixed::Fx;
use super::proto::{ITEM_SPACING, TILE_LANE_LEN};
use super::wake::{WakeRef, NONE};
use super::World;

/// Индекс предмета в таблице имён мира.
pub type ItemId = u16;

/// Запас длины новой линии при нарезке: 200 тайлов (линия не бывает длиннее). Допущение модели,
/// пробами не проверено (сцены M0 короче).
pub const LINE_BUDGET: Fx = 200 * TILE_LANE_LEN;
/// Запас после тайла, с которым работал манипулятор: 3 тайла плюс половина длины этого тайла.
/// Для прямых лент это «тайл с отметкой и ещё два ниже по течению» (пробы перезапуска очереди).
pub const TOUCHED_BUDGET: Fx = 3 * TILE_LANE_LEN;
/// Перенарезка откладывается на `hash mod 600` тиков от первого взаимодействия
/// (пробы `cal-*`: граница с точностью до тика).
pub const REMERGE_SPREAD: u32 = 600;
/// Отметка взаимодействия «выдыхается» через 3000 тиков без повторов; проверка давности —
/// через `3000 + hash mod 6000` тиков после нарезки (пробы `decay-*`, с точностью до цикла
/// манипулятора).
pub const TOUCH_TTL: i64 = 3000;
pub const DECAY_SPREAD: u32 = 6000;

/// Предмет на линии.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct LineItem {
    /// Расстояние до следующего предмета сзади; у последнего предмета линии — 0.
    pub gap: Fx,
    pub item: ItemId,
    /// Номер предмета в мире (для «захвата цели» манипулятором), не меняется.
    /// `u64`: счётчик не переполнится за время жизни сервера (2^64 кладок).
    pub uid: u64,
}

/// Полоса одного тайла ленты: к какой линии относится и где на ней лежит.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct TileLane {
    /// Индекс в `World::lines`.
    pub line: u32,
    /// Начало тайла на линии (расстояние от переднего конца линии до переднего края тайла).
    pub start: Fx,
    /// Тик последнего взаимодействия манипулятора с этой полосой тайла (`None` — не было или
    /// выдохлось).
    pub touched: Option<i64>,
}

/// Транспортная линия: кусок одной полосы цепочки лент, обновляется как целое.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Line {
    /// `false` — ячейка свободна (линия влита в соседнюю), индекс ждёт в `World::free_lines`.
    pub alive: bool,
    /// Полоса: 0 — левая, 1 — правая.
    pub lane: u8,
    /// Группа (полоса цепочки лент) — индекс в `World::line_groups`.
    pub group: u32,
    /// Скорость, 1/256 тайла за тик.
    pub speed: Fx,
    /// Длина, 1/256 тайла.
    pub len: Fx,
    /// Индексы лент (в `World::belts`) от переднего тайла к заднему.
    pub tiles: Vec<u32>,
    /// Предметы от переднего к заднему.
    pub items: Vec<LineItem>,
    /// От переднего конца линии до первого предмета (у пустой линии 0).
    pub fg: Fx,
    /// От последнего предмета до заднего конца линии (у пустой линии 0).
    pub bg: Fx,
    /// Индекс первого предмета с зазором больше 64 за ним (−1 — нет): «что сжимать».
    pub gi: i32,
    /// Индекс первого предмета с зазором меньше 64 за ним (−1 — нет): переуплотнённая пара.
    pub fsq: i32,
    /// Линия ниже по течению (куда выезжают предметы) и линия, впадающая сзади.
    pub out: Option<u32>,
    pub inp: Option<u32>,
    /// Стоит ли в списке активных своей группы.
    pub active: bool,
    /// Тик последнего обновления (защита от двойного обновления за тик).
    pub last_tick: Option<i64>,
    /// Спящие манипуляторы (индексы в `World::inserters`), которые ждут на этой линии предмета
    /// (`pick`) и места (`drop`); порядок — порядок засыпания (см. `wake`).
    pub pick: Vec<u32>,
    pub drop: Vec<u32>,
}

/// Группа линий: одна полоса одной цепочки лент. Список активных — порядок обновления.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct LineGroup {
    pub active: Vec<u32>,
    /// Место группы в `World::active_groups` (`NONE` — группы там нет).
    pub slot: u32,
}

impl Default for LineGroup {
    fn default() -> Self {
        LineGroup { active: Vec::new(), slot: NONE }
    }
}

/// Заявка на перенарезку цепочки.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Remerge {
    pub tick: i64,
    /// Порядковый номер: заявки одного тика исполняются в порядке постановки.
    pub seq: u64,
    /// Только проверить давность отметок (нарезка меняется, лишь если что-то выдохлось).
    pub decay_only: bool,
    /// Головная (самая нижняя по течению) линия цепочки.
    pub line: u32,
}

/// Хэш цепочки: по координатам последнего по течению тайла (1/256 тайла) и номеру полосы
/// (1 — левая, 2 — правая). Задаёт задержку перенарезки. Это общеизвестный 32-битный
/// целочисленный хэш Боба Дженкинса в композиции ниже; совпадение срока перенарезки с игрой
/// подтверждено пробами `cal-*` с точностью до тика (13 координат).
pub fn line_hash(x: Fx, y: Fx, lane: u8) -> u32 {
    fn jenkins32(mut a: u32) -> u32 {
        a = a.wrapping_add(0x7ED5_5D16).wrapping_add(a << 12);
        a = a ^ 0xC761_C23C ^ (a >> 19);
        a = a.wrapping_add(0x1656_67B1).wrapping_add(a << 5);
        a = a.wrapping_add(0xD3A2_646C) ^ (a << 9);
        a = a.wrapping_add(0xFD70_46C5).wrapping_add(a << 3);
        a ^ 0xB55A_4F09 ^ (a >> 16)
    }
    jenkins32(jenkins32(jenkins32(x as u32) ^ (y as u32)).wrapping_add(lane as u32 + 1))
}

/// Ход предмета по линии со скоростью `out_speed` после переезда с линии со скоростью `speed`
/// при остатке хода `rem`: остаток пересчитывается пропорционально скоростям, дробь
/// отбрасывается. Проверено по эталону для обычная ↔ быстрая (проба p12-mixed) и стыков
/// обычная ↔ турбо (набор j, `j-mix-yty`); для остальных пар пробами не проверено.
/// Эквивалентно вещественному делению с отбрасыванием дроби; для скоростей, кратных 8, равно
/// целочисленному (тест `transfer_advance_matches_double`).
pub fn transfer_advance(speed: Fx, out_speed: Fx, rem: Fx) -> Fx {
    if out_speed == speed {
        rem
    } else {
        (out_speed as i64 * rem as i64 / speed as i64) as Fx
    }
}

impl Line {
    pub fn new(lane: u8, group: u32, speed: Fx, len: Fx) -> Line {
        Line {
            alive: true,
            lane,
            group,
            speed,
            len,
            tiles: Vec::new(),
            items: Vec::new(),
            fg: 0,
            bg: 0,
            gi: -1,
            fsq: -1,
            out: None,
            inp: None,
            active: false,
            last_tick: None,
            pick: Vec::new(),
            drop: Vec::new(),
        }
    }

    /// Есть ли на линии место, куда можно положить сбоку. Нет — только когда линия сжата
    /// целиком: нет зазоров больше 64, передний и хвостовой зазоры не больше `скорость + 64`, а
    /// хвостовой зазор вместе с передним зазором линии сзади (`inp_fg`; линии сзади нет — 0) не
    /// больше 64. От этого зависит, будят ли манипулятор, упёршийся в эту линию с предметом.
    pub fn has_room(&self, inp_fg: Option<Fx>) -> bool {
        if self.items.is_empty() {
            return true;
        }
        let lim = self.speed + ITEM_SPACING;
        self.fg > lim || self.bg > lim || self.gi != -1 || self.bg + inp_fg.unwrap_or(0) > ITEM_SPACING
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Предметы с позициями (расстояние до переднего конца линии), от переднего к заднему.
    pub fn positions(&self) -> impl Iterator<Item = (usize, Fx, &LineItem)> {
        let mut p = self.fg;
        self.items.iter().enumerate().map(move |(i, it)| {
            let here = p;
            p += it.gap;
            (i, here, it)
        })
    }

    /// Предметы с позицией в `[lo, hi)`.
    pub fn range(&self, lo: Fx, hi: Fx) -> impl Iterator<Item = (usize, Fx, &LineItem)> {
        self.positions().skip_while(move |&(_, p, _)| p < lo).take_while(move |&(_, p, _)| p < hi)
    }

    fn clear(&mut self) {
        self.items.clear();
        self.fg = 0;
        self.bg = 0;
        self.gi = -1;
        self.fsq = -1;
    }

    /// `gi` вперёд до первого зазора больше 64 (начиная с текущего значения).
    fn adv_gi(&mut self) {
        let n = self.items.len() as i32 - 1;
        let mut v = self.gi;
        while v >= 0 && v < n {
            if self.items[v as usize].gap > ITEM_SPACING {
                self.gi = v;
                return;
            }
            v += 1;
        }
        self.gi = -1;
    }

    /// `fsq` вперёд до первого зазора меньше 64 (начиная с текущего значения).
    fn adv_fsq(&mut self) {
        let n = self.items.len() as i32 - 1;
        let mut v = self.fsq;
        while v >= 0 && v < n {
            if self.items[v as usize].gap < ITEM_SPACING {
                self.fsq = v;
                return;
            }
            v += 1;
        }
        self.fsq = -1;
    }

    /// Первый индекс `>= from` (до предпоследнего предмета) с зазором больше 64, иначе −1.
    fn next_wide(&self, from: i32) -> i32 {
        let n = self.items.len() as i32 - 1;
        let mut v = from;
        while v < n {
            if self.items[v as usize].gap > ITEM_SPACING {
                return v;
            }
            v += 1;
        }
        -1
    }

    /// Кэши `gi` / `fsq` равны своим определениям (проверка в отладочной сборке).
    fn caches_ok(&self) -> bool {
        let n = self.items.len().saturating_sub(1);
        let gi = self.items[..n].iter().position(|it| it.gap > ITEM_SPACING).map_or(-1, |i| i as i32);
        let fsq = self.items[..n].iter().position(|it| it.gap < ITEM_SPACING).map_or(-1, |i| i as i32);
        gi == self.gi && fsq == self.fsq && self.items.last().map_or(true, |it| it.gap == 0)
    }

    /// Расправление переуплотнённых пар после того, как часть линии проехала `d`.
    ///
    /// Зазор меньше 64 растёт на `c = min(d, 64 − зазор)`: всё, что за парой, отодвигается назад
    /// относительно того, что перед ней. «Платит» за это первый зазор больше 64 позади пары
    /// (то, что за ним, остаётся на месте); если такого нет — хвостовой зазор линии.
    ///
    /// Следствия (проверено по эталону: пробы перезапуска очереди — 204 события отъезда,
    /// фаззер; docs/mechanics/belts.md, «Переуплотнённые пары»):
    /// - линия едет: передний предмет пары едет, задний стоит, пока зазор не станет 64;
    /// - перед линии упёрт, а позади пары в этой же линии закрывается зазор больше 64 — задний
    ///   предмет пары **едет назад** на `c`;
    /// - в линии ничего не движется — пара стоит сколько угодно.
    fn relax_pairs(&mut self, mut d: Fx) {
        while self.fsq != -1 {
            if d < 1 {
                return;
            }
            let f = self.fsq;
            let need = ITEM_SPACING - self.items[f as usize].gap;
            let mut c = need.min(d);
            d -= c;
            self.items[f as usize].gap += c;
            let mut w = self.gi;
            if w != -1 {
                if w < f {
                    w = self.next_wide(f);
                }
                while c >= 1 && w != -1 {
                    let take = (self.items[w as usize].gap - ITEM_SPACING).min(c);
                    self.items[w as usize].gap -= take;
                    c -= take;
                    let own = self.gi == w;
                    w = self.next_wide(w);
                    if own {
                        self.gi = w;
                    }
                }
            }
            if self.items[f as usize].gap == ITEM_SPACING {
                self.adv_fsq();
            }
            self.bg -= c;
        }
    }

    /// Въезд предмета с линии выше по течению; `adv` — ход, который ему осталось пройти по этой
    /// линии. Предмет встаёт не ближе 64 к последнему; если хвостовой зазор не больше 64 — отказ.
    fn insert_at_back(&mut self, mut adv: Fx, item: ItemId, uid: u64) -> bool {
        if adv == 0 {
            adv = self.speed;
        }
        let e = adv.max(1);
        let len = self.len;
        let p;
        if self.items.is_empty() {
            p = (len - e).max(0);
            self.fg = p;
        } else {
            if self.bg <= ITEM_SPACING {
                return false;
            }
            let last = len - self.bg;
            p = (len - e).max(last + ITEM_SPACING);
            let n = self.items.len();
            self.items[n - 1].gap = p - last;
            if p - last > ITEM_SPACING && self.gi == -1 {
                self.gi = n as i32 - 1;
            }
        }
        self.items.push(LineItem { gap: 0, item, uid });
        self.bg = len - p;
        debug_assert!(self.caches_ok());
        true
    }

    /// Кладка манипулятором в точку `pos` (расстояние до переднего конца линии, `0 <= pos < len`);
    /// `adv` — ход, который положенный предмет проходит в этом же тике (скорость ленты).
    /// `out_bg` — хвостовой зазор линии ниже по течению, `inp_fg` — передний зазор линии,
    /// впадающей сзади (`None` — такой линии нет; у пустой линии оба зазора равны 0).
    ///
    /// Правило (проверено по эталону — пробы `t*`, `tail-*`, `edge-*`, `cal-*`, фаззер;
    /// docs/mechanics/belts.md, «Кладка»):
    /// - пустая линия: предмет встаёт на `max(pos − adv, 0)` — за передний конец линии не выезжает;
    /// - перед первым предметом: то же; зазор до первого может оказаться меньше 64. Отказ, если
    ///   точка ближе 64 к первому предмету, ниже по течению есть линия и `out_bg + fg <= 64`;
    /// - ровно на последнем предмете — отказ; за последним: предмет встаёт на
    ///   `max(pos − adv, последний + 64)`, но не дальше `len − 1`. Отказ, если точка ближе 64 к
    ///   последнему, сзади есть линия и `bg + inp_fg <= 64`;
    /// - между предметами: передний отодвигает точку вставки на `передний + 64`; задний должен
    ///   стоять строго дальше точки вставки; предмет встаёт на `max(точка − adv, передний + 64)`.
    pub fn try_insert(&mut self, pos: Fx, adv: Fx, out_bg: Option<Fx>, inp_fg: Option<Fx>, item: ItemId, uid: u64) -> bool {
        let len = self.len;
        if pos < 0 || pos >= len {
            return false;
        }
        let new = LineItem { gap: 0, item, uid };
        if self.items.is_empty() {
            let p = (pos - adv).max(0);
            self.items.push(new);
            self.fg = p;
            self.bg = len - p;
            return true;
        }
        let fg = self.fg;
        if pos < fg {
            if pos + ITEM_SPACING >= fg && out_bg.is_some_and(|b| b + fg <= ITEM_SPACING) {
                return false;
            }
            let p = (pos - adv).max(0);
            let gap = fg - p;
            self.items.insert(0, LineItem { gap, ..new });
            if gap > ITEM_SPACING {
                self.gi = 0;
            } else if self.gi != -1 {
                self.gi += 1;
            }
            if gap < ITEM_SPACING {
                self.fsq = 0;
            } else if self.fsq != -1 {
                self.fsq += 1;
            }
            self.fg = p;
            debug_assert!(self.caches_ok());
            return true;
        }
        let n = self.items.len();
        let last = len - self.bg;
        if pos == last {
            return false;
        }
        if pos > last {
            let lim = last + ITEM_SPACING;
            if pos < lim && inp_fg.is_some_and(|f| self.bg + f <= ITEM_SPACING) {
                return false;
            }
            let p = (pos - adv).max(lim).min(len - 1);
            let gap = p - last;
            self.items[n - 1].gap = gap;
            if gap > ITEM_SPACING {
                if self.gi == -1 {
                    self.gi = n as i32 - 1;
                }
            } else if gap < ITEM_SPACING && self.fsq == -1 {
                self.fsq = n as i32 - 1;
            }
            self.items.push(new);
            self.bg = len - p;
            debug_assert!(self.caches_ok());
            return true;
        }
        // между двумя предметами: пары (i, i + 1)
        let mut cur = fg;
        let mut i = 0usize;
        let mut at = pos;
        let nxt = loop {
            let nxt = cur + self.items[i].gap;
            if at < cur + ITEM_SPACING {
                if at == cur {
                    return false;
                }
                let a = cur + ITEM_SPACING;
                if a > len || a > nxt {
                    return false;
                }
                at = a;
            }
            if at < nxt {
                break nxt;
            }
            cur = nxt;
            i += 1;
            if i == n - 1 {
                return false;
            }
        };
        let q = (at - adv).max(cur + ITEM_SPACING);
        self.items[i].gap = q - cur;
        let g2 = nxt - q;
        self.items.insert(i + 1, LineItem { gap: g2, ..new });
        let i = i as i32;
        if g2 < ITEM_SPACING {
            if self.fsq == -1 || self.fsq > i + 1 {
                self.fsq = i + 1;
            }
        } else if self.fsq != -1 && self.fsq > i {
            self.fsq += 1;
        }
        if self.gi != -1 {
            if self.gi > i {
                self.gi += 1;
            } else {
                self.adv_gi();
            }
        }
        debug_assert!(self.caches_ok());
        true
    }

    /// Снятие предмета `idx` (зазоры соседей складываются).
    pub fn remove(&mut self, idx: usize) -> LineItem {
        let n = self.items.len();
        if n == 1 {
            let it = self.items.pop().expect("n == 1");
            self.clear();
            return it;
        }
        let it;
        if idx == 0 {
            self.fg += self.items[0].gap;
            it = self.items.remove(0);
            if self.gi == 0 {
                self.adv_gi();
            } else if self.gi != -1 {
                self.gi -= 1;
            }
            if self.fsq == 0 {
                self.adv_fsq();
            } else if self.fsq != -1 {
                self.fsq -= 1;
            }
        } else if idx == n - 1 {
            it = self.items.pop().expect("n > 1");
            let m = self.items.len() - 1;
            self.bg += self.items[m].gap;
            self.items[m].gap = 0;
            if self.gi == m as i32 {
                self.gi = -1;
            }
            if self.fsq == m as i32 {
                self.fsq = -1;
            }
        } else {
            self.items[idx - 1].gap += self.items[idx].gap;
            let i = idx as i32;
            if self.items[idx - 1].gap > ITEM_SPACING {
                if self.gi == -1 || self.gi >= i {
                    self.gi = i - 1;
                }
            } else if self.gi != -1 && self.gi >= i {
                self.gi -= 1;
            }
            it = self.items.remove(idx);
            let f = self.fsq;
            if f != -1 {
                if i == f || i == f + 1 {
                    self.adv_fsq();
                } else if f > i {
                    self.fsq = f - 1;
                }
            }
        }
        debug_assert!(self.caches_ok());
        it
    }
}

impl World {
    // ---------- список активных ----------

    /// Разбудить линию: в **конец** списка активных группы (она ещё обновится в этом же проходе).
    /// Так будят: снятие предмета (саму линию и линию сзади), движение линии ниже по течению,
    /// разрез и слияние.
    pub(super) fn activate_back(&mut self, li: u32) {
        let l = &mut self.lines[li as usize];
        if l.speed != 0 && !l.items.is_empty() && !l.active {
            l.active = true;
            let g = l.group;
            self.line_groups[g as usize].active.push(li);
            self.list_group(g);
        }
    }

    /// Группа, в которой появилась активная линия, встаёт в **конец** списка активных групп
    /// (если её там нет).
    fn list_group(&mut self, g: u32) {
        let grp = &mut self.line_groups[g as usize];
        if grp.slot == NONE {
            grp.slot = self.active_groups.len() as u32;
            self.active_groups.push(g);
        }
    }

    /// Есть ли место на линии `li` (см. [`Line::has_room`]).
    pub(super) fn line_has_room(&self, li: u32) -> bool {
        let l = &self.lines[li as usize];
        l.has_room(l.inp.map(|i| self.lines[i as usize].fg))
    }

    /// Разбудить линию: в **начало** списка активных группы (в этом проходе она уже не
    /// обновится, если проход идёт). Так будит вставка предмета — манипулятором или переездом с
    /// линии выше по течению.
    pub(super) fn activate_front(&mut self, li: u32) {
        let l = &mut self.lines[li as usize];
        if l.speed != 0 && !l.items.is_empty() && !l.active {
            l.active = true;
            let g = l.group;
            self.line_groups[g as usize].active.insert(0, li);
            if let Some((cg, it)) = &mut self.line_cursor {
                if *cg == g {
                    *it += 1;
                }
            }
            self.list_group(g);
        }
    }

    fn deactivate(&mut self, li: u32) {
        let l = &mut self.lines[li as usize];
        if !l.active {
            return;
        }
        l.active = false;
        let g = l.group;
        let act = &mut self.line_groups[g as usize].active;
        let i = act.iter().position(|&x| x == li).expect("активная линия есть в списке своей группы");
        act.remove(i);
        if let Some((cg, it)) = &mut self.line_cursor {
            if *cg == g && (i as u32) < *it {
                *it -= 1;
            }
        }
    }

    // ---------- движение ----------

    /// Обновление лент за тик `now`: заявки на перенарезку, затем активные линии по группам,
    /// затем отложенные действия — будильники спящих манипуляторов и выбывание остановившихся
    /// групп.
    ///
    /// Будильники из обновления лент не звонят сразу: они копятся (`line_alarms`) и исполняются
    /// после движения всех лент — по группам в порядке списка активных групп (`active_groups`),
    /// внутри группы — в порядке обновления линий. Затем группа без активных линий выбывает из
    /// списка, на её место встаёт последняя. Отсюда порядок манипуляторов, разбуженных в одном
    /// тике с разных лент (пробы `ins-groups-solo-*`, docs/mechanics/inserters.md).
    pub fn step_belts(&mut self, now: i64) {
        self.prepare_lines(now);
        debug_assert!(self.line_alarms.is_empty());
        // Порядок обхода — список активных групп на начало движения. Группы между собой не
        // связаны, поэтому от порядка обхода зависит только порядок отложенных будильников, а он
        // ниже пересортировывается по месту группы в списке.
        let mut order = std::mem::take(&mut self.group_scratch);
        order.clear();
        order.extend_from_slice(&self.active_groups);
        for &g in &order {
            // Список может расти и уменьшаться на ходу: вставка в начало сдвигает курсор
            // (вставленная линия в этом тике не посещается), добавленная в конец — посещается.
            self.line_cursor = Some((g, 0));
            while let Some((_, it)) = self.line_cursor {
                let Some(&li) = self.line_groups[g as usize].active.get(it as usize) else { break };
                self.line_cursor = Some((g, it + 1));
                self.update_line(li, now);
            }
        }
        self.line_cursor = None;
        // отложенные будильники: по месту группы в списке активных групп (сортировка устойчива)
        let mut alarms = std::mem::take(&mut self.line_alarms);
        alarms.sort_by_key(|&(g, _)| self.line_groups[g as usize].slot);
        for &(_, r) in &alarms {
            self.alarm(r);
        }
        alarms.clear();
        self.line_alarms = alarms; // память списка переиспользуется
        // выбывание групп без активных линий — в том же порядке; на место выбывшей встаёт последняя
        for &g in &order {
            if self.line_groups[g as usize].active.is_empty() {
                let slot = self.line_groups[g as usize].slot as usize;
                self.line_groups[g as usize].slot = NONE;
                self.active_groups.swap_remove(slot);
                if let Some(&moved) = self.active_groups.get(slot) {
                    self.line_groups[moved as usize].slot = slot as u32;
                }
            }
        }
        order.clear();
        self.group_scratch = order;
    }

    /// Отложить будильник списка `r` (линия `li` — для номера группы) до конца обновления лент.
    fn defer_alarm(&mut self, li: u32, r: WakeRef) {
        let g = self.lines[li as usize].group;
        self.line_alarms.push((g, r));
    }

    /// Обновление одной линии. Сначала — линия ниже по течению, **если она активна**; если та
    /// спит, эта линия упирается в её старый хвост. Сдвинулась — будит линию сзади; нет — засыпает.
    ///
    /// Отсюда «хвост очереди трогается на тик позже» (проверено по эталону: 1827 событий в
    /// пробах перезапуска, `p20-restart-min`, `compressed-restart-*`): верхняя линия активна и
    /// стоит в списке раньше, соседняя нижняя спит, и её в этом же тике будит ещё более нижняя.
    fn update_line(&mut self, li: u32, now: i64) {
        if self.lines[li as usize].last_tick == Some(now) {
            return;
        }
        // Цепочка «эта линия, затем активные линии ниже по течению, ещё не обновлённые в этом
        // тике» собирается до любого хода (как при обходе вглубь), затем линии ходят снизу вверх.
        // Обход итеративный: глубина не ограничена и стек не растёт.
        let mut chain = std::mem::take(&mut self.chain_scratch);
        chain.clear();
        self.lines[li as usize].last_tick = Some(now);
        chain.push(li);
        let mut cur = li;
        while let Some(o) = self.lines[cur as usize].out {
            let ol = &mut self.lines[o as usize];
            if !ol.active || ol.last_tick == Some(now) {
                break;
            }
            ol.last_tick = Some(now);
            chain.push(o);
            cur = o;
        }
        for &l in chain.iter().rev() {
            self.finish_line(l);
        }
        chain.clear();
        self.chain_scratch = chain;
    }

    /// Ход линии `li` в её обновлении (линия ниже по течению уже обновлена, если была активна).
    fn finish_line(&mut self, li: u32) {
        if self.move_line(li) {
            if let Some(i) = self.lines[li as usize].inp {
                self.activate_back(i);
            }
            // линия сдвинулась: будит ждущих места (если место есть) и ждущих предмета — отложенно
            if !self.lines[li as usize].drop.is_empty() && self.line_has_room(li) {
                self.defer_alarm(li, WakeRef::Drop(li));
            }
            if !self.lines[li as usize].pick.is_empty() {
                self.defer_alarm(li, WakeRef::Pick(li));
            }
        } else {
            self.deactivate(li);
        }
    }

    /// Ход линии за тик. Возвращает «сдвинулось ли что-нибудь».
    ///
    /// 1. Вся линия едет как целое, пока первый предмет не дошёл до переднего конца и не подошёл
    ///    ближе 64 к последнему предмету линии ниже по течению (её хвостовой зазор берётся как
    ///    есть — старый, если та в этом тике ещё не обновлялась).
    /// 2. Первый предмет у переднего конца — переезд в линию ниже по течению (не больше одного
    ///    за тик при ходе до 64); остаток хода пересчитывается по скоростям.
    /// 3. Перед упёрт — закрывается первый зазор больше 64: всё, что за ним, подъезжает.
    fn move_line(&mut self, li: u32) -> bool {
        let li = li as usize;
        if self.lines[li].items.is_empty() {
            return false;
        }
        let dist = self.lines[li].speed;
        let mut rem = dist;
        let out = self.lines[li].out;
        let mut can_out = out.is_some_and(|o| self.lines[o as usize].speed != 0);
        while rem > 0 {
            let fg = self.lines[li].fg;
            let mut m = rem.min(fg);
            if let Some(o) = out {
                let ol = &self.lines[o as usize];
                if !ol.items.is_empty() {
                    m = m.min(fg - ITEM_SPACING + ol.bg);
                }
            }
            if m > 0 {
                let l = &mut self.lines[li];
                l.fg -= m;
                l.bg += m;
                rem -= m;
                l.relax_pairs(m);
                if rem == 0 {
                    break;
                }
                if l.fg > 0 {
                    continue;
                }
            }
            if let (true, Some(o)) = (can_out, out) {
                let more = rem > ITEM_SPACING;
                let adv = transfer_advance(self.lines[li].speed, self.lines[o as usize].speed, rem);
                let (item, uid) = (self.lines[li].items[0].item, self.lines[li].items[0].uid);
                if self.lines[o as usize].insert_at_back(adv, item, uid) {
                    self.activate_front(o);
                    // на линию въехал предмет: будит ждущих на ней предмета — отложенно
                    if !self.lines[o as usize].pick.is_empty() {
                        self.defer_alarm(o, WakeRef::Pick(o));
                    }
                    let l = &mut self.lines[li];
                    if l.items.len() == 1 {
                        l.clear();
                        return true;
                    }
                    l.fg += l.items[0].gap;
                    l.items.remove(0);
                    if l.gi == 0 {
                        l.adv_gi();
                    } else if l.gi != -1 {
                        l.gi -= 1;
                    }
                    if l.fsq == 0 {
                        l.adv_fsq();
                    } else if l.fsq != -1 {
                        l.fsq -= 1;
                    }
                    can_out = more;
                    continue; // переезд остаток хода не уменьшает
                }
            }
            let l = &mut self.lines[li];
            if l.gi == -1 {
                break;
            }
            let gi = l.gi as usize;
            let r = (l.items[gi].gap - ITEM_SPACING).min(rem);
            l.items[gi].gap -= r;
            if l.items[gi].gap <= ITEM_SPACING {
                l.adv_gi();
            }
            l.bg += r;
            rem -= r;
            l.relax_pairs(r);
        }
        debug_assert!(self.lines[li].caches_ok());
        rem < dist
    }

    // ---------- манипуляторы ----------

    /// Кладка манипулятором на полосу `lane` тайла `b` в точку `in_tile` (0..=255 от переднего
    /// края тайла). Попытка (и неудачная тоже) отмечает взаимодействие с тайлом.
    pub fn belt_insert(&mut self, b: usize, lane: usize, in_tile: Fx, item: ItemId, uid: u64, now: i64) -> bool {
        self.touch(b, lane, now);
        let tl = &self.belts[b].lanes[lane];
        let (li, pos) = (tl.line, tl.start + in_tile);
        let l = &self.lines[li as usize];
        let out_bg = l.out.map(|o| self.lines[o as usize].bg);
        let inp_fg = l.inp.map(|i| self.lines[i as usize].fg);
        let adv = l.speed;
        let ok = self.lines[li as usize].try_insert(pos, adv, out_bg, inp_fg, item, uid);
        if ok {
            self.activate_front(li);
            // положенный предмет сразу будит манипуляторы, ждущие на этой линии предмета
            self.alarm(WakeRef::Pick(li));
        }
        ok
    }

    /// Снятие предмета манипулятором: будит линию и линию сзади (в конец списка активных) и,
    /// если на линии есть место, — манипуляторы, ждущие на ней места (сразу).
    pub fn belt_remove(&mut self, li: u32, idx: usize) -> LineItem {
        let it = self.lines[li as usize].remove(idx);
        self.activate_back(li);
        if let Some(i) = self.lines[li as usize].inp {
            self.activate_back(i);
        }
        if self.line_has_room(li) {
            self.alarm(WakeRef::Drop(li));
        }
        it
    }

    /// Отметка взаимодействия манипулятора с полосой тайла. Первая отметка на тайле ставит
    /// заявку на перенарезку цепочки.
    pub fn touch(&mut self, b: usize, lane: usize, now: i64) {
        if self.belts[b].lanes[lane].touched.is_none() {
            let li = self.belts[b].lanes[lane].line;
            self.schedule_remerge(li, now);
        }
        self.belts[b].lanes[lane].touched = Some(now);
    }

    // ---------- перенарезка ----------

    fn chain_head(&self, mut li: u32) -> u32 {
        while let Some(o) = self.lines[li as usize].out {
            li = o;
        }
        li
    }

    fn chain_hash(&self, head: u32) -> u32 {
        let l = &self.lines[head as usize];
        let t = &self.belts[l.tiles[0] as usize];
        line_hash(t.pos.0, t.pos.1, l.lane)
    }

    fn request_of(&self, line: u32) -> Option<usize> {
        self.remerges.iter().position(|r| r.line == line)
    }

    fn push_request(&mut self, tick: i64, decay_only: bool, line: u32) {
        let seq = self.remerge_seq;
        self.remerge_seq += 1;
        self.remerges.push(Remerge { tick, seq, decay_only, line });
    }

    /// Обычная заявка: перенарезка через `hash mod 600` тиков. Если у цепочки уже есть обычная
    /// заявка, новая не ставится; заявка «только проверка давности» заменяется.
    pub(super) fn schedule_remerge(&mut self, line: u32, now: i64) {
        let h = self.chain_head(line);
        if let Some(i) = self.request_of(h) {
            if !self.remerges[i].decay_only {
                return;
            }
            self.remerges.remove(i);
        }
        let delay = (self.chain_hash(h) % REMERGE_SPREAD) as i64;
        self.push_request(now + delay, false, h);
    }

    fn schedule_decay(&mut self, h: u32, now: i64) {
        if self.request_of(h).is_none() {
            let delay = TOUCH_TTL + (self.chain_hash(h) % DECAY_SPREAD) as i64;
            self.push_request(now + delay, true, h);
        }
    }

    /// Заявки со сроком не позже `now`, по возрастанию (тик, номер). До движения лент этого тика.
    pub(super) fn prepare_lines(&mut self, now: i64) {
        while let Some(i) = (0..self.remerges.len()).min_by_key(|&i| (self.remerges[i].tick, self.remerges[i].seq)) {
            if self.remerges[i].tick > now {
                break;
            }
            let (line, decay_only) = (self.remerges[i].line, self.remerges[i].decay_only);
            self.remerge(line, decay_only, now);
        }
    }

    fn can_merge_input(&self, li: u32) -> bool {
        let l = &self.lines[li as usize];
        l.speed != 0 && l.inp.is_some_and(|i| i != li && self.lines[i as usize].speed == l.speed)
    }

    /// Перенарезка цепочки, к которой относится `line`.
    ///
    /// Идём по тайлам от хвоста цепочки к голове с запасом длины: новая линия начинается, когда
    /// тайл не влезает в запас или сзади лента другой скорости. Тайл с отметкой взаимодействия
    /// урезает запас до «этот тайл и ещё два», то есть линия кончается через два тайла ниже по
    /// течению от первого в ней тайла с отметкой. Отметки старше 3000 тиков стираются.
    ///
    /// Проверено по эталону: пробы перезапуска очереди (234 сцены), `cal-*`, `sk-mid`,
    /// `decay-*`, `build-*` (docs/mechanics/belts.md).
    pub(super) fn remerge(&mut self, line: u32, decay_only: bool, now: i64) {
        let head = self.chain_head(line);
        let lane = self.lines[head as usize].lane as usize;
        let (mut fresh, mut decayed) = (false, false);
        // тайлы цепочки от головы к хвосту
        let mut tiles: Vec<u32> = Vec::new();
        let mut cur = Some(head);
        while let Some(li) = cur {
            if let Some(i) = self.request_of(li) {
                self.remerges.remove(i);
            }
            for k in 0..self.lines[li as usize].tiles.len() {
                let b = self.lines[li as usize].tiles[k];
                let tl = &mut self.belts[b as usize].lanes[lane];
                if let Some(t) = tl.touched {
                    if now - t < TOUCH_TTL {
                        fresh = true;
                    } else {
                        tl.touched = None;
                        decayed = true;
                    }
                }
                tiles.push(b);
            }
            cur = self.lines[li as usize].inp;
        }
        if decay_only && !decayed {
            if fresh {
                self.schedule_decay(head, now);
            }
            return;
        }
        // сегменты (верхний тайл, нижний тайл), от хвоста к голове
        let mut segs: Vec<(u32, u32)> = Vec::new();
        let mut budget = LINE_BUDGET;
        for &b in tiles.iter().rev() {
            let tl = &self.belts[b as usize].lanes[lane];
            let li = tl.line;
            let back_of_line = self.lines[li as usize].tiles.last() == Some(&b);
            if segs.is_empty() || TILE_LANE_LEN > budget || (back_of_line && !self.can_merge_input(li)) {
                segs.push((b, b));
                budget = LINE_BUDGET;
            }
            if tl.touched.is_some() {
                budget = budget.min(TOUCHED_BUDGET + TILE_LANE_LEN / 2);
            }
            segs.last_mut().expect("сегмент начат").1 = b;
            budget -= TILE_LANE_LEN;
        }
        // разрезы: по верхнему краю каждого сегмента
        for &(first, _) in &segs {
            let li = self.belts[first as usize].lanes[lane].line;
            let lt = &self.lines[li as usize].tiles;
            if lt.last() != Some(&first) {
                let i = lt.iter().position(|&t| t == first).expect("тайл есть в своей линии");
                self.unmerge(li, i + 1);
            }
        }
        // слияния: нижняя линия сегмента поглощает линии сзади до верхнего тайла; манипуляторы,
        // спавшие на слитых линиях, собираются и после всех слияний будятся
        let mut woken: Vec<u32> = Vec::new();
        for &(first, last) in &segs {
            let li = self.belts[last as usize].lanes[lane].line;
            while self.belts[first as usize].lanes[lane].line != li {
                let merged = self.merge_input(li, &mut woken);
                assert!(merged, "сегмент нарезки: линии разной скорости в одном сегменте");
            }
        }
        for i in woken {
            self.wake(i as usize);
        }
        if fresh {
            self.schedule_decay(head, now);
        }
    }

    fn alloc_line(&mut self, line: Line) -> u32 {
        match self.free_lines.pop() {
            Some(i) => {
                self.lines[i as usize] = line;
                i
            }
            None => {
                self.lines.push(line);
                u32::try_from(self.lines.len() - 1).expect("число линий помещается в u32")
            }
        }
    }

    /// Разрез линии `s`: первые `keep` тайлов (от переднего) остаются, остальные уходят в новую
    /// линию выше по течению. Предмет ровно на границе уходит в верхнюю линию. Новая линия (и
    /// линия за ней) будятся в конец списка активных; место `s` в списке не меняется.
    fn unmerge(&mut self, s: u32, keep: usize) {
        let lane = self.lines[s as usize].lane as usize;
        let cut = self.belts[self.lines[s as usize].tiles[keep] as usize].lanes[lane].start;
        let sl = &mut self.lines[s as usize];
        // k — сколько предметов остаётся; pos — позиция предмета k (или последнего, если ушли все)
        let n = sl.items.len();
        let (mut pos, mut k) = (sl.fg, 0usize);
        while k < n && pos < cut {
            pos += sl.items[k].gap;
            k += 1;
        }
        let mut nl = Line::new(sl.lane, sl.group, sl.speed, sl.len - cut);
        sl.len = cut;
        nl.tiles = sl.tiles.split_off(keep);
        nl.items = sl.items.split_off(k);
        if !nl.items.is_empty() {
            nl.bg = sl.bg;
            nl.fg = pos - cut;
            if sl.gi != -1 {
                if sl.gi >= k as i32 {
                    nl.gi = sl.gi - k as i32;
                } else {
                    nl.gi = 0;
                    nl.adv_gi();
                }
            }
            if sl.fsq != -1 {
                if sl.fsq >= k as i32 {
                    nl.fsq = sl.fsq - k as i32;
                } else {
                    nl.fsq = 0;
                    nl.adv_fsq();
                }
            }
        }
        if sl.items.is_empty() {
            sl.clear();
        } else {
            let m = sl.items.len() - 1;
            pos -= sl.items[m].gap;
            sl.bg = cut - pos;
            sl.items[m].gap = 0;
            if sl.gi >= m as i32 {
                sl.gi = -1;
            }
            if sl.fsq >= m as i32 {
                sl.fsq = -1;
            }
        }
        nl.inp = sl.inp;
        nl.out = Some(s);
        debug_assert!(sl.caches_ok() && nl.caches_ok());
        let n_inp = nl.inp;
        let moved = nl.tiles.clone();
        let ni = self.alloc_line(nl);
        self.lines[s as usize].inp = Some(ni);
        if let Some(i) = n_inp {
            self.lines[i as usize].out = Some(ni);
        }
        for b in moved {
            let tl = &mut self.belts[b as usize].lanes[lane];
            tl.line = ni;
            tl.start -= cut;
        }
        self.activate_back(ni);
        if let Some(i) = n_inp {
            self.activate_back(i);
        }
        // Спящие старой линии переписываются на линии своих тайлов (не просыпаются): сначала
        // ждущие места, потом ждущие предмета, каждый список — в своём порядке.
        for list in [WakeRef::Drop(s), WakeRef::Pick(s)] {
            let sleepers = self.wake_list(list).clone();
            for i in sleepers {
                self.reregister_belt(i as usize);
            }
        }
    }

    /// Линия `s` поглощает линию, впадающую сзади (той же скорости). Поглотившая будится в конец
    /// списка активных, поглощённая исчезает.
    ///
    /// Манипуляторы, спавшие на обеих линиях, выписываются из всех своих списков и собираются в
    /// `woken` (у поглотившей — ждущие места, потом ждущие предмета, затем то же у поглощённой;
    /// каждый список — с конца); будит их вызывающий.
    fn merge_input(&mut self, s: u32, woken: &mut Vec<u32>) -> bool {
        if !self.can_merge_input(s) {
            return false;
        }
        let i = self.lines[s as usize].inp.expect("can_merge_input");
        for list in [WakeRef::Drop(s), WakeRef::Pick(s), WakeRef::Drop(i), WakeRef::Pick(i)] {
            while let Some(m) = self.wake_list_mut(list).pop() {
                self.unregister_all(m as usize, Some(list));
                woken.push(m);
            }
        }
        self.deactivate(i);
        if let Some(r) = self.request_of(i) {
            self.remerges.remove(r);
        }
        let il = std::mem::replace(&mut self.lines[i as usize], Line::new(0, 0, 0, 0));
        self.lines[i as usize].alive = false;
        self.free_lines.push(i);
        let lane = il.lane as usize;
        let s_len = self.lines[s as usize].len;
        for &b in &il.tiles {
            let tl = &mut self.belts[b as usize].lanes[lane];
            tl.line = s;
            tl.start += s_len;
        }
        let sl = &mut self.lines[s as usize];
        sl.tiles.extend_from_slice(&il.tiles);
        if sl.items.is_empty() {
            if !il.items.is_empty() {
                sl.fg = il.fg + sl.len;
                sl.bg = il.bg;
                sl.gi = il.gi;
                sl.fsq = il.fsq;
                sl.items = il.items;
            }
        } else if il.items.is_empty() {
            sl.bg += il.len;
        } else {
            let g = il.fg + sl.bg;
            let cnt = sl.items.len() as i32;
            sl.items[cnt as usize - 1].gap = g;
            if g > ITEM_SPACING {
                if sl.gi == -1 {
                    sl.gi = cnt - 1;
                }
            } else if sl.gi == -1 && il.gi != -1 {
                sl.gi = il.gi + cnt;
            }
            if sl.fsq == -1 {
                if g < ITEM_SPACING {
                    sl.fsq = cnt - 1;
                } else if il.fsq != -1 {
                    sl.fsq = il.fsq + cnt;
                }
            }
            sl.bg = il.bg;
            sl.items.extend(il.items);
        }
        sl.inp = il.inp;
        sl.len += il.len;
        debug_assert!(sl.caches_ok());
        if let Some(u) = il.inp {
            self.lines[u as usize].out = Some(s);
        }
        self.activate_back(s);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(len: Fx, ps: &[Fx]) -> Line {
        let mut l = Line::new(0, 0, 8, len);
        for (i, &p) in ps.iter().enumerate() {
            let gap = ps.get(i + 1).map_or(0, |n| n - p);
            l.items.push(LineItem { gap, item: 0, uid: i as u64 });
        }
        if let (Some(&f), Some(&b)) = (ps.first(), ps.last()) {
            l.fg = f;
            l.bg = len - b;
        }
        let n = ps.len().saturating_sub(1);
        l.gi = l.items[..n].iter().position(|it| it.gap > ITEM_SPACING).map_or(-1, |i| i as i32);
        l.fsq = l.items[..n].iter().position(|it| it.gap < ITEM_SPACING).map_or(-1, |i| i as i32);
        l
    }

    fn pos(l: &Line) -> Vec<Fx> {
        l.positions().map(|(_, p, _)| p).collect()
    }

    /// Числа — из эталонов проб `tests/fidelity/probes/t*` и `p1-deadend`.
    #[test]
    fn insert_next_to_other_item() {
        // t3a: задний на d + 1 — кладёт, зазор 9 < 64
        let mut l = line(1024, &[136]);
        assert!(l.try_insert(135, 8, None, None, 0, 9));
        assert_eq!(pos(&l), vec![127, 136]);
        // t3b: задний ровно на d — ждёт
        let mut l = line(1024, &[136]);
        assert!(!l.try_insert(136, 8, None, None, 0, 9));
        // t1a: передний положен в этом тике (53), кладка на 128 → 104, зазор 51
        let mut l = line(1024, &[104]);
        assert!(l.try_insert(77, 24, None, None, 0, 9));
        assert_eq!(pos(&l), vec![53, 104]);
        // t1b: передний на 53 отодвигает кладку: 128 − 24 = 104 → 53 + 64 = 117
        let mut l = line(1024, &[53]);
        assert!(l.try_insert(128, 24, None, None, 0, 9));
        assert_eq!(pos(&l), vec![53, 117]);
        // t6b: тупик [0, 64], кладка отодвинута на 128, задний на 130 — кладёт
        let mut l = line(1024, &[0, 64, 130]);
        assert!(l.try_insert(77, 16, None, None, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 130]);
        // p1 / t6a: задний ровно на 128 — ждёт
        let mut l = line(1024, &[0, 64, 128]);
        assert!(!l.try_insert(77, 8, None, None, 0, 9));
    }

    /// Хвост линии из 1 тайла (len = 256).
    #[test]
    fn insert_at_line_tail() {
        // сверено (tail-b8-p200, фаззер seed 101): передний на 192, место по правилу на 256 → 255
        let mut l = line(256, &[0, 64, 128, 192]);
        assert!(l.try_insert(200, 8, None, None, 0, 9));
        assert_eq!(pos(&l), vec![0, 64, 128, 192, 255]);
        assert_eq!(l.fsq, 3);
        // то же, но сзади пустая линия — ждёт (tail-b16-p200-two)
        let mut l = line(256, &[0, 64, 128, 192]);
        assert!(!l.try_insert(200, 8, None, Some(0), 0, 9));
        // сзади линия, первый предмет которой не у края: bg + fg > 64 — кладёт
        let mut l = line(256, &[0, 64, 128, 192]);
        assert!(l.try_insert(200, 8, None, Some(1), 0, 9));
        // ровно на последнем предмете — ждёт
        let mut l = line(256, &[100]);
        assert!(!l.try_insert(100, 8, None, None, 0, 9));
        // край тайла: точка 255 на пустой линии → 247 (пробы edge-*)
        let mut l = line(256, &[]);
        assert!(l.try_insert(255, 8, None, None, 0, 9));
        assert_eq!(pos(&l), vec![247]);
    }

    /// Передний конец линии: предмет за него не выезжает (пробы `cal-*`, `build-*`).
    #[test]
    fn insert_at_line_front() {
        let mut l = line(256, &[]);
        assert!(l.try_insert(5, 8, Some(0), None, 0, 9));
        assert_eq!(pos(&l), vec![0]);
        // ближе 64 к первому предмету, ниже по течению линия вплотную — ждёт
        let mut l = line(256, &[60]);
        assert!(!l.try_insert(20, 8, Some(4), None, 0, 9));
        // ниже по течению линии нет — кладёт, зазор меньше 64
        let mut l = line(256, &[60]);
        assert!(l.try_insert(20, 8, None, None, 0, 9));
        assert_eq!(pos(&l), vec![12, 60]);
        assert_eq!(l.fsq, 0);
    }

    #[test]
    fn remove_merges_gaps() {
        let mut l = line(512, &[0, 64, 128, 300]);
        assert_eq!(l.gi, 2);
        l.remove(1);
        assert_eq!(pos(&l), vec![0, 128, 300]);
        assert_eq!((l.gi, l.fsq), (0, -1));
        l.remove(2);
        assert_eq!((pos(&l), l.bg, l.gi), (vec![0, 128], 384, 0));
        l.remove(0);
        l.remove(0);
        assert!(l.is_empty() && l.fg == 0 && l.bg == 0);
    }

    /// Отъезд назад: перед упёрт, пара [64, 100] переуплотнена, сзади закрывается зазор.
    #[test]
    fn relax_pairs_moves_back_only_while_gap_closes_behind() {
        let mut l = line(1024, &[0, 64, 100, 400]);
        assert_eq!((l.gi, l.fsq), (2, 1));
        // сжатие: закрылся зазор за предметом 2 на 8 → предмет 3 подъехал, предмет 2 отъехал назад
        l.items[2].gap -= 8;
        l.bg += 8;
        l.relax_pairs(8);
        assert_eq!(pos(&l), vec![0, 64, 108, 392]);
        // хвост платит, если зазоров больше 64 нет
        let mut l = line(1024, &[0, 64, 100]);
        l.relax_pairs(8);
        assert_eq!((pos(&l), l.bg), (vec![0, 64, 108], 1024 - 108));
    }

    #[test]
    fn hash_reference_values() {
        // лента с центром в (4.5, 0.5): правая полоса 515, левая 343 (пробы cal-*)
        assert_eq!(line_hash(1152, 128, 1) % 600, 515);
        assert_eq!(line_hash(1152, 128, 0) % 600, 343);
    }

    /// Целочисленный пересчёт хода между лентами равен вещественному делению с отбрасыванием
    /// дроби для скоростей, кратных 8.
    #[test]
    fn transfer_advance_matches_double() {
        for s in (8..=64).step_by(8) {
            for o in (8..=64).step_by(8) {
                for rem in 0..=s {
                    let d = ((o as f64 * (1.0 / 256.0)) / (s as f64 * (1.0 / 256.0)) * rem as f64) as Fx;
                    assert_eq!(transfer_advance(s, o, rem), if s == o { rem } else { d }, "{s} {o} {rem}");
                }
            }
        }
    }

    // ---------- мир: разрез и слияние линий, будильник, выбывание групп ----------

    use crate::world::layout::{build, Scene};
    use crate::world::proto::Prototypes;
    use crate::world::World;

    fn world(ents: serde_json::Value) -> World {
        let sc: Scene = serde_json::from_value(serde_json::json!({"name": "t", "ticks": 1, "entities": ents})).unwrap();
        build(&sc, Prototypes::bundled()).unwrap()
    }

    /// Ряд из `n` лент на восток в строке `y` (тайлы x = 0.5 … n − 0.5).
    fn belts_row(n: usize, y: f64) -> Vec<serde_json::Value> {
        (0..n).map(|k| serde_json::json!({"name": "transport-belt", "position": [k as f64 + 0.5, y], "direction": "east"})).collect()
    }

    /// Положить предметы на пустую линию: позиции от переднего конца.
    fn fill(l: &mut Line, ps: &[Fx]) {
        let t = line(l.len, ps);
        (l.items, l.fg, l.bg, l.gi, l.fsq) = (t.items, t.fg, t.bg, t.gi, t.fsq);
    }

    /// Разрез линии: предмет ровно на границе тайлов уходит в верхнюю (новую) линию; слияние
    /// обратно восстанавливает позиции, длину и тайлы.
    #[test]
    fn cut_on_tile_boundary_and_merge_back() {
        let mut w = world(serde_json::Value::Array(belts_row(4, 0.5)));
        // после прогрева цепочка из 4 лент — одна линия на полосу
        let li = w.belts[0].lanes[0].line;
        assert_eq!((w.lines[li as usize].tiles.len(), w.lines[li as usize].len), (4, 1024));
        fill(&mut w.lines[li as usize], &[100, 448, 512, 700]);
        let before = w.lines[li as usize].clone();
        // разрез после второго тайла от переднего конца: граница на 512
        w.unmerge(li, 2);
        let s = &w.lines[li as usize];
        let up = s.inp.expect("новая линия сзади");
        assert_eq!((s.len, pos(s), s.bg), (512, vec![100, 448], 64));
        let u = &w.lines[up as usize];
        assert_eq!((u.len, pos(u), u.fg, u.bg), (512, vec![0, 188], 0, 512 - 188));
        assert_eq!(u.out, Some(li));
        // тайлы верхней линии отсчитываются от её переднего конца
        for (k, &b) in u.tiles.iter().enumerate() {
            assert_eq!((w.belts[b as usize].lanes[0].line, w.belts[b as usize].lanes[0].start), (up, 256 * k as Fx));
        }
        // новая линия с предметами будится в конец списка активных своей группы
        assert!(u.active);
        assert_eq!(w.line_groups[u.group as usize].active.last(), Some(&up));
        // слияние обратно
        let mut woken = Vec::new();
        assert!(w.merge_input(li, &mut woken));
        assert!(woken.is_empty());
        assert!(!w.lines[up as usize].alive);
        let s = &w.lines[li as usize];
        assert_eq!((s.len, pos(s), s.fg, s.bg, s.gi, s.fsq), (before.len, pos(&before), before.fg, before.bg, before.gi, before.fsq));
        assert_eq!((&s.tiles, s.inp), (&before.tiles, None));
        for (k, b) in (0..4).rev().enumerate() {
            assert_eq!((w.belts[b].lanes[0].line, w.belts[b].lanes[0].start), (li, 256 * k as Fx));
        }
    }

    /// Слияние: зазор на стыке — хвостовой зазор нижней линии плюс передний зазор верхней;
    /// «первый зазор больше 64» и «первая переуплотнённая пара» находятся и через стык.
    #[test]
    fn merge_joins_gap_across_boundary() {
        let mut w = world(serde_json::Value::Array(belts_row(2, 0.5)));
        let li = w.belts[0].lanes[0].line;
        w.unmerge(li, 1);
        let up = w.lines[li as usize].inp.unwrap();
        // нижняя: предметы на 0 и 64 (сжаты, хвост 192); верхняя: на 0 и 40 (переуплотнены)
        fill(&mut w.lines[li as usize], &[0, 64]);
        fill(&mut w.lines[up as usize], &[0, 40]);
        let mut woken = Vec::new();
        assert!(w.merge_input(li, &mut woken));
        let s = &w.lines[li as usize];
        assert_eq!((pos(s), s.bg), (vec![0, 64, 256, 296], 512 - 296));
        // зазор 192 на стыке — первый больше 64; пара [256, 296] — первая переуплотнённая
        assert_eq!((s.gi, s.fsq), (1, 2));
        assert!(s.caches_ok());
    }

    /// Будильник списка спящих будит с конца: разбуженные встают в начало списка активных, поэтому
    /// идут в порядке засыпания и все — впереди тех, кто не спал; разбуженный выписывается из всех
    /// своих списков.
    #[test]
    fn alarm_wakes_from_back_into_front() {
        // четыре «сундук → манипулятор → сундук» без предметов: после прогрева все спят
        let mut ents = Vec::new();
        for k in 0..4 {
            let x = k as f64 + 0.5;
            ents.push(serde_json::json!({"name": "wooden-chest", "position": [x, -0.5]}));
            ents.push(serde_json::json!({"name": "inserter", "position": [x, 0.5], "direction": "north"}));
            ents.push(serde_json::json!({"name": "wooden-chest", "position": [x, 1.5]}));
        }
        let mut w = world(serde_json::Value::Array(ents));
        assert!(w.active_inserters().is_empty());
        for i in 0..4 {
            w.unregister_all(i, None);
        }
        // засыпали в порядке 2, 0, 1 (все ждут предмета в сундуке 0); 0 ждёт ещё и места в
        // сундуке 1; манипулятор 3 не спит
        for i in [2, 0, 1] {
            w.register(i, WakeRef::ChestOut(0));
        }
        w.register(0, WakeRef::ChestIn(1));
        w.wake(3);
        w.alarm(WakeRef::ChestOut(0));
        assert_eq!(w.active_inserters(), vec![2, 0, 1, 3]);
        assert!(w.chests[0].out_list.is_empty() && w.chests[1].in_list.is_empty());
        assert!((0..4).all(|i| w.inserters[i].regs.is_empty()));
    }

    /// Группа, в которой за тик не осталось активных линий, выбывает из списка активных групп;
    /// на её место встаёт последняя группа списка.
    #[test]
    fn stopped_group_leaves_active_groups() {
        let mut ents = belts_row(2, 0.5);
        ents.extend(belts_row(2, 2.5));
        ents.extend(belts_row(2, 4.5));
        let mut w = world(serde_json::Value::Array(ents));
        let (a, b, c) = (w.belts[0].lanes[0].line, w.belts[2].lanes[0].line, w.belts[4].lanes[0].line);
        // A: предмет у переднего конца, линии ниже по течению нет — стоит; B и C едут
        fill(&mut w.lines[a as usize], &[0]);
        fill(&mut w.lines[b as usize], &[300]);
        fill(&mut w.lines[c as usize], &[300]);
        for li in [a, b, c] {
            w.activate_back(li);
        }
        let g = |w: &World, li: u32| w.lines[li as usize].group;
        assert_eq!(w.active_groups, vec![g(&w, a), g(&w, b), g(&w, c)]);
        let now = w.tick as i64;
        w.step_belts(now);
        assert!(!w.lines[a as usize].active);
        assert_eq!(w.active_groups, vec![g(&w, c), g(&w, b)]);
        assert_eq!(w.line_groups[g(&w, a) as usize].slot, NONE);
        assert_eq!((w.line_groups[g(&w, c) as usize].slot, w.line_groups[g(&w, b) as usize].slot), (0, 1));
        assert_eq!((pos(&w.lines[b as usize]), pos(&w.lines[c as usize])), (vec![292], vec![292]));
    }
}
