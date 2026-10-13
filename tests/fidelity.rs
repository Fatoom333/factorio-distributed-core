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
use factorio_distributed_core::world::proto::Prototypes;
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
    // M0.5 п. 4: те же два стока ao-order, но в цепочку добавлена третья пара (источник на тайле 12, сток на 15);
    // в тиках 115 и 185 в одной группе звонят три будильника, порядок пары-спорщиков тот же (подмена «голова → хвост»
    // расходится на 210 и 280)
    "ao3-a",
    "ao3-b",
    // M0.5 п. 5: два предмета на земле в тайле захвата безисточникового манипулятора. Берётся предмет с наименьшим y,
    // при равном y — с наименьшим x (gr: восемь диагональных/осевых пар; gs: несимметричные расстояния до точки
    // захвата и малые разности y; gz: то же в отрицательных координатах); sy: два предмета положены в один тик
    // идущему вслед манипулятору — порядок появления роли не играет. gl/gm/gn: цель выбирается один раз в первый тик
    // фазы взятия, позже появившийся предмет её не меняет (граница между 44 и 51 тиком при начале фазы на 49-м)
    "gr-tl-br-f1",
    "gr-br-tl-f1",
    "gr-l-r-f1",
    "gr-r-l-f1",
    "gr-t-b-f1",
    "gr-b-t-f1",
    "gr-tr-bl-f1",
    "gr-bl-tr-f1",
    "gs-d1-f1",
    "gs-d2-f1",
    "gs-d3-f1",
    "gs-d4-f1",
    "gs-d5-f1",
    "gs-d6-f1",
    "gz-d1-f1",
    "gz-d3-f1",
    "gz-d5-f1",
    "sy-il",
    "sy-ir",
    "gl-k0",
    "gl-k1",
    "gl-k2",
    "gl-k3",
    "gl-k4",
    "gl-k5",
    "gm-k0a",
    "gm-k1c",
    "gn-k3a",
    "gn-k4c",
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
    // M0.5 п. 7: рельс без вагона в тайле захвата/выгрузки — манипулятор «ждёт поезд» (rl-far — контроль)
    "rl-drop-h",
    "rl-drop-v",
    "rl-drop-empty",
    "rl-drop-e",
    "rl-drop-n",
    "rl-drop-fast",
    "rl-drop-long",
    "rl-pick-h",
    "rl-pick-v",
    "rl-pick-long",
    "rl-far",
    "rl-two",
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
    // M0.5, повороты и боковая загрузка (bench/m05-probes/turns; эталоны укорочены, docs/mechanics/belts.md):
    // поворот направо, синие ленты, сток в конце — длины полос поворота 106/295 (длина 107 расходится на тике 50)
    "tt-turn-r-b-drain",
    // кольцо 4×4 по часовой — одна линия на полосу, голова — тайл, построенный первым (голова с наибольшим
    // ключом расходится на тике 130)
    "tt-ring-cw-small-y",
    // боковая загрузка в пустую ленту: точки 67/188 от переднего края приёмника (66 расходится на тике 157)
    "tt-side-n-empty",
    // две боковые загрузки с обеих сторон в забитую ленту со стоком: спор за освободившееся место — побудка
    // боковых входов, когда на приёмнике появилось место (без неё — тик 394), сильная отметка, граница через
    // один тайл (слабая — тик 461)
    "tt-side-both-jamsink",
    // поворот другого типа (жёлтая → красный поворот): разные скорости на прямом стыке
    "tt-mixturn-yr-l",
    // сток берёт с углового тайла с обеих полос: погоня руки по дуге поворота (точка как на прямой —
    // тик 61)
    "tt-sink-r-both-n",
    // фаззер (сид 9101, сцена 202): боковая загрузка с другой скоростью, остаток хода усекается до 0 —
    // берётся ход приёмника (с 0 — тик 102)
    "tt-fz-side-adv0",
    // фаззер (сид 9102, сцена 175): при выходе вбок предмет источника доезжает до края, хвост приёмника
    // его не ограничивает (с ограничением — тик 118)
    "tt-fz-side-notail",
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
    // --- M0.5, будильники выгрузки (docs/mechanics/inserters.md, «Порядок, сон и пробуждение») ---
    // снимать по одной сцене; в скобках — тик, с которого расходится модель с подменённым порядком
    // два кладущих в одну точку ленты (зеркальные векторы, разные предметы), лента забита, сток под точкой
    // выгрузки — звонок сразу: первым кладёт раньше заснувший (подмена «будить с начала списка» — 354, 354, 139)
    "da-line-imm-ab",
    "da-line-imm-ba",
    "da-line-imm-fa",
    // то же, сток в конце ленты на другой линии — звонок отложенный, после движения лент (687, 356, 160)
    "da-line-def-ab",
    "da-line-def-fb",
    "da-line-def-ff",
    // три кладущих в одну точку: разные типы и порядок постройки, быстрая лента (408, 217, 125, 105)
    "da-line3-mid-bbb",
    "da-line3-mid-r",
    "da-line3-end-fbf",
    "da-line3-end-fastbelt",
    // три манипулятора ждут места в полном сундуке, четвёртый забирает по одному: общий порядок засыпания
    // у спящих с предметом и с пустой рукой (37, 72, 72, 14)
    "da-chest-bbb",
    "da-chest-fbb",
    "da-chest-bfb",
    "da-chest-fff",
    // два кладущих на две полосы одной ленты, общий сундук-источник, спор за последний предмет: отложенные
    // звонки двух групп идут по списку активных групп (обратный порядок групп — 602, 602, 634)
    "da-lanes-n4-k49-ns",
    "da-lanes-n4-k49-sn",
    "da-lanes-n5-k57-ns",
    // два кладущих в одну точку на земле, третий забирает: сна нет; когда ждут оба, место достаётся
    // построенному позже (в `-fb` — только с тика 151); подмены порядка звонков на исход не влияют;
    // предметы на земле в дамп не попадают — сверяется по рукам и сундуку третьего
    "da-ground-ff-ab",
    "da-ground-ff-ba",
    "da-ground-bb-ab",
    "da-ground-fb",
    // 3–4 кладущих на разных линиях одной полосы, два звонка выгрузки в одной группе в одном тике: совпали,
    // но порядок линий внутри группы на исход не влияет (у каждого свой сундук, спорить не за что)
    "da-chain-0246",
    "da-chain-b45",
    // то же с общими сундуками и лентой из отрезков разных типов: порядок двух звонков выгрузки в одной
    // группе — порядок обновления линий (обратный порядок звонков выгрузки внутри группы — 90, 432, 159,
    // 438, 233, 155, 366, 545, 154)
    "da-lines-a",
    "da-lines-b",
    "da-lines-c",
    "da-lines-d",
    "da-lines-e",
    "da-lines-f",
    "da-lines-g",
    "da-lines-h",
    "da-lines-i",
    // два звонка выгрузки и звонок захвата в одной группе в одном тике: звонок захвата идёт последним
    // (подмена «сначала звонки захвата» — 348)
    "da-mix-a",
    // --- M0.5, границы условия «есть место на линии» (docs/mechanics/belts.md) ---
    // передний зазор ровно «скорость + 64» после того, как сток снял первый предмет линии: места нет,
    // спящий кладущий не просыпается (нестрогая граница расходится с тика T+1 или T+5); линии скоростей
    // 8, 16 и 32, сдвиг очереди на «скорость» дают повороты ниже по течению
    "rm-fg-p50",
    "rm-fg-p56",
    "rm-fg-p63",
    "rm-fg-p70",
    "rm-fg-p60-ex",
    "rm-fg-p52-tu",
    "rm-fg16-p56",
    "rm-fg16-p63",
    "rm-fg32-p70",
    "rm-fg32-p76",
    // положительный контроль: после снятия первого предмета передний зазор больше «скорость + 64»
    // (79, 89, 79 для скорости 8; 108 для 16; 107, 107 для 24; 108 для 32) — спящий разбужен и кладёт в
    // T+1 (модель без члена переднего зазора расходится с тика T+1: 348, 349, 290, 328, 372, 314, 328)
    "rm-pos-p23",
    "rm-pos-p45",
    "rm-pos-d5-p23",
    "rm-pos16-p60",
    "rm-pos24-p67",
    "rm-pos24-d5-p67",
    "rm-pos32-p80",
    // хвостовой зазор ровно 64 при пустой линии сзади (тиком позже снятия): места нет
    // (нестрогая граница расходится с тика T+1)
    "rm-sum-p50",
    "rm-sum-p55",
    "rm-sum-p56",
    "rm-sum-p63",
    "rm-sum-p60-ex",
    "rm-sum-p52-tu",
    "rm-sum16-p56",
    "rm-sum16-p63",
    // контроль: та же сцена, хвост не заполнен — место есть по хвостовому зазору, манипулятор кладёт в T+1
    "rm-ctl-p56",
    "rm-ctl-u-p56",
    "rm-ctl16-p56",
    "rm-ctl16-u-p56",
    // --- M0.5, подземные ленты (docs/mechanics/belts.md, «Подземные ленты») ---
    // первая серия (bench/m05-probes/gen_underground.py): длина пары, направления, очередь, боковая загрузка, манипуляторы, без пары, стыки
    "ug-d1",
    "ug-d3",
    "ug-d5",
    "ug-d6-nopair",
    "ug-fast-d7",
    "ug-turbo-d11",
    "ug-dir-s",
    "ug-dir-n",
    "ug-queue",
    "ug-restart",
    "ug-side-in-l",
    "ug-side-in-r",
    "ug-side-out-l",
    "ug-side-out-r",
    "ug-take-in",
    "ug-drop-in",
    "ug-drop-in-back",
    "ug-drop-out",
    "ug-lone-in",
    "ug-lone-out",
    "ug-mix-tier",
    "ug-chain2",
    "ug-nest",
    "ug-block-same",
    "ug-out-side",
    // серия разбора правил: линии и очереди, захват, отметки, поиск пары, без пары, боковая загрузка, края тайла и «крыша»
    // (сцены edge-*, weak-*, strong-* — с кастомными точками выгрузки)
    "ug-r-behind-out",
    "ug-r-c-d2",
    "ug-r-c-d5",
    "ug-r-c-f4",
    "ug-r-c-out",
    "ug-r-edge-e-in-axis",
    "ug-r-edge-e-in-front",
    "ug-r-edge-e-in-fronthalf",
    "ug-r-edge-e-in-rear",
    "ug-r-edge-e-out-front",
    "ug-r-edge-e-out-rear",
    "ug-r-edge-n-in-front",
    "ug-r-edge-n-in-rear",
    "ug-r-edge-n-out-axis",
    "ug-r-edge-n-out-front",
    "ug-r-edge-n-out-rear",
    "ug-r-edge-s-in-front",
    "ug-r-edge-s-in-rear",
    "ug-r-edge-s-out-front",
    "ug-r-edge-s-out-rear",
    "ug-r-edge-s-out-rearhalf",
    "ug-r-edge-w-in-front",
    "ug-r-edge-w-in-mid",
    "ug-r-edge-w-in-rear",
    "ug-r-edge-w-out-front",
    "ug-r-edge-w-out-mid",
    "ug-r-edge-w-out-rear",
    "ug-r-headon-in",
    "ug-r-lane-e-in-l",
    "ug-r-lane-e-in-r",
    "ug-r-lane-n-out-r",
    "ug-r-lane-s-out-l",
    "ug-r-lane-s-out-r",
    "ug-r-lane-w-in-r",
    "ug-r-mix-32-8-24",
    "ug-r-mix-8-16-8",
    "ug-r-out-to-side",
    "ug-r-out-to-turn",
    "ug-r-pair-back-out",
    "ug-r-pair-cross-same",
    "ug-r-pair-exp-far10",
    "ug-r-pair-exp-max9",
    "ug-r-pair-far6",
    "ug-r-pair-fast-far8",
    "ug-r-pair-in-in-out",
    "ug-r-pair-in-out-out",
    "ug-r-pair-interleave",
    "ug-r-pair-max5",
    "ug-r-pair-other-tier",
    "ug-r-pair-over-belt",
    "ug-r-pair-turbo-far12",
    "ug-r-pick-both",
    "ug-r-pick-in",
    "ug-r-pick-out",
    "ug-r-side-in-left",
    "ug-r-side-in-left-fast",
    "ug-r-side-in-left-str",
    "ug-r-side-in-right",
    "ug-r-side-in-right-d4",
    "ug-r-side-out-left",
    "ug-r-side-out-left-slow",
    "ug-r-side-out-right",
    "ug-r-side-out-right-str",
    "ug-r-solo-in",
    "ug-r-solo-out",
    "ug-r-strong-d1",
    "ug-r-strong-d2",
    "ug-r-weak-d2",
    "ug-r-weak-d3",
    // вторая серия (тот же генератор, `probes_x`): поворот во вход, выход в бок подземки, два боковых входа, сильная отметка
    // с очередью, кольцо через подземку, сон манипуляторов на тайлах подземки, захват наискосок, отметка перед входом
    "ug-x-drop-in-q",
    "ug-x-lane-diag-in-n",
    "ug-x-lane-diag-in-s",
    "ug-x-lane-diag-out-n",
    "ug-x-lane-diag-out-s",
    "ug-x-out-into-in",
    "ug-x-out-into-out",
    "ug-x-ring",
    "ug-x-ring-full",
    "ug-x-ring-rev",
    "ug-x-side-in-both",
    "ug-x-side-in-both-back",
    "ug-x-side-in-both-q",
    "ug-x-side-in-q-sink",
    "ug-x-side-out-q-sink",
    "ug-x-take-in-out",
    "ug-x-take-out-q",
    "ug-x-turn-into-in",
    "ug-x-turn-into-in-q",
    "ug-x-weak-before-d1",
    "ug-x-weak-before-d2",
    "ug-x-weak-before-d3",
    // третья серия (`probes_y`): вход подземки не «питает» ленту перед собой (форма этой ленты);
    // кольцо, в котором первой построена подземка (вход или выход)
    "ug-y-in-no-feed",
    "ug-y-in-no-feed-lone",
    "ug-y-in-no-feed-side",
    "ug-y-ring-in-first",
    "ug-y-ring-in-first-full",
    "ug-y-ring-out-first",
    "ug-y-ring-out-first-full",
];

/// Пробы с известным расхождением (см. docs/model-fidelity.md). Сейчас таких нет: `edge-lat-far` и
/// `edge-w-tail` (выгрузка на край тайла без ленты — это выгрузка на землю) совпали после модели
/// земли, пробы подземных лент `ug-*` — после модели подземок.
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

/// Геометрия модели против чисел игры: ответы операции мода `scene_info`, сохранённые в
/// `tests/fidelity/info/` (сцены — одноимённые пробы). Сверяются позиции, точки полос лент (в том
/// числе на повороте), вид, пара и длины линий подземок — то же, что `scene_capture` сверяет на
/// живой игре перед съёмкой.
#[test]
fn geometry_matches_scene_info() {
    let mut n = 0;
    for e in std::fs::read_dir(dir().join("info")).unwrap().filter_map(|e| e.ok()) {
        let file = e.file_name().to_str().unwrap().to_string();
        let Some(name) = file.strip_suffix(".info.json") else { continue };
        let scene = fidelity::load_scene(&dir().join("probes").join(format!("{name}.scene.json"))).unwrap();
        let world = World::from_scene(&scene, Prototypes::bundled()).unwrap();
        let info: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
        if let Err(err) = fidelity::check_geometry(&world, &info) {
            panic!("{name}: {err}");
        }
        n += 1;
    }
    assert!(n >= 3, "в tests/fidelity/info нет ответов scene_info");
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

/// Вагон уезжает (мод: сцена с `until`, только в мод-пробах — модель вагонов не считает), пока
/// манипулятор кладёт в вагон: после тика `at` в тайле выгрузки остаётся рельс без вагона.
/// Модель: до `at` в тайле выгрузки земля (для снимка без разницы — сундук и рука те же), в
/// тик `at` цель меняется на `Train`. Эталоны — `tests/fidelity/wagon/` (манипулятор с предметом
/// в руке доезжает до точки выгрузки и стоит; пустая рука стоит в точке захвата).
#[test]
fn train_leaves_while_inserter_holds() {
    use factorio_distributed_core::world::inserter::Target;
    let scene = r#"{"name": "rl-leave", "ticks": 220, "entities": [
        {"name": "wooden-chest", "position": [0.5, 0.5], "items": {"iron-plate": 5}},
        {"name": "inserter", "position": [1.5, 0.5], "direction": "west"}]}"#;
    let scene = factorio_distributed_core::world::layout::Scene::from_json(scene).unwrap();
    for at in [5usize, 20, 36, 105] {
        let dump = fidelity::load_dump(&dir().join("wagon").join(format!("rl-leave-ul-{at}.dump.jsonl"))).unwrap();
        let mut w = World::from_scene(&scene, factorio_distributed_core::world::proto::Prototypes::bundled()).unwrap();
        for (i, want) in dump.iter().enumerate() {
            if i > 0 {
                w.step();
            }
            if i == at {
                w.inserters[0].drop = Target::Train;
            }
            // снимок тика `at` в игре сделан после ухода вагона, но состояние рук и сундуков от
            // этого не меняется — сравниваем целиком
            if let Some(d) = fidelity::diff(want, &w.dump()) {
                panic!("уход вагона на тике {at}, тик {i}: {d}");
            }
        }
    }
}
