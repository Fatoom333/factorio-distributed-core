//! Программная (целочисленная) эмуляция `float`/`double` IEEE 754 для тех мест, где движок
//! считает во `float`/`double` и от последнего бита зависит результат (рука манипулятора).
//!
//! Только конечные нормальные числа и ноль; двоичная арифметика IEEE 754 с округлением к
//! ближайшему, при равенстве — к чётному. Бесконечности, NaN и денормалы не нужны: значения здесь —
//! углы в [0, 1), вылеты и высоты руки в тайлах; выход за диапазон (потеря порядка или
//! переполнение) — паника, а не тихая ошибка.
//! Только целые операции: результат одинаков на любой машине.

/// Число с плавающей точкой: `(-1)^neg · m · 2^e`; `m == 0` (ноль) или `m` ровно из `p` бит
/// (`2^(p-1) <= m < 2^p`), где `p` — точность формата (24 для `float`, 53 для `double`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Sf {
    pub neg: bool,
    pub m: u64,
    pub e: i32,
}

/// Точность `float` (бит мантиссы со скрытой единицей).
pub const P32: u32 = 24;
/// Точность `double`.
pub const P64: u32 = 53;

/// Наименьший показатель нормального числа (`2^-126` для `float`, `2^-1022` для `double`),
/// в терминах `m · 2^e` с `p`-битной `m`.
fn min_e(p: u32) -> i32 {
    if p == P32 {
        -126 - 23
    } else {
        -1022 - 52
    }
}

/// Наибольший показатель конечного числа в тех же терминах (`2^127·(2 − 2^-23)` для `float`,
/// `2^1023·(2 − 2^-52)` для `double`).
fn max_e(p: u32) -> i32 {
    if p == P32 {
        127 - 23
    } else {
        1023 - 52
    }
}

impl Sf {
    /// Число записано в формате точности `p`: ноль или мантисса ровно из `p` бит.
    pub fn is_format(self, p: u32) -> bool {
        self.m == 0 || self.m >> (p - 1) == 1
    }

    pub const ZERO: Sf = Sf {
        neg: false,
        m: 0,
        e: 0,
    };

    /// Точное значение `(-1)^neg · m · 2^e` (+ «хвост» `sticky` — ненулевые биты ниже `m`),
    /// округлённое до `p` бит.
    pub fn round(neg: bool, m: u128, e: i32, sticky: bool, p: u32) -> Sf {
        if m == 0 {
            assert!(!sticky, "softfloat: потеря значащих бит у нуля");
            return Sf::ZERO;
        }
        let bits = 128 - m.leading_zeros();
        let (mut mm, mut ee) = if bits > p {
            let shift = bits - p;
            let rem = m & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            let mut q = m >> shift;
            let up = rem > half || (rem == half && (sticky || q & 1 == 1));
            if up {
                q += 1;
            }
            (q, e + shift as i32)
        } else {
            assert!(!sticky, "softfloat: хвост у точного значения");
            (m << (p - bits), e - (p - bits) as i32)
        };
        if mm == 1u128 << p {
            mm >>= 1;
            ee += 1;
        }
        assert!(ee >= min_e(p), "softfloat: денормал/потеря порядка");
        assert!(ee <= max_e(p), "softfloat: переполнение порядка");
        Sf {
            neg,
            m: mm as u64,
            e: ee,
        }
    }

    /// Целое → число формата `p` (с округлением, как при переводе целого в float/double на процессоре).
    pub fn from_int(v: i64, p: u32) -> Sf {
        Sf::round(v < 0, v.unsigned_abs() as u128, 0, false, p)
    }

    /// `v · 2^scale` → формат `p` (точно, если помещается; иначе — округление).
    pub fn from_scaled(v: u128, scale: i32, p: u32) -> Sf {
        Sf::round(false, v, scale, false, p)
    }

    /// Битовый образ `double` → `Sf` (для констант движка; только нормальные числа).
    pub const fn from_f64_bits(b: u64) -> Sf {
        let neg = b >> 63 == 1;
        let exp = ((b >> 52) & 0x7ff) as i32;
        let frac = b & ((1u64 << 52) - 1);
        if exp == 0 {
            assert!(frac == 0, "softfloat: денормал в константе");
            return Sf::ZERO;
        }
        assert!(exp != 0x7ff, "softfloat: inf/nan в константе");
        Sf {
            neg,
            m: frac | (1u64 << 52),
            e: exp - 1075,
        }
    }

    /// `f64` → `Sf` (только при загрузке чисел прототипов; точно).
    pub fn from_f64(x: f64) -> Sf {
        Sf::from_f64_bits(x.to_bits())
    }

    /// Квадратный корень с правильным округлением IEEE 754; только `>= 0`.
    pub fn sqrt(self, p: u32) -> Sf {
        assert!(!self.neg || self.m == 0, "softfloat: корень из отрицательного");
        if self.m == 0 {
            return Sf::ZERO;
        }
        // m·2^e, e чётный; m·2^(2k) — до 126 бит, корень — до 63 бит (≥ p + 2 при p ≤ 53).
        let (mut m, mut e) = (self.m as u128, self.e);
        if e & 1 != 0 {
            m <<= 1;
            e -= 1;
        }
        let bits = 128 - m.leading_zeros();
        let k = (126 - bits) / 2;
        let n = m << (2 * k);
        let r = super::fixed::isqrt_u128(n);
        Sf::round(false, r, e / 2 - k as i32, r * r != n, p)
    }

    /// Перевод в другой формат (float → double точно, double → float — с округлением).
    pub fn to_prec(self, p: u32) -> Sf {
        Sf::round(self.neg, self.m as u128, self.e, false, p)
    }

    /// Приближённое значение для отладочного вывода (не для состояния и не для расчёта).
    pub fn approx_f64(self) -> f64 {
        let v = self.m as f64 * 2f64.powi(self.e);
        if self.neg {
            -v
        } else {
            v
        }
    }

    pub fn is_zero(self) -> bool {
        self.m == 0
    }

    pub fn abs(self) -> Sf {
        Sf { neg: false, ..self }
    }

    pub fn negate(self) -> Sf {
        if self.m == 0 {
            self
        } else {
            Sf {
                neg: !self.neg,
                ..self
            }
        }
    }

    /// Умножение на `2^k` (точно).
    pub fn mul_pow2(self, k: i32) -> Sf {
        if self.m == 0 {
            self
        } else {
            Sf {
                e: self.e + k,
                ..self
            }
        }
    }

    pub fn mul(self, o: Sf, p: u32) -> Sf {
        if self.m == 0 || o.m == 0 {
            return Sf::ZERO;
        }
        Sf::round(
            self.neg != o.neg,
            self.m as u128 * o.m as u128,
            self.e + o.e,
            false,
            p,
        )
    }

    pub fn add(self, o: Sf, p: u32) -> Sf {
        if self.m == 0 {
            return o.to_prec(p);
        }
        if o.m == 0 {
            return self.to_prec(p);
        }
        // hi — с большим показателем
        let (hi, lo) = if self.e >= o.e { (self, o) } else { (o, self) };
        let d = (hi.e - lo.e) as u32;
        // 53 + 74 = 127 бит: сдвиг hi до 74 бит точен. Дальше lo сдвигается вправо, а
        // выпавшие биты помечаются хвостом `sticky` (hi после сдвига >= 2^126, поэтому хвост
        // влияет только на округление).
        const SPAN: u32 = 74;
        let (hm, lm, e, sticky) = if d <= SPAN {
            ((hi.m as u128) << d, lo.m as u128, lo.e, false)
        } else {
            let s = d - SPAN;
            let (lv, st) = if s >= 64 {
                (0u64, true)
            } else {
                (lo.m >> s, lo.m & ((1u64 << s) - 1) != 0)
            };
            ((hi.m as u128) << SPAN, lv as u128, hi.e - SPAN as i32, st)
        };
        if hi.neg == lo.neg {
            Sf::round(hi.neg, hm + lm, e, sticky, p)
        } else if sticky {
            // hi − lo − ε, 0 < ε < 1: (hm − lm − 1) + (1 − ε)
            Sf::round(hi.neg, hm - lm - 1, e, true, p)
        } else if hm >= lm {
            Sf::round(hi.neg, hm - lm, e, false, p)
        } else {
            Sf::round(lo.neg, lm - hm, e, false, p)
        }
    }

    pub fn sub(self, o: Sf, p: u32) -> Sf {
        self.add(o.negate(), p)
    }

    /// Сравнение значений.
    pub fn cmp_val(self, o: Sf) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let key = |x: Sf| -> i8 {
            if x.m == 0 {
                0
            } else if x.neg {
                -1
            } else {
                1
            }
        };
        let (ka, kb) = (key(self), key(o));
        if ka != kb || ka == 0 {
            return ka.cmp(&kb);
        }
        // одного знака, оба ненулевые: сравниваем модули
        let mag = {
            // точности могут различаться: сравниваем старший бит, затем мантиссы после выравнивания
            let (a, b) = (self, o);
            let ba = 64 - a.m.leading_zeros() as i32 + a.e; // старший бит + 1
            let bb = 64 - b.m.leading_zeros() as i32 + b.e;
            if ba != bb {
                ba.cmp(&bb)
            } else {
                let e = a.e.min(b.e);
                let am = (a.m as u128) << (a.e - e);
                let bm = (b.m as u128) << (b.e - e);
                am.cmp(&bm)
            }
        };
        if ka > 0 {
            mag
        } else {
            match mag {
                Less => Greater,
                Greater => Less,
                Equal => Equal,
            }
        }
    }

    pub fn lt(self, o: Sf) -> bool {
        self.cmp_val(o) == std::cmp::Ordering::Less
    }

    pub fn min(self, o: Sf) -> Sf {
        // a < b ? a : b (при равенстве — второй аргумент)
        if self.lt(o) {
            self
        } else {
            o
        }
    }

    pub fn max(self, o: Sf) -> Sf {
        if o.lt(self) {
            self
        } else {
            o
        }
    }

    /// Усечение к нулю до целого (как перевод float/double в целое на процессоре).
    pub fn trunc(self) -> i64 {
        if self.m == 0 {
            return 0;
        }
        let mag: i64 = if self.e >= 0 {
            assert!(self.e < 10, "softfloat: слишком большое для целого");
            (self.m << self.e) as i64
        } else if self.e <= -64 {
            0
        } else {
            (self.m >> (-self.e)) as i64
        };
        if self.neg {
            -mag
        } else {
            mag
        }
    }

    /// Округление до целого к ближайшему, при равенстве — к чётному (как округление float/double в целое на процессоре).
    pub fn round_even(self) -> i64 {
        if self.m == 0 {
            return 0;
        }
        let mag: i64 = if self.e >= 0 {
            assert!(self.e < 10, "softfloat: слишком большое для целого");
            (self.m << self.e) as i64
        } else if self.e < -64 {
            0
        } else {
            let s = (-self.e) as u32;
            let m = self.m as u128;
            let q = m >> s;
            let rem = m & ((1u128 << s) - 1);
            let half = 1u128 << (s - 1);
            (if rem > half || (rem == half && q & 1 == 1) {
                q + 1
            } else {
                q
            }) as i64
        };
        if self.neg {
            -mag
        } else {
            mag
        }
    }

    /// Точное значение в единицах `2^-scale` (паника, если не целое или не влезает).
    pub fn to_scaled(self, scale: i32) -> i128 {
        if self.m == 0 {
            return 0;
        }
        let sh = self.e + scale;
        let mag: i128 = if sh >= 0 {
            assert!(sh < 64, "softfloat: не влезает");
            (self.m as i128) << sh
        } else {
            let s = (-sh) as u32;
            assert!(
                s < 64 && self.m & ((1u64 << s) - 1) == 0,
                "softfloat: не целое в 2^-{scale}"
            );
            (self.m >> s) as i128
        };
        if self.neg {
            -mag
        } else {
            mag
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Сверка с аппаратной арифметикой IEEE (только в тестах: f64/f32 здесь — эталон).
    fn to_f64(x: Sf) -> f64 {
        let v = x.m as f64 * 2f64.powi(x.e);
        if x.neg {
            -v
        } else {
            v
        }
    }
    fn of_f64(v: f64) -> Sf {
        Sf::from_f64_bits(v.to_bits())
    }
    fn of_f32(v: f32) -> Sf {
        of_f64(v as f64).to_prec(P32)
    }

    #[test]
    fn matches_hardware() {
        // простой детерминированный ГПСЧ
        let mut s: u64 = 0x1234_5678_9abc_def1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..200_000 {
            let a = (next() % 2_000_000) as f64 / 1_000_000.0 - 1.0;
            let ex = (next() % 160) as i32 - 130;
            let b = ((next() % 2_000_000) as f64 / 777_777.0 - 1.3) * 2f64.powi(ex);
            let (sa, sb) = (of_f64(a), of_f64(b));
            assert_eq!(to_f64(sa.add(sb, P64)), a + b, "{a} + {b}");
            assert_eq!(to_f64(sa.sub(sb, P64)), a - b, "{a} - {b}");
            assert_eq!(to_f64(sa.mul(sb, P64)), a * b, "{a} * {b}");
            assert_eq!(to_f64(sa.to_prec(P32)), (a as f32) as f64);
            assert_eq!(sa.trunc(), a.trunc() as i64);
            assert_eq!(sa.cmp_val(sb), a.partial_cmp(&b).unwrap());
            if ex < -60 {
                continue;
            }
            let (fa, fb) = (a as f32, b as f32);
            let (ga, gb) = (of_f32(fa), of_f32(fb));
            assert_eq!(to_f64(ga.add(gb, P32)), (fa + fb) as f64, "{fa} + {fb}");
            assert_eq!(to_f64(ga.sub(gb, P32)), (fa - fb) as f64, "{fa} - {fb}");
            assert_eq!(to_f64(ga.mul(gb, P32)), (fa * fb) as f64, "{fa} * {fb}");
            assert_eq!(to_f64(sa.abs().sqrt(P64)), a.abs().sqrt(), "sqrt {a}");
            assert_eq!(to_f64(sb.abs().sqrt(P64)), b.abs().sqrt(), "sqrt {b}");
            let r = (a * 3.0).round_ties_even();
            assert_eq!(of_f64(a * 3.0).round_even(), r as i64);
        }
        assert_eq!(of_f64(0.5).round_even(), 0);
        assert_eq!(of_f64(1.5).round_even(), 2);
        assert_eq!(of_f64(-0.5).round_even(), 0);
    }

    fn rng() -> impl FnMut() -> u64 {
        let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
        move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        }
    }

    /// Случайное число в [1, 2) со случайным знаком и случайным 53-битным (или 24-битным) хвостом.
    fn unit(next: &mut impl FnMut() -> u64) -> f64 {
        let v = f64::from_bits(0x3ff0_0000_0000_0000 | (next() >> 12));
        if next() & 1 == 1 {
            -v
        } else {
            v
        }
    }

    /// Сложение и вычитание `float` и `double` при любой разнице порядков (в том числе когда
    /// меньшее слагаемое целиком в «хвосте»): против аппаратной арифметики, без денормалов.
    #[test]
    fn add_sub_wide_exponent_gaps() {
        let mut next = rng();
        for _ in 0..200_000 {
            let d = (next() % 151) as i32; // разница порядков 0..=150
            // float: a ~ 2^100, b ~ 2^(100 − d) >= 2^-50 — всё нормальное, результат тоже
            let a = (unit(&mut next) as f32) * 2f32.powi(100);
            let b = (unit(&mut next) as f32) * 2f32.powi(100 - d);
            assert!(a.is_normal() && b.is_normal());
            let (ga, gb) = (of_f32(a), of_f32(b));
            assert_eq!(to_f64(ga.add(gb, P32)), (a + b) as f64, "f32 {a} + {b} (d={d})");
            assert_eq!(to_f64(ga.sub(gb, P32)), (a - b) as f64, "f32 {a} - {b} (d={d})");
            assert_eq!(to_f64(gb.add(ga, P32)), (b + a) as f64, "f32 {b} + {a} (d={d})");
            // double: a ~ 2^500, b ~ 2^(500 − d)
            let a = unit(&mut next) * 2f64.powi(500);
            let b = unit(&mut next) * 2f64.powi(500 - d - (next() % 60) as i32);
            let (sa, sb) = (of_f64(a), of_f64(b));
            assert_eq!(sa.add(sb, P64), of_f64(a + b), "f64 {a} + {b}");
            assert_eq!(sa.sub(sb, P64), of_f64(a - b), "f64 {a} - {b}");
        }
    }

    #[test]
    fn sqrt_min_max_round_even_random() {
        let mut next = rng();
        for _ in 0..200_000 {
            let a = unit(&mut next).abs() as f32 * 2f32.powi((next() % 120) as i32 - 60);
            assert_eq!(to_f64(of_f32(a).sqrt(P32)), a.sqrt() as f64, "sqrt f32 {a}");
            let (x, y) = (unit(&mut next) * 2f64.powi((next() % 8) as i32), unit(&mut next));
            let (sx, sy) = (of_f64(x), of_f64(y));
            assert_eq!(to_f64(sx.min(sy)), if x < y { x } else { y }, "min {x} {y}");
            assert_eq!(to_f64(sx.max(sy)), if y < x { x } else { y }, "max {x} {y}");
            assert_eq!(sx.min(sx), sx);
            // целое + половина: ровно на границе округления к чётному
            let h = (next() % 4001) as f64 - 2000.0 + 0.5;
            assert_eq!(of_f64(h).round_even(), h.round_ties_even() as i64, "round_even {h}");
            assert_eq!(sx.round_even(), x.round_ties_even() as i64, "round_even {x}");
        }
    }

    #[test]
    #[should_panic(expected = "переполнение порядка")]
    fn exponent_overflow_panics() {
        let big = Sf::from_f64(1e300);
        let _ = big.mul(big, P64);
    }

    #[test]
    #[should_panic(expected = "переполнение порядка")]
    fn f32_exponent_overflow_panics() {
        let big = of_f32(1e30);
        let _ = big.mul(big, P32);
    }
}
