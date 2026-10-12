//! Прототипы (числа конкретных сущностей) и константы движка.
//!
//! **Числа прототипов** в коде модели не записаны: они читаются из выжимки `data.raw` игры
//! ([`Prototypes::from_json`]). Выжимку делает `bin/proto_extract` из
//! `script-output/data-raw-dump.json` (`factorio --dump-data`); в репо лежит
//! `data/prototypes-m0.json` (Factorio 2.0.77, base + space-age + quality + elevated-rails),
//! её отдаёт [`Prototypes::bundled`]. Под другой набор модов — пересобрать выжимку.
//!
//! Перевод в фиксированную точку — при загрузке (единственное место, где модель видит `f64`;
//! перевод детерминирован: `x · 2^k` точно, затем округление). В состоянии мира `float` нет.
//!
//! **Константы движка** (их нет ни в прототипах, ни в рантайм-API) — ниже, у каждой источник.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::fixed::{turn_from_f64, Fx, Turn, EXT_ONE, TILE};
use super::softfloat::{Sf, P64};

// ---------- константы движка ----------

/// Шаг между предметами на полосе: 0.25 тайла = 64. В прототипах и в рантайм-API нет
/// (константа движка); проверено по эталону (`LuaTransportLine.get_detailed_contents`):
/// упёршиеся предметы стоят на p = 0, 64, 128, 192 — на лентах всех четырёх скоростей.
pub const ITEM_SPACING: Fx = 64;

/// Смещение середины полосы от оси ленты: 0.234375 тайла = 60/256. Число игры:
/// `LuaEntity.get_line_item_position(линия, позиция)` у ленты на востоке в (2.5, 0.5) даёт
/// y = 0.265625 для линии 1 и 0.734375 для линии 2 (0.5 ∓ 60/256) при любой позиции
/// (операция мода `scene_info`; `scene_capture` сверяет её с моделью для каждой ленты сцены).
pub const LANE_OFFSET: Fx = 60;

/// Длина полосы прямого тайла — 1 тайл (256). Число игры: `get_line_item_position(линия, p)`
/// сдвигается на p тайлов вдоль ленты, p = 0 — край тайла по ходу (`scene_info`); и по
/// эталону — при переходе на следующий тайл `p` прибавляет 256.
pub const TILE_LANE_LEN: Fx = 256;

/// Длины полос поворота: внутренняя 106, внешняя 295 (четверти дуг радиусов 0.265625 и
/// 0.734375 тайла, усечённые к 1/256; сумма точных длин — π/2). Число игры: `line_length`
/// полос поворота 0.4140625 и 1.15234375 (`(end − start)/256`); в эталоне `p` на угловом тайле
/// доходит до 105 и 294.
pub const TURN_INNER_LEN: Fx = 106;
pub const TURN_OUTER_LEN: Fx = 295;

/// Боковая загрузка: точки вставки в ближнюю полосу приёмника, от его переднего края. Полоса
/// источника, ближняя к переднему краю приёмника, — в 67 (0.265 тайла), дальняя — в 188 (0.735).
pub const SIDE_NEAR: Fx = 67;
pub const SIDE_FAR: Fx = 188;

// Закон высоты руки (`Δh = (остаток − 0.25)·(поворот·20)`, предел 0.75) и синус руки — тоже
// константы движка: `float`/`double`, выведенные из наблюдений (Factorio 2.0.77), они записаны
// рядом с кодом (`inserter::Hand::step`, `fixed::hand_offset`). От прототипа там зависит
// только `rotation_speed` (через пройденный за тик угол).

// ---------- прототипы ----------

/// Лента (`transport-belt` в data.raw).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct BeltProto {
    pub name: String,
    /// `speed` (тайлов за тик) · 256 — в 1/256 тайла за тик (должно быть целым).
    pub speed: Fx,
}

/// Манипулятор (`inserter` в data.raw), только то, что модель умеет: электрический, не bulk,
/// рука на 1 предмет, гоняется за предметами на ленте.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct InserterProto {
    pub name: String,
    /// `rotation_speed` (оборотов за тик) · 2^64 — точно (у `double` < 1 не больше 53 бит).
    pub rotation_speed: Turn,
    /// `extension_speed` (тайлов за тик), `double` как в прототипе (движок считает вылет в `double`).
    pub extension_speed: Sf,
    /// Длины `pickup_position` и `insert_position` в `double` (`sqrt(x² + y²)`) — цели вылета
    /// руки (проверено по эталону: у выгрузки рука на 1.2 тайла, а не на 307/256).
    pub pickup_len: Sf,
    pub insert_len: Sf,
    /// `pickup_position` для направления «север», тайлы Q48 (направление для угла руки).
    pub pickup_q48: (i64, i64),
    /// `insert_position` для направления «север», тайлы Q48.
    pub insert_q48: (i64, i64),
    /// Те же векторы в 1/256 тайла с усечением к нулю — точки захвата/выгрузки на карте
    /// (`insert_position {0, 1.2}` → 307: `LuaEntity.drop_position` манипулятора в 1.5 —
    /// 2.69921875 = 1.5 + 307/256; `scene_capture` сверяет точки с игрой в каждой сцене).
    pub pickup: (Fx, Fx),
    pub insert: (Fx, Fx),
    /// `starting_distance` — вылет руки только что построенного манипулятора, тайлы, `double`
    /// (в прототипах base: 0.7 по умолчанию, у длиннорукого 1.7).
    pub starting_distance: Sf,
}

/// Сундук (`container` в data.raw).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ChestProto {
    pub name: String,
    /// `inventory_size` — число слотов.
    pub slots: u32,
}

/// Отказ: прототип есть, но модель его не умеет (причина — для сообщения об ошибке сцены).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub name: String,
    pub why: String,
}

/// Прототипы, которые знает модель: всё из выжимки; неподдержанные — в `unsupported`.
#[derive(Clone, Debug, Default)]
pub struct Prototypes {
    /// Строка `_source` выжимки (версия игры и моды).
    pub source: String,
    pub belts: BTreeMap<String, BeltProto>,
    pub inserters: BTreeMap<String, InserterProto>,
    pub chests: BTreeMap<String, ChestProto>,
    pub stack_size: BTreeMap<String, u32>,
    pub unsupported: BTreeMap<String, String>,
}

type R<T> = Result<T, String>;

fn num(p: &Value, f: &str, who: &str) -> R<f64> {
    p[f].as_f64().ok_or(format!("{who}: нет числа {f}"))
}

fn vec2(p: &Value, f: &str, who: &str) -> R<(f64, f64)> {
    // MapPosition/Vector: [x, y] или {x, y}
    let v = &p[f];
    let (x, y) = if v.is_array() { (&v[0], &v[1]) } else { (&v["x"], &v["y"]) };
    match (x.as_f64(), y.as_f64()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(format!("{who}: плохой вектор {f}")),
    }
}

/// Сущность умещается в один тайл (модель M0 — только 1×1).
fn fits_one_tile(p: &Value, who: &str) -> R<bool> {
    let b = &p["collision_box"];
    let c = [&b[0][0], &b[0][1], &b[1][0], &b[1][1]];
    let mut v = [0.0f64; 4];
    for (k, x) in c.iter().enumerate() {
        v[k] = x.as_f64().ok_or(format!("{who}: плохой collision_box"))?;
    }
    Ok(v.iter().all(|x| x.abs() < 0.5))
}

/// Тайлы (f64) → Q48 с округлением. `x · 2^48` в `f64` точно.
fn q48(x: f64) -> i64 {
    (x * EXT_ONE as f64).round() as i64
}

/// Длина вектора в `double`: `sqrt(x·x + y·y)` (каждая операция — `double`).
fn len_f64(v: (f64, f64)) -> Sf {
    let (x, y) = (Sf::from_f64(v.0), Sf::from_f64(v.1));
    x.mul(x, P64).add(y.mul(y, P64), P64).sqrt(P64)
}

/// Тайлы (f64) → 1/256 с усечением к нулю (как точка выгрузки в игре, см. `InserterProto::insert`).
fn fx_trunc(x: f64) -> Fx {
    (x * TILE as f64).trunc() as Fx
}

impl Prototypes {
    /// Выжимка из репо (`data/prototypes-m0.json`), разобранная один раз.
    pub fn bundled() -> &'static Prototypes {
        static P: OnceLock<Prototypes> = OnceLock::new();
        P.get_or_init(|| {
            Prototypes::from_json(include_str!("../../data/prototypes-m0.json")).expect("data/prototypes-m0.json")
        })
    }

    pub fn load(path: &std::path::Path) -> R<Prototypes> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Prototypes::from_json(&s)
    }

    /// Разбор выжимки (формат `bin/proto_extract`).
    pub fn from_json(s: &str) -> R<Prototypes> {
        let d: Value = serde_json::from_str(s).map_err(|e| e.to_string())?;
        let mut out = Prototypes { source: d["_source"].as_str().unwrap_or("").to_string(), ..Default::default() };
        let obj = |k: &str| d[k].as_object().ok_or(format!("в выжимке нет раздела {k}"));

        for (name, p) in obj("transport-belt")? {
            let who = format!("transport-belt/{name}");
            let v = num(p, "speed", &who)? * TILE as f64;
            if v.fract() != 0.0 {
                out.unsupported.insert(name.clone(), format!("скорость {} не кратна 1/256 тайла", v / 256.0));
                continue;
            }
            // Нулевая и отрицательная скорость не имеют смысла; скорость больше шага между
            // предметами (64) модель не умеет (предмет перепрыгивал бы соседа).
            if !(v > 0.0 && v <= ITEM_SPACING as f64) {
                out.unsupported.insert(name.clone(), format!("скорость {} вне (0, {}] 1/256 тайла за тик", v, ITEM_SPACING));
                continue;
            }
            if !fits_one_tile(p, &who)? {
                out.unsupported.insert(name.clone(), "не 1×1".into());
                continue;
            }
            out.belts.insert(name.clone(), BeltProto { name: name.clone(), speed: Fx::try_from(v as i64).map_err(|e| format!("{who}: speed: {e}"))? });
        }

        for (name, p) in obj("inserter")? {
            let who = format!("inserter/{name}");
            let energy = p["energy_source"]["type"].as_str().unwrap_or("?");
            let bulk = p["bulk"].as_bool().unwrap_or(false);
            let bonus = p["stack_size_bonus"].as_u64().unwrap_or(0);
            let why = if energy != "electric" {
                Some(format!("источник энергии {energy} (модель — только электрический, всегда запитан)"))
            } else if bulk {
                Some("bulk (рука на несколько предметов) — не M0".into())
            } else if bonus != 0 {
                Some(format!("stack_size_bonus = {bonus} (рука > 1) — не M0"))
            } else if p["chases_belt_items"].as_bool() == Some(false) {
                Some("chases_belt_items = false — не смоделировано".into())
            } else if p["wait_for_full_hand"].as_bool() == Some(true) {
                Some("wait_for_full_hand — не M0".into())
            } else if !fits_one_tile(p, &who)? {
                Some("не 1×1".into())
            } else {
                None
            };
            if let Some(why) = why {
                out.unsupported.insert(name.clone(), why);
                continue;
            }
            let rs = num(p, "rotation_speed", &who)?;
            let es = num(p, "extension_speed", &who)?;
            if !(rs > 0.0 && rs < 1.0 && es > 0.0) {
                return Err(format!("{who}: скорости вне диапазона"));
            }
            let sd = num(p, "starting_distance", &who)?;
            if !(sd.is_finite() && (0.0..=64.0).contains(&sd)) {
                return Err(format!("{who}: starting_distance вне диапазона"));
            }
            let pk = vec2(p, "pickup_position", &who)?;
            let ins = vec2(p, "insert_position", &who)?;
            out.inserters.insert(
                name.clone(),
                InserterProto {
                    name: name.clone(),
                    rotation_speed: turn_from_f64(rs),
                    extension_speed: Sf::from_f64(es),
                    pickup_len: len_f64(pk),
                    insert_len: len_f64(ins),
                    pickup_q48: (q48(pk.0), q48(pk.1)),
                    insert_q48: (q48(ins.0), q48(ins.1)),
                    pickup: (fx_trunc(pk.0), fx_trunc(pk.1)),
                    insert: (fx_trunc(ins.0), fx_trunc(ins.1)),
                    starting_distance: Sf::from_f64(sd),
                },
            );
        }

        for (name, p) in obj("container")? {
            let who = format!("container/{name}");
            if !fits_one_tile(p, &who)? {
                out.unsupported.insert(name.clone(), "не 1×1".into());
                continue;
            }
            let slots = p["inventory_size"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or(format!("{who}: нет или плохой inventory_size"))?;
            out.chests.insert(name.clone(), ChestProto { name: name.clone(), slots });
        }

        for (name, v) in obj("item_stack_size")? {
            let ss = v
                .as_u64()
                .filter(|&s| s > 0)
                .and_then(|s| u32::try_from(s).ok())
                .ok_or(format!("item/{name}: плохой stack_size"))?;
            out.stack_size.insert(name.clone(), ss);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Выжимка из репо разбирается, и в ней то, что нужно M0, с числами `data.raw`.
    #[test]
    fn bundled_m0() {
        let p = Prototypes::bundled();
        assert!(p.source.contains("2.0.77"), "{}", p.source);
        assert_eq!(p.belts["transport-belt"].speed, 8); // speed 0.03125
        assert_eq!(p.belts["turbo-transport-belt"].speed, 32); // speed 0.125
        let i = &p.inserters["inserter"];
        assert_eq!(i.rotation_speed, 258_254_417_031_933_760); // 0.014000000000000002 · 2^64
        assert_eq!(i.extension_speed, Sf::from_f64(0.035));
        assert_eq!(i.insert_len, Sf::from_f64(1.2));
        assert_eq!((i.pickup, i.insert), ((0, -256), (0, 307)));
        let l = &p.inserters["long-handed-inserter"];
        assert_eq!((l.pickup, l.insert), ((0, -512), (0, 563))); // {0,-2}, {0,2.2}: 563.2 → 563
        assert_eq!((i.starting_distance, l.starting_distance), (Sf::from_f64(0.7), Sf::from_f64(1.7)));
        assert!(p.inserters.contains_key("fast-inserter"));
        for n in ["burner-inserter", "bulk-inserter", "stack-inserter"] {
            assert!(p.unsupported.contains_key(n), "{n}");
        }
        assert_eq!(p.chests["wooden-chest"].slots, 16);
        assert_eq!(p.stack_size["iron-plate"], 100);
        assert_eq!(p.stack_size["electronic-circuit"], 200);
    }

    /// Скорость ленты <= 0 или больше шага между предметами — unsupported, а не поддержанная лента.
    #[test]
    fn belt_speed_out_of_range_unsupported() {
        let belt = |speed: f64| {
            serde_json::json!({"collision_box": [[-0.4, -0.4], [0.4, 0.4]], "speed": speed})
        };
        let d = serde_json::json!({
            "_source": "test",
            "transport-belt": {"zero": belt(0.0), "neg": belt(-0.03125), "fast": belt(0.5), "ok": belt(0.25),
                               "frac": belt(0.03)},
            "inserter": {}, "container": {}, "item_stack_size": {},
        });
        let p = Prototypes::from_json(&d.to_string()).unwrap();
        assert_eq!(p.belts.keys().collect::<Vec<_>>(), vec!["ok"]);
        assert_eq!(p.belts["ok"].speed, 64);
        for n in ["zero", "neg", "fast", "frac"] {
            assert!(p.unsupported.contains_key(n), "{n}");
        }
    }

    /// Выжимка в репо совпадает с `data.raw` окружения (`$FD_ENV_DIR/dump-env/script-output/
    /// data-raw-dump.json`, не в репо). Нужно окружение замеров (bench/setup-env.ps1), поэтому
    /// тест в обычном прогоне пропущен: `cargo test -- --ignored` с заданным `FD_ENV_DIR`.
    #[test]
    #[ignore = "нужен data.raw из окружения замеров (FD_ENV_DIR)"]
    fn bundled_matches_data_raw() {
        let env = std::env::var("FD_ENV_DIR").expect("FD_ENV_DIR не задан");
        let path = std::path::Path::new(&env).join("dump-env/script-output/data-raw-dump.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let d: Value = serde_json::from_str(&text).unwrap();
        let p = Prototypes::bundled();
        for b in p.belts.values() {
            assert_eq!(d["transport-belt"][&b.name]["speed"].as_f64().unwrap() * 256.0, b.speed as f64, "{}", b.name);
        }
        for i in p.inserters.values() {
            let r = &d["inserter"][&i.name];
            assert_eq!(turn_from_f64(r["rotation_speed"].as_f64().unwrap()), i.rotation_speed, "{}", i.name);
            assert_eq!(Sf::from_f64(r["extension_speed"].as_f64().unwrap()), i.extension_speed, "{}", i.name);
            assert_eq!(Sf::from_f64(r["starting_distance"].as_f64().unwrap_or(0.7)), i.starting_distance, "{}", i.name);
        }
        for c in p.chests.values() {
            assert_eq!(d["container"][&c.name]["inventory_size"].as_u64().unwrap() as u32, c.slots, "{}", c.name);
        }
        assert_eq!(d["item"]["iron-plate"]["stack_size"].as_u64(), Some(p.stack_size["iron-plate"] as u64));
    }
}
