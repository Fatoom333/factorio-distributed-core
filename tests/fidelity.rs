//! Сверка модели мира M0 с эталонами из игры и тесты отката/детерминизма.
//!
//! - основные сцены `tests/fidelity/*.scene.json` (3600 тиков) — каждая отдельным тестом;
//! - пробные сцены `tests/fidelity/probes/` (обычно 300–800 тиков) — каждое направление и частный
//!   случай; тест падает на любом расхождении;
//! - известные расхождения — тесты с `#[ignore]` (видны в `cargo test -- --ignored`).
//!
//! Внешние сцены (не из репо) — тест `extra_scenes_report` (`#[ignore]`), переменная окружения
//! `FD_EXTRA_FIDELITY_DIR`: каждая `<name>.scene.json` с `<name>.dump.jsonl` рядом прогоняется и
//! печатается, без падения.

use std::path::{Path, PathBuf};

use factorio_distributed_core::fidelity;
use factorio_distributed_core::world::World;

const SCENES: &[&str] = &[
    "straight-belt",
    "chest-to-belt",
    "belt-to-chest-fast",
    "two-lanes",
    "long-express",
    "compressed-restart-fast",
    "compressed-restart-long",
];

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
    // порядок полос при захвате: манипулятор почти прямо напротив ленты (отклонение 9.5°) берёт ближнюю
    "ins-lane-9deg",
    // два стока с двух лент в один сундук, третья лента уже остановилась; эталон снят в одиночку
    // (без других сцен на карте) — см. комментарий у ins-groups-solo-nsx ниже
    "ins-groups-solo-snx",
    // перезапуск сжатой очереди: линия нарезана взаимодействиями манипуляторов, хвост очереди
    // трогается на тик позже (тик 758)
    "p20-restart-min",
    // кладка ровно на край тайла (все направления ленты, боковые края, ось ленты)
    "edge-e-back",
    "edge-e-fwd",
    "edge-e-tail",
    "edge-e-tail-fast",
    "edge-lat-axis",
    "edge-lat-near",
    "edge-n-back",
    "edge-n-fwd",
    "edge-s-back",
    "edge-s-fwd",
    "edge-w-back",
    "edge-w-fwd",
    // линии лент на других сценах (набор j, bench/restart-probes/genj.py; эталоны укорочены):
    // лента на юг с источником на каждом тайле и стоком в конце; стык обычная → турбо → обычная
    // с очередью через оба стыка; кладка у самого края тайла (в_тайле 4, 253, 77, 1) в очередь
    "j-dir-s-y10",
    "j-mix-yty",
    "j-edge-mixed",
    // --- порядок, сон и пробуждение манипуляторов (docs/mechanics/inserters.md) ---
    // конкуренция стоков, берущих с ленты: два стока на одном тайле (оба порядка); сундук-приёмник
    // заполняет второй манипулятор во время погони
    "c2a-onetile-ns",
    "c2b-onetile-sn",
    "c4-chase-full",
    // порядок полос при захвате, когда манипулятор стоит не прямо напротив ленты: по диагонали
    // (юго-запад, юго-восток, то же на повёрнутой сцене) и под 26.6° / 11.3° — берёт левую полосу
    "ins-lane-diag-sw",
    "ins-lane-diag-se",
    "ins-lane-diag-rot",
    "ins-lane-27deg",
    "ins-lane-11deg",
    // предмет, положенный в этом тике, берётся в этом же тике, если берущий не спит и обновляется
    // позже положившего (лента; сундук)
    "ins-sametick-belt",
    "ins-sametick-chest",
    // один предмет в сундуке, два манипулятора разного типа: берёт тот, кто на прогреве раньше
    // довёл руку до точки захвата (быстрый / длиннорукий / обычный / обычный с дальней точкой)
    "ins-warm-fast-long",
    "ins-warm-long-basic",
    "ins-warm-basic-far",
    // сундук-приёмник заполнился во время погони за предметом: рука доезжает до предмета и только
    // потом бросает цель
    "ins-chase-full",
    // два манипулятора ждут места в полном сундуке, третий по одному забирает: кладут по очереди
    "ins-full-chest-wait",
    // два стока с двух лент в один сундук с местом на 1 предмет; третья лента к этому времени
    // остановилась. Эталоны ins-groups-solo-* сняты в одиночку: когда на карте идут другие сцены,
    // исход может быть другим (список активных групп линий общий на карту)
    "ins-groups-solo-nsx",
    // стоки в 4 тайлах от источника: на тике 485 линия делится на отрезки, стоки оказываются на
    // отдельном отрезке, засыпают между предметами, и их порядок меняется (видно с тика 572)
    "ins-cut-k4",
    // --- линии лент: срок перенарезки, постройка, выдыхание (docs/mechanics/belts.md) ---
    // задержка перенарезки: кладка через 62 / 124 тика после первой зажата передним концом
    // линии при hash mod 600 = 62 / 124 и не зажата при 63 / 125
    "cal-h062",
    "cal-h063",
    "cal-h124",
    "cal-h125",
    // сток посреди ленты, до которого предметы не доезжают, отмечает свой тайл и режет линию
    "sk-mid",
    // состояние после постройки (поле сцены `warmup` = 60): линии по одному тайлу, слияние по
    // самой ранней заявке; порядок постройки лент — от хвоста и от головы
    "build-ht-150",
    "build-th-150",
    // отметка взаимодействия выдыхается через 3000 тиков, линии сливаются обратно (3600 тиков)
    "decay-a-h003-d0003",
    // --- M0.5, хвосты правил лент и манипуляторов (bench/m05-probes) ---
    // предел линии в 200 тайлов: лента 206–207 тайлов, источники у границы линии (тайл 203 при
    // источнике на тайле 0, тайл 200 без него); модель без предела / с пределом 199 / 201 тайла
    // расходится с игрой на тиках 391 (lim-t-n206-a) и 251–259 (lim-t-n207-b) — проверено подменой
    "lim-t-n206-a",
    "lim-t-n207-b",
    // перезапись спящих манипуляторов при разрезе линии: порядок записи в списках сохраняется
    // (обратный порядок расходится с игрой на тиках 256–568)
    "sl-rewrite-a",
    "sl-rewrite-b",
    "sl-rewrite-c",
    "sl-rewrite-d",
    // побудка спящих при слиянии линий: слияние не оставляет их спать, порядок побудки — как в
    // модели (обратный порядок и «не будить» расходятся на тиках 46–218)
    "sl-merge-a",
    "sl-merge-b",
    "sl-merge-c",
    "sl-merge-d",
    "sl-merge-e",
    // то же в конце длинной съёмки: выдыхание отметок и слияние через 3000+ тиков при спящих
    "sl-decay-a",
    // порядок будильников внутри одной группы линий: два стока на разных отрезках одной ленты
    // просыпаются в одном тике и спорят за последнее место в сундуке; побеждает тот, чей будильник
    // позже в порядке обновления отрезков (модель), а не тот, чей отрезок ближе к голове
    // (порядок «голова → хвост» расходится с игрой на тиках 210 и 280)
    "ao-order-a",
    "ao-order-b",
    // кастомная точка выгрузки при отрицательной абсолютной координате: игра хранит её с
    // усечением к нулю (drop 1.2 → 308/256 от центра, а не 307/256); набор j, эталон укорочен
    "j-edge-f12-r",
    "edge-lat-far",
    "edge-w-tail",
    // манипулятор без цели (M0.5, docs/mechanics/inserters.md, «Манипулятор без цели»): нет источника,
    // выгрузка на землю, предмет с земли, занятое место; `nt-*` — по одному манипулятору, `ng*` —
    // земля между двумя (границы коробки 70/71 и тайл захвата — по обеим осям и в отрицательных
    // координатах)
    "nt-src-none-i",
    "nt-src-none-f",
    "nt-src-none-l",
    "nt-neither-i",
    "nt-drop-none-i",
    "nt-drop-none-f",
    "nt-drop-none-l",
    "nt-drop-ins",
    "nt-src-ins",
    "nt-belt-ground",
    "ng-x000",
    "ng-x070",
    "ng-x071",
    "ng-y036",
    "ng-y071",
    "ng-chain",
    "ng-belt-src",
    "ng-xm071",
    "nt-belt-dead",
    "ngc-12",
    "ngc-21",
    "ngn-chain",
    "ngn-x70",
    "ngn-x71",
    "ngn-y70",
    "ngn-y71",
    "ngp-xm100",
    "ngp-x080",
    "ngp-xm160",
    "ngp-xm200",
    "ngp-y100",
    "ngp-y128",
    "ngp-ym128",
    "ngq-a00",
    "ngq-b00",
    "ngq-c00",
    "ngq-d00",
    "ngq-d01",
];

/// Пробы с известным расхождением (см. docs/model-fidelity.md). Сейчас таких нет: `edge-lat-far` и
/// `edge-w-tail` (выгрузка на край тайла без ленты — это выгрузка на землю) совпали после модели
/// земли.
const PROBES_KNOWN: &[&str] = &[];

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

/// Сжатая очередь, которую разбирает сток (подпор): хвост очереди трогается на тик позже —
/// линия нарезана взаимодействиями манипуляторов (docs/mechanics/belts.md). Эталон — пробная
/// сцена p14 (быстрые ленты); до модели линий расходилась на тике 836.
#[test]
fn compressed_restart_fast() {
    check("compressed-restart-fast");
}

/// То же на 40 обычных лентах (проба p18); до модели линий расходилась на тике 296.
#[test]
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
#[ignore = "известные расхождения (сейчас список пуст)"]
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
        let lists = [PROBES, PROBES_KNOWN];
        assert_eq!(
            lists.iter().filter(|l| l.contains(&n.as_str())).count(),
            1,
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
            // у коротких сцен (compressed-restart-*: 900 и 400 тиков) поздние точки отката пропускаются
            if k >= n {
                continue;
            }
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
            assert_eq!(w.chest_insert(0, item, 7), 7);
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
