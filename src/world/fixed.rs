//! Фиксированная точка и детерминированная целочисленная тригонометрия.
//!
//! В состоянии мира нет `f32`/`f64`. Единицы:
//! - позиция — `i32` в 1/256 тайла (как `MapPosition` в Factorio);
//! - угол руки манипулятора — [`Turn`]: `u64`, где 2^64 = один оборот
//!   (0 = север, по часовой стрелке, ось y экрана вниз — как ориентация в игре);
//! - вылет руки — `double` в целочисленной эмуляции ([`super::softfloat::Sf`]); [`Ext`] (Q48) —
//!   только для направлений векторов прототипа (угол руки);
//! - «высота» руки — `float` (программная эмуляция, [`super::softfloat::Sf`]).
//!
//! Синус и косинус руки — многочлен ([`hand_offset`], коэффициенты подобраны по эталонам).
//! Угол цели (`atan2`) — ряд Арктангенса в `i128` с 62 дробными битами ([`orientation_of`]):
//! одинаковый результат на любой машине (только целые сложения, умножения, сдвиги и деления).
//!
//! Игра хранит угол руки во `float` (32 бита). Это видно по эталону: рука, «прилипшая»
//! к предмету на ленте, показывает координату на 1/256 меньше точной, ровно когда
//! угол, округлённый до `float`, чуть меньше точного (проверено по эталону: захваты в
//! straight-belt, belt-to-chest-fast, long-express, two-lanes). Поэтому угол цели
//! округляется до сетки `float` функцией [`orientation_f32`]; шаг угла — во `float`
//! (`inserter::Hand::step`; выведено из наблюдений, совпадает со всеми снятыми положениями руки). Всё это — целочисленная эмуляция
//! округления, а не вычисление во `float`.
//!
//! Вылет руки — `double` (выведено из наблюдений), с программной эмуляцией; гипотеза «вылет во
//! `float`» проверена и отвергнута (docs/mechanics/inserters.md, «Арифметика руки»).

/// Позиция в 1/256 тайла.
pub type Fx = i32;
/// Один тайл в единицах [`Fx`].
pub const TILE: Fx = 256;

/// Угол в оборотах: 2^64 = один оборот.
pub type Turn = u64;
/// Пол-оборота.
pub const HALF_TURN: Turn = 1 << 63;

/// Вылет (и высота) руки: тайлы, 48 дробных бит.
pub type Ext = u64;
pub const EXT_FRAC: u32 = 48;
pub const EXT_ONE: Ext = 1 << EXT_FRAC;

/// Сколько дробных бит у промежуточных значений тригонометрии.
const Q: u32 = 62;
const ONE_Q: i128 = 1 << Q;
// Математические константы (не числа игры), посчитаны заранее с 80 знаками.
/// 2π · 2^62.
const TWO_PI_Q62: i128 = 28_976_077_832_308_491_370;
/// π/4 · 2^62.
const PI_4_Q62: i128 = 3_622_009_729_038_561_421;
/// π/8 · 2^62.
const PI_8_Q62: i128 = 1_811_004_864_519_280_711;
/// tan(π/8) · 2^62 (= √2 − 1).
const TAN_PI_8_Q62: i128 = 1_910_222_894_239_003_202;
/// tan(π/16) · 2^62.
const TAN_PI_16_Q62: i128 = 917_321_383_537_060_868;

#[inline]
fn mul_q(a: i128, b: i128) -> i128 {
    (a * b) >> Q
}

/// atan(z), z = num/den, 0 <= num <= den, den > 0. Радианы Q62.
fn atan_ratio(num: i128, den: i128) -> i128 {
    if num == 0 {
        return 0;
    }
    let mut z = (num << Q) / den; // [0, 1]
    let mut base: i128 = 0;
    let mut sign: i128 = 1;
    if z > TAN_PI_8_Q62 {
        // atan(z) = π/4 − atan((1−z)/(1+z))
        z = ((ONE_Q - z) << Q) / (ONE_Q + z);
        base = PI_4_Q62;
        sign = -1;
    }
    let mut inner_base: i128 = 0;
    if z > TAN_PI_16_Q62 {
        // atan(z) = π/8 + atan((z − t)/(1 + z·t)), t = tan(π/8)
        let num2 = z - TAN_PI_8_Q62;
        let den2 = ONE_Q + mul_q(z, TAN_PI_8_Q62);
        z = (num2 << Q) / den2;
        inner_base = PI_8_Q62;
    }
    // ряд: z − z^3/3 + z^5/5 ...
    let z2 = mul_q(z, z);
    let mut p = z;
    let mut s = z;
    let mut n: i128 = 1;
    loop {
        p = -mul_q(p, z2);
        n += 2;
        let term = p / n;
        if term == 0 {
            break;
        }
        s += term;
    }
    base + sign * (inner_base + s)
}

/// Радианы Q62 (0..π/4] → обороты (2^64 = оборот).
fn rad_to_turn(rad: i128) -> u64 {
    // rad · 2^64 / (2π · 2^62); rad < 2^62, сдвиг на 64 помещается в i128.
    ((rad << 64) / TWO_PI_Q62) as u64
}

/// Ориентация вектора (dx, dy) в осях карты (y вниз): 0 = север, по часовой.
/// Как `atan2(dx, -dy) / 2π` по модулю 1. Для (0, 0) — 0.
pub fn orientation_of(dx: i64, dy: i64) -> Turn {
    // Ось «север» = −y. X = dx (восток), Y = −dy (север).
    let x = dx as i128;
    let y = -(dy as i128);
    if x == 0 && y == 0 {
        return 0;
    }
    let ax = x.abs();
    let ay = y.abs();
    // угол от оси +Y к +X, по часовой, в первой четверти: atan(ax/ay)
    let first: u64 = if ax <= ay {
        rad_to_turn(atan_ratio(ax, ay))
    } else {
        (1u64 << 62) - rad_to_turn(atan_ratio(ay, ax))
    };
    match (x >= 0, y >= 0) {
        (true, true) => first,                                    // СВ: 0..1/4
        (true, false) => HALF_TURN - first,                       // ЮВ: 1/4..1/2
        (false, false) => HALF_TURN.wrapping_add(first),          // ЮЗ: 1/2..3/4
        (false, true) => 0u64.wrapping_sub(first),                // СЗ: 3/4..1
    }
}

/// Округление знакового значения (2^-64 оборота, |v| < 2 оборотов) до сетки `float`.
fn round_f32_signed(v: i128) -> i128 {
    let m = v.unsigned_abs();
    // |v| < 2^65 — 24 значащих бита считаем по u128.
    let r = if m == 0 {
        0
    } else {
        let top = 127 - m.leading_zeros();
        if top < 24 {
            m
        } else {
            let shift = top - 23;
            let unit = 1u128 << shift;
            let rem = m & (unit - 1);
            let base = m - rem;
            let half = unit >> 1;
            if rem > half || (rem == half && (base >> shift) & 1 == 1) {
                base + unit
            } else {
                base
            }
        }
    };
    if v < 0 {
        -(r as i128)
    } else {
        r as i128
    }
}

const TURN_I: i128 = 1 << 64;

/// Угол во `float` после прибавления к нему `delta` (как `orientation += speed` в игре):
/// сумма округляется до `float`, затем приводится в [0, 1) — вычитание 1 точно, а
/// прибавление 1 к отрицательному снова округляется до `float`.
pub fn turn_add_f32(o: Turn, delta: i128) -> Turn {
    let n = round_f32_signed(o as i128 + delta);
    let w = if n >= TURN_I {
        n - TURN_I
    } else if n < 0 {
        round_f32_signed(n + TURN_I)
    } else {
        n
    };
    // ровно 1.0 после округления — это 0
    (w.rem_euclid(TURN_I)) as Turn
}

/// Ориентация вектора, как её считает игра для цели руки: `atan2(dx, −dy) / 2π` в
/// `double` (в (−0.5, 0.5]), приведение к `float`, отрицательное — `+1` во `float`.
/// Проверено по эталону: семь захватов с ленты из пяти сцен дают ровно те же
/// координаты руки (с точностью до знака ошибки округления), см. inserters.md.
pub fn orientation_f32(dx: i64, dy: i64) -> Turn {
    let t = orientation_of(dx, dy);
    // в знаковый вид (−0.5, 0.5]
    let signed: i128 = if t > HALF_TURN { t as i128 - TURN_I } else { t as i128 };
    turn_add_f32(0, signed)
}

use super::softfloat::{Sf, P32, P64};

/// Перевод оборотов (f64 из прототипа) в [`Turn`]. Только при загрузке.
pub fn turn_from_f64(x: f64) -> Turn {
    // 2^64 не помещается в f64-целое без потери, но x < 1, точность 2^-53 достаточна.
    (x * 18_446_744_073_709_551_616.0).round() as Turn
}

/// Позиция из f64 (JSON сцены) в 1/256 тайла. Только при загрузке.
pub fn fx_from_f64(x: f64) -> Fx {
    (x * 256.0).round() as Fx
}

/// Целочисленный корень (floor) для u128.
pub fn isqrt_u128(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Начальное приближение сверху: 2^ceil(bits/2) >= sqrt(n); дальше Ньютон вниз.
    let bits = 128 - n.leading_zeros();
    let mut x: u128 = 1u128 << bits.div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            break;
        }
        x = y;
    }
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// Длина вектора (dx, dy) в 1/256 тайла → тайлы `double`: `sqrt(dx² + dy²)/256`, корень с
/// правильным округлением IEEE 754; сумма квадратов в `double` точна (< 2^53).
pub fn hypot_f64(dx: i64, dy: i64) -> Sf {
    let s = dx as i128 * dx as i128 + dy as i128 * dy as i128;
    assert!(s < 1 << 53, "вектор руки слишком длинный");
    Sf::from_int(s as i64, P64).sqrt(P64).mul_pow2(-8)
}


/// Коэффициенты синуса руки (`double`): `sin(2π·u) ≈ u·((A1 − A3·u²) + (A5 − A7·u²)·u⁴ + A9·u⁸)`,
/// `u ∈ [−1/4, 1/4]` оборота. Подобрано по эталонам: многочлен 9-й степени удовлетворяет всем 1145
/// «чувствительным» компонентам вектора руки (линейная программа), эти коэффициенты совпадают со всеми снятыми
/// положениями руки (фаззер, seeds 1 и 2, bench/README.md; проба p16); угол руки — `float`, вылет — `double`.
/// Это не `sin` из libm: погрешность ~5·10⁻⁹, и от её знака зависит последний бит руки.
const SIN_A1: Sf = Sf::from_f64_bits(0x401921fb51bf1614); // 6.283185269630412
const SIN_A3: Sf = Sf::from_f64_bits(0x4044abbc02329376); // 41.34167506665737
const SIN_A5: Sf = Sf::from_f64_bits(0x405466876b29f494); // 81.60201529595571
const SIN_A7: Sf = Sf::from_f64_bits(0x405324687a27a35e); // 76.56887678023256
const SIN_A9: Sf = Sf::from_f64_bits(0x4043d4243780214b); // 39.65735524898863

/// `cos(2π·v)` по подобранной схеме: `u = 1/4 − |v − round(v)|` (round — к ближайшему чётному),
/// затем многочлен; порядок операций подобран по наблюдениям (каждая операция — `double`).
/// `sin(2π·o)` берётся как `cos(2π·(o − 1/4))`.
fn engine_cos(v: Sf) -> Sf {
    let n = Sf::from_int(v.round_even(), P64);
    let u = Sf::from_f64_bits(0x3fd0000000000000).sub(v.sub(n, P64).abs(), P64); // 0.25 − |v − n|
    let u2 = u.mul(u, P64);
    let t = SIN_A5.sub(SIN_A7.mul(u2, P64), P64);
    let u4 = u2.mul(u2, P64);
    let t1 = SIN_A1.sub(SIN_A3.mul(u2, P64), P64);
    let t = t.mul(u4, P64).add(t1, P64);
    let u8 = u4.mul(u4, P64);
    t.add(u8.mul(SIN_A9, P64), P64).mul(u, P64)
}

/// Смещение руки от центра манипулятора в 1/256 тайла (без высоты),
/// выведено из наблюдений: угол `o` (`float`) → `double`; `S = sin`, `C = cos` по [`engine_cos`];
/// `x = −trunc((ext·S)·(−256))`, `y = −trunc((ext·C)·256)`, `ext` — `double`, `trunc` —
/// усечение к нулю. Высоту вычитает [`super::inserter::Hand::offset`].
/// Проверено по эталону: все 1.8 млн наблюдений руки фаззера (seeds 1 и 2, bench/README.md).
pub fn hand_offset(o: Turn, e: Sf) -> (i64, i64) {
    polar_offset(Sf::from_scaled(o as u128, -64, P64), e)
}

/// `(trunc(256·e·sin 2πv), −trunc(256·e·cos 2πv))`: угол `v` (доля оборота) и длина `e` — `double`.
fn polar_offset(v: Sf, e: Sf) -> (i64, i64) {
    let s = engine_cos(v.sub(Sf::from_f64_bits(0x3fd0000000000000), P64));
    let c = engine_cos(v);
    let x = e.mul(s, P64).mul_pow2(8).trunc();
    let y = -e.mul(c, P64).mul_pow2(8).trunc();
    (x, y)
}

/// Радиусы полос поворота (`float`): внутренняя 0.265, внешняя 0.735 тайла от внутреннего угла.
const TURN_R_INNER: Sf = Sf::from_f64_bits(0x3fd0f5c280000000);
const TURN_R_OUTER: Sf = Sf::from_f64_bits(0x3fe7851ec0000000);

/// Точка предмета на полосе поворота (к ней тянется рука манипулятора), выведено из
/// наблюдений: дуга вокруг внутреннего угла тайла. `d` — расстояние до переднего края тайла по
/// полосе, `len` — длина полосы (106 или 295), `outer` — внешняя полоса, `left` — поворот налево.
/// Угол `a = 1/4 + 1/4·d/len` (`double`, затем `float`); смещение от центра тайла в системе ленты,
/// смотрящей на север (x — вправо, y — назад), 1/256 тайла:
/// `x = ∓128 ± trunc(256·r·sin 2πa)`, `y = −128 − trunc(256·r·cos 2πa)` (верхний знак — налево).
pub fn turn_arc_offset(d: i64, len: i64, outer: bool, left: bool) -> (i64, i64) {
    assert!((0..1 << 16).contains(&d) && (1..1 << 16).contains(&len));
    // d/256·1/4 / (len/256) — частное в `double` с округлением к ближайшему
    let num = (d as u128) << 100;
    let den = 4 * len as u128;
    let q = Sf::round(false, num / den, -100, num % den != 0, P64);
    let quarter = Sf::from_f64_bits(0x3fd0000000000000);
    let one = Sf::from_int(1, P32);
    let mut a = q.add(quarter, P64).to_prec(P32);
    if !a.lt(one) {
        // доля оборота: дробная часть во `float`
        a = a.sub(Sf::from_int(a.trunc(), P32), P32);
    }
    let r = if outer { TURN_R_OUTER } else { TURN_R_INNER };
    let (x, y) = polar_offset(a, r);
    if left {
        (-128 + x, -128 + y)
    } else {
        (128 - x, -128 + y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(x: f64) -> Turn {
        turn_from_f64(x)
    }

    #[test]
    fn orientation_basic() {
        assert_eq!(orientation_of(0, -5), 0);
        assert_eq!(orientation_of(5, 0), 1u64 << 62);
        assert_eq!(orientation_of(0, 5), HALF_TURN);
        assert_eq!(orientation_of(-5, 0), 3u64 << 62);
        for &(dx, dy) in &[(-381i64, 60i64), (325, -60), (17, 300), (-1, -1000)] {
            let o = orientation_of(dx, dy) as f64 / 2f64.powi(64);
            let want = ((dx as f64).atan2(-(dy as f64)) / std::f64::consts::TAU).rem_euclid(1.0);
            assert!((o - want).abs() < 1e-15, "{dx},{dy}: {o} vs {want}");
        }
    }

    #[test]
    fn f32_grid() {
        let v = turn(0.7251);
        let r = turn_add_f32(0, v as i128) as f64 / 2f64.powi(64);
        assert_eq!(r as f32, 0.7251f32);
        assert_eq!(r, 0.7251f32 as f64);
    }

    #[test]
    fn orientation_f32_negative_wraps_in_float() {
        // west-north-west: atan2 < 0, float(v) + 1.0f (два округления)
        let o = orientation_f32(-136, -60) as f64 / 2f64.powi(64);
        let v = ((-136f64).atan2(60.0) / std::f64::consts::TAU) as f32;
        assert_eq!(o, (v + 1.0f32) as f64);
    }

    #[test]
    fn hand_offsets_from_reference() {
        // Источник в straight-belt, тик 2: угол 0.75 + 0.014, вылет 1.035 → (−263, ...).
        let o = turn(0.764);
        let e = Sf::from_f64(1.035);
        let (x, _y) = hand_offset(o, e);
        assert_eq!(x, -263);
    }
}
