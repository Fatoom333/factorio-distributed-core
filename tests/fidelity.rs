//! Сверка модели мира M0 с эталонами из игры и тесты отката/детерминизма.
//!
//! - основные сцены `tests/fidelity/*.scene.json` (3600 тиков) — каждая отдельным тестом;
//! - пробные сцены `tests/fidelity/probes/` (300–600 тиков) — каждое направление и частный
//!   случай; тест падает на любом расхождении;
//! - известные расхождения — тесты с `#[ignore]` (видны в `cargo test -- --ignored`).
//!
//! Внешние сцены (не из репо) — тест `extra_scenes_report` (`#[ignore]`), переменная окружения
//! `FD_EXTRA_FIDELITY_DIR`: каждая `<name>.scene.json` с `<name>.dump.jsonl` рядом прогоняется и
//! печатается, без падения.

use std::path::{Path, PathBuf};

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

const SCENES: &[&str] = &["straight-belt", "chest-to-belt", "belt-to-chest-fast", "two-lanes", "long-express"];

/// Пробы, которые совпадают с игрой целиком.
const PROBES: &[&str] = &[
    "p1-deadend",
    "p2-dropwest",
    "p3-dropnorth",
    "p4-dropsouth",
    "p5-sinkperp-north",
    "p6-sinkaway",
    "p7-dense",
    "p8-c2c",
    "p9-order",
    "p10-chain",
    "p10b-chainrev",
    "p11-dropeast",
    "p12-mixed",
    "p13-twoitems",
    "p15-dense-express",
    "p16-dense-slow",
    "p17-belt2belt",
    "c3b-lastroom-ba",
    "c4b-chase-full",
    // порядок манипуляторов: последний разбуженный (наполнением сундука-источника) — первым
    "c1a-lastitem-ab",
    "c1b-lastitem-ba",
    "c1c-lastitem-ns",
    "c1d-lastitem-sn",
    "c3a-lastroom-ab",
    // два источника кладут рядом в одном тике; кладка, когда задний предмет ближе 64
    "t1a-sametick",
    "t1b-sametick-rev",
    "t1c-fillorder",
    "t1d-fillorder-rev",
    "t1e-chestsfirst",
    "t1f-chestsfirst-rev",
    "t2a-deadend",
    "t2b-deadend-rev",
    "t3a-behind-gap9",
    "t3b-behind-gap8",
    "t4a-fast-gap16",
    "t4b-fast-gap17",
    "t5-express-gap24",
    "t6a-push-gap0",
    "t6b-push-gap2",
    "t6c-push-gap17",
    // быстрый и длиннорукий манипуляторы (числа — из выжимки data.raw, закон руки — общий)
    "f1-fast-sink",
    "f2-fast-source",
    "l1-long-belt",
    "l2-long-chain",
    // кладка за хвост линии: без ленты сзади — прижимается к 255, с лентой сзади — ждёт
    "tail-b8-p200",
    "tail-b32-p250",
    "tail-b16-p200-two",
];

/// Пробы с известным расхождением (см. docs/model-fidelity.md).
const PROBES_KNOWN: &[&str] = &[
    // конкуренция стоков, берущих с ленты: их порядок задаёт пробуждение во время прогона
    // (не смоделировано)
    "c2a-onetile-ns",
    "c2b-onetile-sn",
    "c4-chase-full",
    // перезапуск сжатой очереди: хвост трогается на тик позже (тик 758), docs/model-fidelity.md
    "p20-restart-min",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fidelity")
}

/// Сверка одной сцены: `Err` с описанием первого расхождения.
fn compare(d: &Path, name: &str) -> Result<u64, String> {
    let scene = fidelity::load_scene(&d.join(format!("{name}.scene.json"))).map_err(|e| format!("{name}: {e}"))?;
    let dump = fidelity::load_dump(&d.join(format!("{name}.dump.jsonl"))).map_err(|e| format!("{name}: {e}"))?;
    if dump.len() as u64 != scene.ticks + 1 {
        return Err(format!("{name}: в эталоне {} строк, а не ticks + 1 = {}", dump.len(), scene.ticks + 1));
    }
    let r = fidelity::run(&scene, &dump).map_err(|e| format!("{name}: {e}"))?;
    if let Some(m) = &r.first_mismatch {
        return Err(format!("{name}: совпало {} из {} тиков; первое расхождение — {m}", r.matched, r.total_ticks));
    }
    assert_eq!(r.matched, r.total_ticks);
    Ok(r.matched)
}

fn check(name: &str) {
    if let Err(e) = compare(&dir(), name) {
        panic!("{e}");
    }
}

#[test]
fn straight_belt() {
    check("straight-belt");
}

#[test]
fn chest_to_belt() {
    check("chest-to-belt");
}

#[test]
fn belt_to_chest_fast() {
    check("belt-to-chest-fast");
}

#[test]
fn two_lanes() {
    check("two-lanes");
}

#[test]
fn long_express() {
    check("long-express");
}

/// Сжатая очередь, которую разбирает сток (подпор): в игре хвост очереди трогается на тик
/// позже модели. Эталон — пробная сцена p14 (быстрые ленты), расхождение на тике 836.
#[test]
#[ignore = "известное расхождение: перезапуск сжатой очереди"]
fn compressed_restart_fast() {
    check("compressed-restart-fast");
}

/// То же на 40 обычных лентах (проба p18), расхождение на тике 296.
#[test]
#[ignore = "известное расхождение: перезапуск сжатой очереди"]
fn compressed_restart_long() {
    check("compressed-restart-long");
}

#[test]
fn probes_match() {
    let d = dir().join("probes");
    let errs: Vec<String> = PROBES.iter().filter_map(|n| compare(&d, n).err()).collect();
    assert!(errs.is_empty(), "расхождения в пробах:\n{}", errs.join("\n"));
}

#[test]
#[ignore = "известные расхождения: порядок стоков с ленты при конкуренции (c2*, c4), перезапуск сжатой очереди (p20)"]
fn probes_known_divergences() {
    let d = dir().join("probes");
    let errs: Vec<String> = PROBES_KNOWN.iter().filter_map(|n| compare(&d, n).err()).collect();
    assert!(errs.is_empty(), "расхождения в пробах:\n{}", errs.join("\n"));
}

/// Каждая проба в папке — в одном из списков (новая проба не пропадёт молча).
#[test]
fn probes_all_listed() {
    let mut names: Vec<String> = std::fs::read_dir(dir().join("probes"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.strip_suffix(".scene.json")).map(String::from))
        .collect();
    names.sort();
    for n in &names {
        assert!(
            PROBES.contains(&n.as_str()) ^ PROBES_KNOWN.contains(&n.as_str()),
            "проба {n}: должна быть ровно в одном из списков PROBES / PROBES_KNOWN"
        );
    }
    assert_eq!(names.len(), PROBES.len() + PROBES_KNOWN.len(), "в списках есть пробы без файлов");
}

/// Отчёт по внешним сценам: `FD_EXTRA_FIDELITY_DIR=<папка> cargo test -- --ignored extra_scenes_report --nocapture`.
#[test]
#[ignore = "отчёт по внешним сценам: нужен FD_EXTRA_FIDELITY_DIR"]
fn extra_scenes_report() {
    let extra = std::env::var("FD_EXTRA_FIDELITY_DIR").expect("FD_EXTRA_FIDELITY_DIR не задан");
    let mut names: Vec<_> = std::fs::read_dir(&extra)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.strip_suffix(".scene.json")).map(String::from))
        .collect();
    names.sort();
    for name in names {
        match compare(Path::new(&extra), &name) {
            Ok(n) => println!("{name}: {n}/{n}"),
            Err(e) => println!("{e}"),
        }
    }
}

fn world(name: &str) -> (World, u64) {
    let scene = fidelity::load_scene(&dir().join(format!("{name}.scene.json"))).unwrap();
    (World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled()).unwrap(), scene.ticks)
}

/// Откат: снимок (`clone`) на тике k; ветка «на догадке» с другим входом (в сундук-источник
/// подложили предметы) уходит на 150 тиков и действительно расходится с прямым прогоном;
/// затем снимок восстанавливается и прогон идёт с настоящими входами до конца — состояние
/// байт в байт (JSON) равно прямому прогону.
///
/// Тест ловит только скрытое глобальное состояние: то, что не лежит в `World` и не
/// копируется `clone()` (статики, кэши, внешний счётчик), — тогда ветка на догадке оставила бы
/// след в повторном прогоне. Корректность самой модели он не проверяет: прямой прогон и повтор
/// считаются одним и тем же кодом.
#[test]
fn rollback_equals_straight_run() {
    for name in SCENES {
        let (w0, n) = world(name);
        assert!(!w0.chests.is_empty(), "{name}: нужен сундук для подмены входа");
        for k in [0u64, 1, 37, 300, 777, 2000] {
            let guess_end = (k + 150).min(n);
            let mut straight = w0.clone();
            let mut straight_at_guess_end = None;
            for t in 0..n {
                straight.step();
                if t + 1 == guess_end {
                    straight_at_guess_end = Some(straight.clone());
                }
            }

            let mut w = w0.clone();
            for _ in 0..k {
                w.step();
            }
            let snapshot = w.clone();
            // ветка на догадке: другой вход — в первый сундук подложили 7 предметов
            let item = 0; // первый предмет сцены
            let stack = w.item_stack[item as usize];
            assert_eq!(w.chests[0].inv.insert(item, 7, stack), 7);
            for _ in k..guess_end {
                w.step();
            }
            assert_ne!(
                Some(&w),
                straight_at_guess_end.as_ref(),
                "{name}: ветка на догадке не отличается от прямого прогона — тест ничего не проверяет"
            );
            // откат и повтор с настоящими входами
            let mut w = snapshot;
            for _ in k..n {
                w.step();
            }
            let a = serde_json::to_vec(&straight).unwrap();
            let b = serde_json::to_vec(&w).unwrap();
            assert_eq!(a, b, "сцена {name}: откат на тик {k} разошёлся с прямым прогоном");
        }
    }
}

/// Детерминизм: два независимо собранных мира после N шагов байт в байт равны.
#[test]
fn deterministic_from_scene() {
    for name in SCENES {
        let (mut a, n) = world(name);
        let (mut b, _) = world(name);
        for _ in 0..n {
            a.step();
            b.step();
        }
        assert_eq!(serde_json::to_vec(&a).unwrap(), serde_json::to_vec(&b).unwrap(), "{name}");
    }
}
