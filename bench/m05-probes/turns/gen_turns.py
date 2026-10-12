"""Генератор ручных проб M0.5: повороты и боковая загрузка. Сцены пишутся в каталог из argv[1].

    python gen_turns.py scenes
    fuzz_capture --scenes scenes --out <каталог эталонов> --batch 40

Сцена — как в tests/fidelity/README.md. Подробности геометрии — tl.py.
Полосы: lines[0] — левая по ходу, lines[1] — правая. Манипулятор кладёт на дальнюю от себя полосу:
с севера на ленту, идущую на восток, — на правую; с юга — на левую.
"""
import sys
from tl import *

OUT = sys.argv[1] if len(sys.argv) > 1 else "scenes"
SC = []


def add(sc):
    SC.append(sc)
    sc.write(OUT)


def dense(sc, x0, y, sides="NS", step=2, n=2, items=None):
    """Быстрые манипуляторы на тайлах x0, x0+step, ... по обе стороны (N: железо/уголь, S: медь/камень)."""
    items = items or {}
    for k in range(n):
        for s in sides:
            sc.feed(x0 + k * step, y, s, items.get(s), ins="fast-inserter")


def sparse(sc, x0, y, sides="NS"):
    for s in sides:
        sc.feed(x0, y, s)


def chain(sc, x, y, spec, feed_sides):
    """Лента-цепочка с питанием на первом тайле (быстрые манипуляторы с указанных сторон)."""
    sc.path(x, y, spec)
    for s in feed_sides:
        sc.feed(x, y, s, ins="fast-inserter")


MAIN_IT = {"N": "coal", "S": "stone"}

# ---------------------------------------------------------------- A. одиночные повороты
for kind in "yrbt":
    for turn, spec in (("l", "E7 N8"), ("r", "E7 S8")):
        for mode in ("sp", "dn"):
            sc = Sc(f"tt-turn-{turn}-{kind}-{mode}", 800, kind)
            if mode == "sp":
                sc.path(0, 0, spec)
                sparse(sc, 1, 0)
            else:
                sc.path(0, 0, spec.replace("8", "4"))
                dense(sc, 1, 0)
            add(sc)

# ---------------------------------------------------------------- B. S и U
for kind in "yr":
    for nm, spec in (("lr", "E5 N4 E5"), ("rl", "E5 S4 E5"), ("lr1", "E5 N1 E5"), ("rl1", "E5 S1 E5")):
        sc = Sc(f"tt-s-{nm}-{kind}", 900, kind)
        sc.path(0, 0, spec)
        dense(sc, 1, 0)
        add(sc)

# U-развороты: строка 0 идёт на восток, возвращается на запад; питание на тайлах 0 и 2 (возврат занимает x >= 4)
for nm, kind, turn, g in (("l-g1-y", "y", "N", 1), ("l-g2-y", "y", "N", 2), ("l-g1-b", "b", "N", 1), ("l-g3-r", "r", "N", 3),
                          ("r-g1-y", "y", "S", 1), ("r-g2-b", "b", "S", 2)):
    sc = Sc(f"tt-u-{nm}", 900, kind)
    sc.path(0, 0, f"E8 {turn}{g} W5")
    dense(sc, 0, 0, step=2, n=2)
    add(sc)

# ---------------------------------------------------------------- C. кольца
for nm, kind, spec, ticks in (("cw-y", "y", "E5 S5 W5 N5", 1200), ("ccw-y", "y", "E5 N5 W5 S5", 1200),
                              ("cw-b", "b", "E5 S5 W5 N5", 1200), ("ccw-b", "b", "E5 N5 W5 S5", 1200),
                              ("cw-small-y", "y", "E3 S3 W3 N3", 1000), ("ccw-small-r", "r", "E3 N3 W3 S3", 1000)):
    sc = Sc(f"tt-ring-{nm}", ticks, kind)
    sc.path(0, 0, spec)
    out_side = "N" if nm.startswith("cw") else "S"
    in_side = "S" if nm.startswith("cw") else "N"
    if "small" in nm:
        sc.feed(1, 0, out_side, ins="fast-inserter")
        sc.feed(2, 0, in_side, ins="fast-inserter")
    else:
        sc.feed(1, 0, out_side, ins="fast-inserter")
        sc.feed(3, 0, in_side, ins="fast-inserter")
    add(sc)

sc = Sc("tt-ring-cw-sink-y", 1200, "y")
sc.path(0, 0, "E5 S5 W5 N5")
sc.feed(1, 0, "N", ins="fast-inserter"); sc.feed(3, 0, "S", ins="fast-inserter")
sc.sink(3, 5, "S")
add(sc)
sc = Sc("tt-ring-ccw-sink-b", 1200, "b")
sc.path(0, 0, "E5 N5 W5 S5")
sc.feed(1, 0, "S", ins="fast-inserter"); sc.feed(3, 0, "N", ins="fast-inserter")
sc.sink(3, -5, "N")
add(sc)


# полные кольца: 8 быстрых манипуляторов, кольцо забивается целиком
for nm, kind, spec, out_side, in_side in (("cw-full-y", "y", "E5 S5 W5 N5", "N", "S"), ("ccw-full-b", "b", "E5 N5 W5 S5", "S", "N")):
    sc = Sc(f"tt-ring-{nm}", 1200, kind)
    sc.path(0, 0, spec)
    for x in (1, 2, 3, 4):
        sc.feed(x, 0, out_side, ins="fast-inserter")
        sc.feed(x, 0, in_side, ins="fast-inserter")
    add(sc)

# повороты со стоком в конце пути (поток через поворот не стоит)
for kind in "yb":
    for turn, spec, ex, ey, ed in (("l", "E7 N4", 7, -3, "north"), ("r", "E7 S4", 7, 3, "south")):
        sc = Sc(f"tt-turn-{turn}-{kind}-drain", 900, kind)
        sc.path(0, 0, spec)
        dense(sc, 1, 0)
        sc.end_sink(ex, ey, ed)
        add(sc)

# ---------------------------------------------------------------- D. боковая загрузка
def side_scene(name, main_kind="y", side_kind="y", sides="N", flow=True, jam=False, drain=False, join=6, length=12,
               behind=True, ticks=900):
    """Основная лента на восток по y = 0, боковые цепочки в тайл join с севера (S5) и/или юга (N5)."""
    sc = Sc(name, ticks, main_kind)
    if jam:
        length = join + 3
    if behind:
        sc.path(0, 0, f"E{length}")
    else:
        sc.path(join, 0, f"E{length - join}")
    if flow and behind:
        dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
    for s in sides:
        if s == "N":
            chain(sc, join, -5, f"S5:{side_kind}", "EW")
        else:
            chain(sc, join, 5, f"N5:{side_kind}", "EW")
    if drain:
        sc.end_sink(length - 1, 0, "east")
    return sc


for sides, sn in (("N", "n"), ("S", "s"), ("NS", "both")):
    add(side_scene(f"tt-side-{sn}-empty", sides=sides, flow=False))
    add(side_scene(f"tt-side-{sn}-flow", sides=sides, flow=True))
    add(side_scene(f"tt-side-{sn}-jam", sides=sides, flow=True, jam=True, join=3, ticks=800))
    add(side_scene(f"tt-side-{sn}-jamsink", sides=sides, flow=True, jam=True, drain=True, join=3, ticks=1000))
add(side_scene("tt-side-both-nobehind", sides="NS", flow=False, behind=False, length=12))
# в последний тайл
for sides, sn in (("N", "n"), ("S", "s"), ("NS", "both")):
    add(side_scene(f"tt-side-last-{sn}-empty", sides=sides, flow=False, join=5, length=6))
    add(side_scene(f"tt-side-last-{sn}-flow", sides=sides, flow=True, join=5, length=6))

# перед поворотом, после поворота, на повороте
sc = Sc("tt-side-preturn-n", 900); sc.path(0, 0, "E6 N3"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 5, -6, "S5", "EW"); add(sc)
sc = Sc("tt-side-preturn-s", 900); sc.path(0, 0, "E6 N3"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 5, 6, "N5", "EW"); add(sc)
sc = Sc("tt-side-postturn-w", 900); sc.path(0, 0, "E6 N5"); dense(sc, 0, 0, sides="S", step=1, n=2, items=MAIN_IT)
chain(sc, -4, -1, "E10", "NS"); add(sc)
sc = Sc("tt-side-postturn-e", 900); sc.path(0, 0, "E6 N5"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 16, -1, "W10", "NS"); add(sc)
sc = Sc("tt-onturn-2side", 900); sc.path(0, 0, "E6 N4"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 16, 0, "W10", "NS"); add(sc)
sc = Sc("tt-onturn-3way", 900); sc.path(0, 0, "E6"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 6, 6, "N11", "EW"); chain(sc, 16, 0, "W10", "NS"); add(sc)
# боковая цепочка сама идёт через поворот
sc = Sc("tt-side-viaturn-n", 900); sc.path(0, 0, "E12"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 2, -4, "E4 S4", "NS"); add(sc)
sc = Sc("tt-side-viaturn-s", 900); sc.path(0, 0, "E12"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
chain(sc, 2, 4, "E4 N4", "NS"); add(sc)

# ---------------------------------------------------------------- E. разные скорости
for a, b in (("y", "b"), ("b", "y"), ("y", "t"), ("t", "y"), ("r", "b"), ("b", "r"), ("y", "r"), ("r", "y")):
    sc = Sc(f"tt-mix-{a}{b}", 900, a)
    sc.path(0, 0, f"E6 E6:{b}")
    dense(sc, 0, 0, step=1, n=2)
    add(sc)
for a, b in (("y", "r"), ("r", "y")):
    for turn, ax in (("l", "N"), ("r", "S")):
        sc = Sc(f"tt-mixturn-{a}{b}-{turn}", 900, a)
        sc.path(0, 0, f"E6 {ax}6:{b}")
        dense(sc, 0, 0, step=1, n=2)
        add(sc)
# боковая загрузка между скоростями (основная лента и боковая)
for mk, sk in (("b", "y"), ("y", "b"), ("t", "y"), ("y", "t"), ("r", "y"), ("y", "r")):
    add(side_scene(f"tt-sidemix-{sk}-into-{mk}", main_kind=mk, side_kind=sk, sides="N", flow=True, ticks=900))
    add(side_scene(f"tt-sidemix-{sk}-into-{mk}-jam", main_kind=mk, side_kind=sk, sides="S", flow=True, jam=True,
                   drain=True, join=3, ticks=900))

# ---------------------------------------------------------------- F. манипуляторы на повороте
# левый поворот E->N: внутренняя полоса — левая (1), питается с юга; правый E->S: внутренняя — правая (2), с севера
for turn, spec, free in (("l", "E5 N5", ("E", "S")), ("r", "E5 S5", ("E", "N"))):
    inner_feed = "S" if turn == "l" else "N"
    outer_feed = "N" if inner_feed == "S" else "S"
    for lanes, fs in (("inner", inner_feed), ("outer", outer_feed), ("both", "NS")):
        for fside in free:
            sc = Sc(f"tt-sink-{turn}-{lanes}-{fside.lower()}", 800, "y")
            sc.path(0, 0, spec)
            for s in fs:
                sc.feed(1, 0, s, ins="fast-inserter"); sc.feed(3, 0, s, ins="fast-inserter")
            sc.sink(5, 0, fside)
            add(sc)
    sc = Sc(f"tt-sink-{turn}-both-fast", 800, "y")
    sc.path(0, 0, spec)
    for s in "NS":
        sc.feed(1, 0, s, ins="fast-inserter"); sc.feed(3, 0, s, ins="fast-inserter")
    sc.sink(5, 0, free[0], ins="fast-inserter")
    add(sc)
    # кладка на поворот (без потока / с потоком по ленте)
    for fside in free:
        for flow in (False, True):
            sc = Sc(f"tt-place-{turn}-{fside.lower()}{'-flow' if flow else ''}", 800, "y")
            sc.path(0, 0, spec)
            sc.feed(5, 0, fside, ins="fast-inserter")
            if flow:
                sc.feed(1, 0, "N", "coal", ins="fast-inserter"); sc.feed(1, 0, "S", "stone", ins="fast-inserter")
            add(sc)
# сток на тайле слияния (боковая загрузка)
for sd, sink_side in (("N", "S"), ("S", "N")):
    sc = Sc(f"tt-sink-sidejoin-{sd.lower()}", 800, "y")
    sc.path(0, 0, "E12"); dense(sc, 0, 0, step=1, n=2, items=MAIN_IT)
    if sd == "N":
        chain(sc, 6, -5, "S5", "EW")
    else:
        chain(sc, 6, 5, "N5", "EW")
    sc.sink(6, 0, sink_side)
    add(sc)

# ---------------------------------------------------------------- G. длинные линии (змейка > 200 тайлов)
SNAKE = "E124 S3 W124 S3 E124 S3 W20"
for nm, kind, idx, side, ticks in (("t", "t", 3, "NS", 1200), ("b", "b", 100, "N", 1200), ("y", "y", 190, "N", 1200),
                                   ("r", "r", 100, "N", 1000)):
    sc = Sc(f"tt-snake-{nm}", ticks, kind)
    tiles = sc.path(-62, -8, SNAKE)
    x, y, d = tiles[idx]
    for s in side:
        sc.feed(x, y, s, ins="fast-inserter")
    add(sc)

print(len(SC), "сцен")
