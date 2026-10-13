//! Выжимка прототипов для модели мира из `data.raw` игры (`factorio --dump-data` пишет
//! `script-output/data-raw-dump.json`). Результат — маленький JSON, который читает
//! `world::proto::Prototypes` (в репо лежит `data/prototypes-m0.json`).
//!
//!   cargo run --release --bin proto_extract -- --in <data-raw-dump.json> --out data/prototypes-m0.json \
//!       --source "factorio --dump-data, Factorio 2.0.77, моды: base space-age quality elevated-rails"
//!
//! Пересобрать под другой набор модов: снять `--dump-data` с этими модами и запустить заново.
//! Берутся все прототипы типов `transport-belt`, `underground-belt`, `inserter`, `container` и `stack_size` всех
//! предметов (любой тип с полем `stack_size`). Поля, которых нет в `data.raw`, записываются
//! значением по умолчанию из документации прототипов игры (`doc-html/prototypes/*.html`), чтобы
//! файл был самодостаточным. Что из этого модель умеет, решает `Prototypes` (неподдержанное —
//! ошибка сцены, не молчаливая подмена).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{json, Map, Value};

type R<T> = Result<T, Box<dyn std::error::Error>>;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Поле прототипа и его умолчание (JSON).
type Fields = &'static [(&'static str, Option<&'static str>)];

/// Поля прототипа с умолчаниями (`None` — обязательное поле). Источник умолчаний —
/// `doc-html/prototypes/<Тип>Prototype.html` той же версии игры.
const BELT_FIELDS: Fields = &[("speed", None), ("collision_box", Some("[[0,0],[0,0]]"))];
const UNDERGROUND_FIELDS: Fields =
    &[("speed", None), ("max_distance", None), ("collision_box", Some("[[0,0],[0,0]]"))];
const INSERTER_FIELDS: Fields = &[
    ("rotation_speed", None),
    ("extension_speed", None),
    ("pickup_position", None),
    ("insert_position", None),
    // EnergySource: берём только `type` (electric / burner / void)
    ("energy_source", None),
    ("bulk", Some("false")),
    ("stack_size_bonus", Some("0")),
    ("starting_distance", Some("0.7")),
    ("uses_inserter_stack_size_bonus", Some("true")),
    ("chases_belt_items", Some("true")),
    ("wait_for_full_hand", Some("false")),
    ("grab_less_to_match_belt_stack", Some("false")),
    ("max_belt_stack_size", Some("1")),
    ("collision_box", Some("[[0,0],[0,0]]")),
];
const CONTAINER_FIELDS: Fields =
    &[("inventory_size", None), ("collision_box", Some("[[0,0],[0,0]]"))];

fn pick(kind: &str, name: &str, p: &Value, fields: Fields) -> R<Value> {
    let mut o = Map::new();
    for (f, def) in fields {
        let v = match (p.get(*f), def) {
            (Some(v), _) => v.clone(),
            (None, Some(d)) => serde_json::from_str(d)?,
            (None, None) => return Err(format!("{kind}/{name}: нет поля {f}").into()),
        };
        let v = if *f == "energy_source" { json!({"type": v["type"]}) } else { v };
        o.insert((*f).to_string(), v);
    }
    Ok(Value::Object(o))
}

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(inp), Some(out), Some(source)) = (arg(&args, "--in"), arg(&args, "--out"), arg(&args, "--source")) else {
        return Err("нужно: --in <data-raw-dump.json> --out <файл> --source \"<версия игры и моды>\"".into());
    };
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&inp)?)?;
    let sections: [(&str, Fields); 4] = [
        ("transport-belt", BELT_FIELDS),
        ("underground-belt", UNDERGROUND_FIELDS),
        ("inserter", INSERTER_FIELDS),
        ("container", CONTAINER_FIELDS),
    ];

    // Формат: по строке на прототип — файл маленький и читается глазами / в diff.
    let mut s = String::new();
    s.push_str("{\n");
    writeln!(s, "  \"_source\": {},", Value::String(source))?;
    s.push_str(
        "  \"_note\": \"Выжимка data.raw (bin/proto_extract). Поля, которых нет в data.raw, — умолчания из doc-html/prototypes той же версии.\",\n",
    );
    for (kind, fields) in sections {
        let protos = d[kind].as_object().ok_or(format!("в data.raw нет {kind}"))?;
        let picked: BTreeMap<&String, Value> =
            protos.iter().map(|(n, p)| Ok((n, pick(kind, n, p, fields)?))).collect::<R<_>>()?;
        writeln!(s, "  \"{kind}\": {{")?;
        let n = picked.len();
        for (i, (name, v)) in picked.into_iter().enumerate() {
            writeln!(s, "    {}: {}{}", Value::String(name.clone()), v, if i + 1 < n { "," } else { "" })?;
        }
        s.push_str("  },\n");
    }
    // stack_size всех предметов (любой тип с полем stack_size).
    let mut items: BTreeMap<String, u64> = BTreeMap::new();
    for (_, protos) in d.as_object().ok_or("data.raw — не объект")? {
        let Some(protos) = protos.as_object() else { continue };
        for (name, p) in protos {
            if let Some(ss) = p.get("stack_size").and_then(Value::as_u64) {
                items.insert(name.clone(), ss);
            }
        }
    }
    s.push_str("  \"item_stack_size\": {\n");
    let n = items.len();
    for (i, (name, ss)) in items.into_iter().enumerate() {
        writeln!(s, "    {}: {ss}{}", Value::String(name), if i + 1 < n { "," } else { "" })?;
    }
    s.push_str("  }\n}\n");
    // проверка, что вышел валидный JSON
    let _: Value = serde_json::from_str(&s)?;
    std::fs::write(&out, s)?;
    println!("записано {out}");
    Ok(())
}
