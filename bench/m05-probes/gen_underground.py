#!/usr/bin/env python3
"""Пробы подземных лент (ug-*): по одному эффекту на сцену.

  python bench/m05-probes/gen_underground.py <папка>       # пишет <папка>/ug-*.scene.json
  fuzz_capture --scenes <папка> --out <папка> --batch 1    # снять эталоны (см. bench/README.md)

Сцены описаны «на восток» в целых тайлах (центр сущности — тайл + 0.5), ряд y = 0; для других
направлений вся сцена поворачивается. У подземной ленты поле "type": "input" / "output",
direction — куда едут предметы.
"""
import json
import os
import sys

DIRS = ["north", "east", "south", "west"]
TIER = {
    "y": ("transport-belt", "underground-belt"),
    "f": ("fast-transport-belt", "fast-underground-belt"),
    "e": ("express-transport-belt", "express-underground-belt"),
    "t": ("turbo-transport-belt", "turbo-underground-belt"),
}


def belt(x, y=0, d=1, tier="y"):
    return {"name": TIER[tier][0], "t": (x, y), "d": d}


def ug(kind, x, y=0, d=1, tier="y"):
    return {"name": TIER[tier][1], "t": (x, y), "d": d, "type": kind}


def feed(x, y, side, item=None, n=20, ins="inserter"):
    """Манипулятор и сундук со стороны `side` (0 север .. 3 запад) от тайла ленты (x, y).
    С `item` — источник (сундук -> лента), без — сток (лента -> сундук)."""
    dx, dy = [(0, -1), (1, 0), (0, 1), (-1, 0)][side]
    chest = {"name": "wooden-chest", "t": (x + 2 * dx, y + 2 * dy)}
    if item:
        chest["items"] = {item: n}
    # direction манипулятора — сторона, откуда он берёт
    return [{"name": ins, "t": (x + dx, y + dy), "d": side if item else (side + 2) % 4}, chest]


def src2(x, n=20, ins="inserter"):
    """Два источника на тайл (x, 0): железо с севера, медь с юга — предметы на обеих полосах."""
    return feed(x, 0, 0, "iron-plate", n, ins) + feed(x, 0, 2, "copper-plate", n, ins)


def scene(name, ticks, ents, rot=0):
    out = []
    for e in ents:
        x, y = e["t"]
        for _ in range(rot):  # поворот на 90° по часовой: (x, y) -> (-y, x)
            x, y = -y, x
        o = {"name": e["name"], "position": [x + 0.5, y + 0.5]}
        if "d" in e:
            o["direction"] = DIRS[(e["d"] + rot) % 4]
        for k in ("type", "items", "pickup", "drop"):  # векторы не поворачиваются: только при rot = 0
            if k in e:
                o[k] = e[k]
        out.append(o)
    return {"name": name, "ticks": ticks, "entities": out}


def pair(dist, tier="y", lead=2, tail=2, belts=None):
    """Ленты, вход в x = 0, выход в x = dist, ленты; источники на первой ленте."""
    b = belts or tier
    ents = [belt(x, tier=b) for x in range(-lead, 0)]
    ents += [ug("input", 0, tier=tier), ug("output", dist, tier=tier)]
    ents += [belt(dist + 1 + k, tier=b) for k in range(tail)]
    return ents


def probes():
    p = []
    # пара и её длина; предел дальности из прототипа (5 / 7 / 11), за пределом пары нет
    p.append(scene("ug-d1", 400, pair(1) + src2(-2)))
    p.append(scene("ug-d3", 400, pair(3) + src2(-2)))
    p.append(scene("ug-d5", 500, pair(5) + src2(-2)))
    p.append(scene("ug-d6-nopair", 400, pair(6) + src2(-2)))
    p.append(scene("ug-fast-d7", 400, pair(7, "f") + src2(-2)))
    p.append(scene("ug-turbo-d11", 400, pair(11, "t") + src2(-2)))
    # направления
    p.append(scene("ug-dir-s", 400, pair(3) + src2(-2), rot=1))
    p.append(scene("ug-dir-n", 400, pair(3) + src2(-2), rot=3))
    # очередь и подпор через подземку: тупик за выходом; то же со стоком в конце
    q = pair(2, lead=1, tail=1)
    # (489 тиков: очередь встаёт на тике 469, дальше состояние не меняется)
    p.append(scene("ug-queue", 489, q + src2(-1, 100, "fast-inserter")))
    p.append(scene("ug-restart", 800, q + src2(-1, 100, "fast-inserter") + feed(3, 0, 2)))
    # боковая загрузка во вход и в выход, слева (с севера) и справа (с юга)
    for name, x, side in (("ug-side-in-l", 0, 0), ("ug-side-in-r", 0, 2), ("ug-side-out-l", 3, 0), ("ug-side-out-r", 3, 2)):
        dy = -1 if side == 0 else 1
        feeder = [belt(x, 2 * dy, (side + 2) % 4), belt(x, dy, (side + 2) % 4)]
        srcs = feed(x, 2 * dy, 3, "iron-plate") + feed(x, 2 * dy, 1, "copper-plate")
        p.append(scene(name, 500, pair(3, lead=0) + feeder + srcs))
    # манипулятор берёт со входа; кладёт на вход (сбоку и сзади) и на выход
    p.append(scene("ug-take-in", 500, pair(3) + src2(-2) + feed(0, 0, 2)))
    p.append(scene("ug-drop-in", 400, pair(3, lead=0) + src2(0)))
    p.append(scene("ug-drop-in-back", 400, pair(3, lead=0) + feed(0, 0, 3, "iron-plate")))
    p.append(scene("ug-drop-out", 400, pair(3, lead=0) + src2(3)))
    # подземка без пары: вход — тупик, выход — половина тайла перед лентой
    p.append(scene("ug-lone-in", 500, [belt(-2), belt(-1), ug("input", 0)] + src2(-2)))
    p.append(scene("ug-lone-out", 400, [ug("output", 0), belt(1), belt(2)] + src2(0)))
    # смешение ярусов: быстрая подземка между обычными лентами; две пары подряд
    p.append(scene("ug-mix-tier", 500, pair(4, "f", belts="y") + src2(-2)))
    p.append(scene("ug-chain2", 500, pair(2, tail=0) + [ug("input", 3), ug("output", 5), belt(6)] + src2(-2)))
    # поиск пары: чужой ярус в промежутке; вход того же яруса в промежутке
    nest = [ug("input", 1, tier="f"), ug("output", 3, tier="f")]
    p.append(scene("ug-nest", 500, pair(4) + nest + src2(-2)))
    p.append(scene("ug-block-same", 500, pair(4) + [ug("input", 2)] + src2(-2)))
    # выход впадает сбоку в ленту поперёк
    cross = [belt(4, y, 2) for y in (-1, 0, 1, 2)]
    p.append(scene("ug-out-side", 500, pair(3, tail=0) + cross + src2(-2)))
    return p + probes_x()


def probes_x():
    """Вторая серия (ug-x-*): случаи, которые первая серия и случайные сцены не покрывают прицельно."""
    p = []
    fast = "fast-inserter"
    # поворот, выходящий прямо во вход; то же с очередью (выход — тупик) и стоком на выходе
    turn = [belt(-1, -2, 2), belt(-1, -1, 2), belt(-1, 0, 1)]
    tsrc = feed(-1, -2, 3, "iron-plate") + feed(-1, -2, 1, "copper-plate")
    p.append(scene("ug-x-turn-into-in", 500, turn + pair(2, lead=0) + tsrc))
    tsrc = feed(-1, -2, 3, "iron-plate", 100, fast) + feed(-1, -2, 1, "copper-plate", 100, fast)
    p.append(scene("ug-x-turn-into-in-q", 800, turn + pair(2, lead=0, tail=0) + tsrc + feed(2, 0, 2)))
    # выход одной пары бьёт в бок входа / выхода другой пары (идущей на юг)
    a = pair(2, tail=0)
    p.append(scene("ug-x-out-into-in", 700, a + [ug("input", 3, 0, 2), ug("output", 3, 2, 2), belt(3, 3, 2), belt(3, 4, 2)] + src2(-2, 40)))
    p.append(scene("ug-x-out-into-out", 700, a + [ug("input", 3, -2, 2), ug("output", 3, 0, 2), belt(3, 1, 2), belt(3, 2, 2)] + src2(-2, 40)))
    # два боковых источника во вход с двух сторон: свободно; с очередью и стоком; ещё и с подачей сзади
    north = [belt(0, -2, 2), belt(0, -1, 2)]
    south = [belt(0, 2, 0), belt(0, 1, 0)]

    def both(n, ins):
        return (feed(0, -2, 3, "iron-plate", n, ins) + feed(0, -2, 1, "copper-plate", n, ins)
                + feed(0, 2, 3, "iron-plate", n, ins) + feed(0, 2, 1, "copper-plate", n, ins))

    p.append(scene("ug-x-side-in-both", 500, pair(3, lead=0) + north + south + both(20, "inserter")))
    p.append(scene("ug-x-side-in-both-q", 800, pair(3, lead=0, tail=1) + north + south + both(100, fast) + feed(4, 0, 2)))
    p.append(scene("ug-x-side-in-both-back", 800, pair(3, lead=3, tail=1) + north + south + both(100, fast) + src2(-3, 100, fast) + feed(4, 0, 2)))
    # сильная отметка на выходе и на входе, очередь и перезапуски: сток в конце тупика
    p.append(scene("ug-x-side-out-q-sink", 800, pair(3, lead=0, tail=5) + [belt(3, -2, 2), belt(3, -1, 2)]
                   + feed(3, -2, 3, "iron-plate", 100, fast) + feed(3, -2, 1, "copper-plate", 100, fast) + src2(0, 100, fast) + feed(8, 0, 2)))
    p.append(scene("ug-x-side-in-q-sink", 800, pair(3, lead=0, tail=5) + north
                   + feed(0, -2, 3, "iron-plate", 100, fast) + feed(0, -2, 1, "copper-plate", 100, fast) + src2(3, 100, fast) + feed(8, 0, 2)))
    # кольцо через подземку: свободное, заполненное, с обратным порядком постройки
    ring = [belt(0, 0, 1), ug("input", 1), ug("output", 3), belt(4, 0, 2), belt(4, 1, 3), belt(3, 1, 3), belt(2, 1, 3), belt(1, 1, 3), belt(0, 1, 0)]
    rsrc = lambda n, ins="inserter": feed(2, 1, 0, "iron-plate", n, ins) + feed(2, 1, 2, "copper-plate", n, ins)
    p.append(scene("ug-x-ring", 600, ring + rsrc(12)))
    # (1065 тиков: кольцо забивается к тику 1045, дальше состояние не меняется)
    p.append(scene("ug-x-ring-full", 1065, ring + rsrc(100, fast)))
    p.append(scene("ug-x-ring-rev", 1065, ring[::-1] + rsrc(100, fast)))
    # сон манипуляторов на тайлах подземки: кладут на вход при очереди; стоки на входе и на выходе
    p.append(scene("ug-x-drop-in-q", 800, pair(2, lead=0, tail=1) + src2(0, 100, fast) + feed(3, 0, 2)))
    p.append(scene("ug-x-take-in-out", 700, pair(3) + feed(-2, 0, 0, "iron-plate", 30) + feed(0, 0, 2) + feed(3, 0, 0)))
    p.append(scene("ug-x-take-out-q", 800, pair(3, tail=0) + src2(-2, 60, fast) + feed(3, 0, 2) + feed(3, 0, 0)))
    # порядок полос при захвате наискосок (кастомные точки; нужен тестовый мод векторов)
    for name, ix, px, dy in (("in-s", 1, -1, 1), ("in-n", 1, -1, -1), ("out-s", 2, 1, 1), ("out-n", 2, 1, -1)):
        sink = [{"name": "inserter", "t": (ix, dy), "pickup": [px, -dy], "drop": [0, dy]}, {"name": "wooden-chest", "t": (ix, 2 * dy)}]
        p.append(scene("ug-x-lane-diag-" + name, 700, pair(3, tail=0) + src2(-2, 40) + sink))
    # слабая отметка на ленте прямо перед входом, очередь и перезапуски
    for d in (1, 2, 3):
        p.append(scene("ug-x-weak-before-d%d" % d, 800, pair(d, lead=1, tail=4) + src2(-1, 100, fast) + feed(d + 4, 0, 2)))
    return p + probes_y()


def probes_y():
    """Третья серия (ug-y-*): вход подземки не «питает» ленту перед собой; кольцо, в котором
    первой построена подземка."""
    p = []
    fast = "fast-inserter"
    # Лента на юг стоит перед входом (в промежутке пары). Вход смотрит в неё, но форму ей не
    # задаёт: без другого соседа она прямая (а не поворот)...
    down = [belt(1, 0, 2), belt(1, 1, 2), belt(1, 2, 2)]
    p.append(scene("ug-y-in-no-feed", 500, pair(2) + down + src2(-2) + feed(1, 0, 0, "iron-plate")))
    # ...а с лентой, впадающей с другого бока, — поворот (один питающий бок), а не прямая с
    # боковой загрузкой (два питающих бока).
    p.append(scene("ug-y-in-no-feed-side", 500, pair(3) + down + [belt(2, 0, 3)] + src2(-2) + feed(2, 0, 0, "iron-plate")))
    # то же, вход без пары
    p.append(scene("ug-y-in-no-feed-lone", 500, [belt(-2), belt(-1), ug("input", 0)] + down + src2(-2) + feed(1, 0, 0, "iron-plate")))
    # Кольцо через подземку, первой построена подземка: вход (затем остальное по ходу) и выход.
    ring = [belt(0, 0, 1), ug("input", 1), ug("output", 3), belt(4, 0, 2), belt(4, 1, 3), belt(3, 1, 3), belt(2, 1, 3), belt(1, 1, 3), belt(0, 1, 0)]
    rsrc = lambda n, ins="inserter": feed(2, 1, 0, "iron-plate", n, ins) + feed(2, 1, 2, "copper-plate", n, ins)
    for name, k in (("in", 1), ("out", 2)):
        order = ring[k:] + ring[:k]
        p.append(scene("ug-y-ring-%s-first" % name, 600, order + rsrc(12)))
        p.append(scene("ug-y-ring-%s-first-full" % name, 700, order + rsrc(100, fast)))
    return p


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    os.makedirs(sys.argv[1], exist_ok=True)
    for s in probes():
        with open(os.path.join(sys.argv[1], s["name"] + ".scene.json"), "w", encoding="utf-8", newline="\n") as f:
            f.write(json.dumps(s) + "\n")
    print(len(probes()), "scenes")


if __name__ == "__main__":
    main()
