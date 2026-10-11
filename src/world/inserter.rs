//! Манипулятор: рука (угол, вылет, высота) и её движение за тик.
//!
//! Что проверено по эталону (сцены tests/fidelity и пробные сцены, см.
//! docs/mechanics/inserters.md):
//! - рука — угол `o` (оборот, во `float`), вылет `ext` и высота `h` (`float`); позиция руки
//!   `центр + усечение_к_нулю(ext · (S, −C) · 256) − (0, усечение(256·h))`, где S, C — синус
//!   и косинус движка ([`hand_offset`], многочлен, подобранный по эталонам);
//! - за тик угол меняется не больше чем на `rotation_speed`, вылет — на шаг
//!   `min(|цель − вылет|, extension_speed)` в `double`; затем доводка: если после шага до цели
//!   вылета осталось меньше `extension_speed`, вылет ставится точно на цель, и только тогда
//!   проверяется угол — если до цели меньше `rotation_speed`, угол тоже ставится на цель и рука
//!   пришла (так нога в пол-оборота занимает 35 тиков, а не 36);
//! - на ровно пол-оборота рука идёт «длинным» знаком: разность `цель − угол` = +0.5
//!   крутит в минус, −0.5 — в плюс (сырые значения в [0, 1));
//! - высота `h` (рисуется вверх по экрану) за тик меняется на
//!   `(остаток − 0.25) · (поворот за тик · 20)` тайла во `float`, где остаток — угол до цели
//!   после шага (в оборотах), и лежит в [0, 0.75] тайла: рука поднимается, пока до цели больше
//!   четверти оборота, и опускается ближе (выведено из наблюдений, см. [`Hand::step`]).

use serde::Serialize;

use super::belt::ItemId;
use super::fixed::{hand_offset, Fx, Turn};
use super::softfloat::{Sf, P32, P64};
use super::wake::WakeRef;

/// Куда смотрит манипулятор (откуда берёт) и куда кладёт.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Target {
    None,
    Chest(u32),
    Belt(u32),
}

/// Рука манипулятора.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Hand {
    /// Угол (оборот · 2^64), всегда на сетке `float`.
    pub o: Turn,
    /// Вылет, тайлы, `double` (программная эмуляция).
    pub ext: Sf,
    /// Высота, тайлы, `float` (программная эмуляция; в [0, 0.75]).
    pub h: Sf,
}

/// Цель руки на этот тик.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct HandTarget {
    pub o: Turn,
    /// Вылет цели, тайлы, `double`.
    pub ext: Sf,
}

/// Константы `float`, выведенные из наблюдений (Factorio 2.0.77). Закон высоты руки — постоянные
/// (0.25f, 20.0f, 0.75f), а не поля прототипа: от прототипа зависят только `extension_speed` и
/// `rotation_speed` (у обычного качества множитель 1.0) и энергия на движение/поворот.
/// Значит, закон один для всех электрических манипуляторов (проверено по эталону для
/// `inserter`, `fast-inserter`, `long-handed-inserter`: фаззер, seed 101 (bench/README.md); предел 0.75 эталоны
/// достигают только у `inserter`, у быстрого/длиннорукого высота за прогон не выше 0.53/0.63).
const F_ONE: Sf = Sf {
    neg: false,
    m: 1 << 23,
    e: -23,
}; // 1.0f
const F_HALF: Sf = Sf {
    neg: false,
    m: 1 << 23,
    e: -24,
}; // 0.5f
const F_QUARTER: Sf = Sf {
    neg: false,
    m: 1 << 23,
    e: -25,
}; // 0.25f
/// Множитель высоты руки: `Δh = (остаток − 0.25) · (поворот · 20)`.
const F_TWENTY: Sf = Sf {
    neg: false,
    m: 5 << 21,
    e: -19,
}; // 20.0f
/// Предел высоты руки, тайлы.
const F_H_MAX: Sf = Sf {
    neg: false,
    m: 3 << 22,
    e: -24,
}; // 0.75f
/// Порог шага вылета (`double` 0.001): шаг не больше порога пропускается (выведено из наблюдений).
const D_EXT_MIN: Sf = Sf::from_f64_bits(0x3f50624dd2f1a9fc);

/// Угол (обороты на сетке `float`) → `float`.
fn turn_f32(o: Turn) -> Sf {
    let v = Sf::from_scaled(o as u128, -64, P32);
    debug_assert_eq!(v.to_scaled(64) as u128, o as u128, "угол не на сетке float");
    v
}

/// `float` в [0, 1) → угол (точно; паника, если не помещается в 2^-64).
fn f32_turn(v: Sf) -> Turn {
    let t = v.to_scaled(64);
    assert!((0..1i128 << 64).contains(&t), "угол вне [0, 1)");
    t as Turn
}

/// Нормализация угла после сложения/вычитания: вне [0, 1) — `x − trunc(x)`, `+1`, снова
/// `x − trunc(x)`, каждое действие во `float` (выведено из наблюдений).
fn normalise(x: Sf) -> Sf {
    if !x.lt(Sf::ZERO) && x.lt(F_ONE) {
        return x;
    }
    let x = x.sub(Sf::from_int(x.trunc(), P32), P32).add(F_ONE, P32);
    x.sub(Sf::from_int(x.trunc(), P32), P32)
}

/// Угол `cur` идёт к `tgt` не больше чем на `step` коротким путём; возвращает пройденное
/// (`float`, >= 0). Порядок операций выведен из наблюдений (`float`): расстояние без перехода через 0 — `|tgt − cur|`, с переходом —
/// `(1 − большее) + меньшее`; ровно 0.5 идёт через 0 (`0.5 > d` — строгое).
fn move_to(cur: &mut Sf, tgt: Sf, step: Sf) -> Sf {
    if !step.lt(F_HALF) {
        // шаг >= пол-оборота: сразу на цель, движение — короткое расстояние
        let d = tgt.sub(*cur, P32).abs();
        let alt = if tgt.lt(*cur) {
            F_ONE.sub(*cur, P32).add(tgt, P32)
        } else {
            F_ONE.sub(tgt, P32).add(*cur, P32)
        };
        *cur = tgt;
        return if d.lt(alt) { d } else { alt };
    }
    if tgt.lt(*cur) {
        let d = cur.sub(tgt, P32);
        if d.lt(F_HALF) {
            let s = step.min(d);
            *cur = cur.sub(s, P32);
            s
        } else {
            let s = step.min(F_ONE.sub(*cur, P32).add(tgt, P32));
            *cur = normalise(s.add(*cur, P32));
            s
        }
    } else {
        let d = tgt.sub(*cur, P32);
        if d.lt(F_HALF) {
            let s = step.min(d);
            *cur = cur.add(s, P32);
            s
        } else {
            let s = step.min(F_ONE.sub(tgt, P32).add(*cur, P32));
            *cur = normalise(cur.sub(s, P32));
            s
        }
    }
}

impl Hand {
    /// Шаг руки к цели (электрический манипулятор с полной энергией). Возвращает `true`,
    /// если рука на цели. Выведено из наблюдений
    /// (совпадает со всеми снятыми положениями руки; поля руки: угол `float`, вылет `double`, высота `float`):
    /// 1. вылет: шаг `min(|цель − вылет|, extension_speed)` к цели;
    /// 2. если угол не равен цели: поворот на `min(float(rotation_speed), расстояние)`
    ///    ([`move_to`]); высота `h = clamp(h + (min(r, 1 − r) − 0.25)·(поворот·20), 0, 0.75)`,
    ///    `r = |цель − угол|` после поворота — всё во `float`;
    /// 3. если до цели вылета меньше шага — вылет на цель; тогда, если угол до цели
    ///    (`d − round(d)`, `d = угол − цель` во `float`) по модулю меньше `rotation_speed`
    ///    (`double`), угол на цель и рука пришла.
    ///
    /// Вылет — `double` (программная эмуляция): при шаге 0.05 (длиннорукий) вылет копит
    /// ошибку `double` (2.0 + 3·0.05 = 2.1499999999999995) и от неё зависит, на каком тике
    /// он «доезжает» (проверено по эталону: фаззер, seed 101, длиннорукий манипулятор).
    /// От прототипа здесь только `rotation_speed` и `extension_speed`.
    pub fn step(&mut self, t: HandTarget, rotation_speed: Turn, extension_speed: Sf) -> bool {
        let es = extension_speed;
        // Форматы полей: вылет, его цель и шаг — double (P64), высота — float (P32).
        debug_assert!(self.ext.is_format(P64) && t.ext.is_format(P64) && es.is_format(P64), "вылет не double");
        debug_assert!(self.h.is_format(P32), "высота не float");
        // 1. вылет (`double`); шаг не больше 0.001 по модулю движок пропускает (доводка в п. 3
        // всё равно поставит вылет на цель, порог влияет только на расход энергии)
        let de = t.ext.sub(self.ext, P64);
        let s = de.abs().min(es);
        if D_EXT_MIN.lt(s) {
            self.ext = if de.neg { self.ext.sub(s, P64) } else { self.ext.add(s, P64) };
        }
        // 2. угол и высота (`rotation_speed`, переведённый из `double` в `float` — шаг во `float`)
        if self.o != t.o {
            let step = Sf::from_scaled(rotation_speed as u128, -64, P32);
            let mut cur = turn_f32(self.o);
            let tgt = turn_f32(t.o);
            let moved = move_to(&mut cur, tgt, step);
            self.o = f32_turn(cur);
            let r = tgt.sub(cur, P32).abs();
            let r = r.min(F_ONE.sub(r, P32));
            let dh = r.sub(F_QUARTER, P32).mul(moved.mul(F_TWENTY, P32), P32);
            self.h = dh.add(self.h, P32).max(Sf::ZERO).min(F_H_MAX);
        }
        // 3. доводка
        if !t.ext.sub(self.ext, P64).abs().lt(es) {
            return false;
        }
        self.ext = t.ext;
        let d = turn_f32(self.o).sub(turn_f32(t.o), P32);
        let d = d.sub(Sf::from_int(d.round_even(), P32), P32).abs();
        if d.lt(Sf::from_scaled(rotation_speed as u128, -64, P64)) {
            self.o = t.o;
            return true;
        }
        false
    }

    /// Позиция руки относительно центра манипулятора, 1/256 тайла (как `held_stack_position`):
    /// высота вычитается отдельно, `trunc(h · 256)` (выведено из наблюдений).
    pub fn offset(&self) -> (Fx, Fx) {
        let (x, y) = hand_offset(self.o, self.ext);
        let h = self.h.mul_pow2(8).trunc();
        (x as Fx, (y - h) as Fx)
    }
}

/// Манипулятор.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Inserter {
    /// Центр, 1/256 тайла.
    pub pos: (Fx, Fx),
    /// Точка захвата (абсолютная), 1/256 тайла — как `pickup_position` в игре.
    pub pickup_point: (Fx, Fx),
    /// Индекс в `World::ins_protos`.
    pub proto: u8,
    pub pickup: Target,
    pub drop: Target,
    /// Точка выгрузки (абсолютная), 1/256 тайла — как `drop_position` в игре.
    pub drop_point: (Fx, Fx),
    /// Номинальные позиции руки: у точки захвата и у точки выгрузки.
    pub at_pickup: HandTarget,
    pub at_drop: HandTarget,
    pub hand: Hand,
    pub held: Option<ItemId>,
    pub held_count: u32,
    /// Предмет на ленте, за которым едет рука (`uid`): манипулятор держится за него,
    /// пока тот на тайле захвата (проверено по эталону two-lanes, тик 282).
    pub chase: Option<u64>,
    /// «Была выбрана цель на ленте»: ставится при удачном выборе предмета, снимается паузой
    /// после потери цели и отказом приёмника; взятие предмета его не снимает.
    pub has_target: bool,
    /// Фаза: `true` — идёт брать, `false` — идёт класть.
    pub pickup_mode: bool,
    /// Стоит ли в списке активных (`World::ins_head`); спящий не обновляется.
    pub awake: bool,
    /// Соседи в списке активных (`wake::NONE` — нет).
    pub act_prev: u32,
    pub act_next: u32,
    /// Списки спящих, в которые он записан (не больше трёх: две полосы и цель выгрузки).
    pub regs: Vec<WakeRef>,
}
