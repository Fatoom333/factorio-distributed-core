"""Набор j: «те же идеи, другие сцены» для правил линий лент — другие направления, стыки скоростей,
стоки посреди ленты, кладка у края тайла, две полосы с разной нарезкой.
python genj.py <out_dir>"""
import json, os, sys

BELTS = {"y": "transport-belt", "r": "fast-transport-belt", "b": "express-transport-belt", "t": "turbo-transport-belt"}
DIRS = ["north", "east", "south", "west"]


def rot_v(v, k):
    x, y = v
    for _ in range(k):
        x, y = -y, x
    return [x, y]


def rot_d(d, k):
    return DIRS[(DIRS.index(d) + k) % 4]


def scene(name, types, d="east", srcs=(), side_sinks=(), end_sink=True, origin=(0, 0), ticks=2000, order="th", sink_name="inserter"):
    """types: строка типов лент от хвоста к голове; srcs: (тайл от хвоста, сторона 'n'/'s', dx или None, имя, число)."""
    n = len(types)
    k = {"east": 0, "south": 1, "west": 2, "north": 3}[d]
    ents = []
    belts = [{"name": BELTS[t], "position": [i + 0.5, 0.5], "direction": "east"} for i, t in enumerate(types)]
    if order == "ht":
        belts = belts[::-1]
    for (i, side, dx, iname, count) in srcs:
        s = -1 if side == "n" else 1
        item = "iron-plate" if side == "n" else "copper-plate"
        ents.append({"name": "wooden-chest", "position": [i + 0.5, 0.5 + 2 * s], "items": {item: count}})
        e = {"name": iname, "position": [i + 0.5, 0.5 + s], "direction": "north" if side == "n" else "south"}
        if dx is not None:
            e["drop"] = [dx, -1.2 * s]
        ents.append(e)
    ents = ents + belts if order != "bf" else belts + ents
    if end_sink:
        ents.append({"name": sink_name, "position": [n + 0.5, 0.5], "direction": "west"})
        ents.append({"name": "steel-chest", "position": [n + 1.5, 0.5]})
    for (i, side) in side_sinks:  # тайл от хвоста; сток стоит со стороны side
        s = -1 if side == "n" else 1
        ents.append({"name": "inserter", "position": [i + 0.5, 0.5 + s], "direction": "south" if side == "n" else "north"})
        ents.append({"name": "steel-chest", "position": [i + 0.5, 0.5 + 2 * s]})
    cx = n // 2
    out = []
    for e in ents:
        x, y = e["position"]
        x, y = rot_v([x - cx - 0.5, y - 0.5], k)
        e = dict(e)
        e["position"] = [x + 0.5 + origin[0], y + 0.5 + origin[1]]
        if "direction" in e:
            e["direction"] = rot_d(e["direction"], k)
        if "drop" in e:
            e["drop"] = rot_v(e["drop"], k)
        out.append(e)
    return {"name": name, "ticks": ticks, "entities": out}


def all_src(n, side="n", dx=None, iname="inserter", count=400, tiles=None):
    return [(i, side, dx, iname, count) for i in (range(n) if tiles is None else tiles)]


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    sc = []
    o = [(0, 0), (3, -2), (-4, 1), (2, 5), (-1, -5), (5, 3), (-3, 4), (1, -3)]
    oi = 0

    def org():
        nonlocal oi
        oi += 1
        return o[oi % len(o)]

    # 1. другие направления: источники на каждом тайле, сток в конце
    for d in ("south", "north", "west"):
        for n in (10, 20):
            for bt in "yr":
                sc.append(scene(f"j-dir-{d[0]}-{bt}{n}", bt * n, d, all_src(n), origin=org(), ticks=1800 + 60 * n))
    # 2. стыки скоростей, очередь через стык
    for tag, types in (("yr", "yyyyyrrrrr"), ("ry", "rrrrryyyyy"), ("by", "bbbbyyyyyy"), ("yty", "yyyyttttyyyy"),
                       ("tb", "ttttttbbbbbb"), ("yb", "yyyyyybbbbbb"), ("ty", "ttttttyyyyyy")):
        sc.append(scene(f"j-mix-{tag}", types, "east", all_src(len(types)), origin=org(), ticks=2400))
    sc.append(scene("j-mix-yr-s", "yyyyyrrrrr", "south", all_src(10), origin=org(), ticks=2400))
    sc.append(scene("j-mix-by-w", "bbbbyyyyyy", "west", all_src(10), origin=org(), ticks=2400))
    sc.append(scene("j-mix-ry-tail", "rrrrryyyyy", "east", all_src(10, tiles=range(4)), origin=org(), ticks=2400))
    # 3. стоки посреди ленты и очередь
    n = 14
    sc.append(scene("j-mid-dead", "y" * n, "east", all_src(n, tiles=range(4)), side_sinks=[(10, "s"), (6, "s")], end_sink=False, origin=org(), ticks=3000))
    sc.append(scene("j-mid-end", "y" * n, "east", all_src(n, tiles=range(6)), side_sinks=[(9, "s")], origin=org(), ticks=3000))
    sc.append(scene("j-mid-fast", "r" * n, "east", all_src(n, tiles=range(8)), side_sinks=[(11, "s"), (5, "s")], end_sink=False, origin=org(), ticks=3000))
    sc.append(scene("j-mid-south", "y" * n, "south", all_src(n, tiles=range(5)), side_sinks=[(8, "s")], origin=org(), ticks=3000))
    sc.append(scene("j-mid-allsrc", "r" * 12, "east", all_src(12, tiles=[0, 1, 2, 3, 5, 6, 8, 9, 11]), side_sinks=[(4, "s"), (7, "s")], origin=org(), ticks=3000))
    # 4. кладка у края тайла в сжатой очереди: в_тайле = 128 − dx·256
    for tag, dx in (("f4", 124 / 256), ("f0", 127 / 256), ("f12", 116 / 256), ("b253", -125 / 256), ("b255", -127 / 256), ("b200", -72 / 256)):
        for bt in "yr":
            sc.append(scene(f"j-edge-{tag}-{bt}", bt * 10, "east", all_src(10, dx=dx), origin=org(), ticks=2000))
    sc.append(scene("j-edge-mixed", "y" * 12, "east",
                    [(i, "n", [124 / 256, -125 / 256, None, 127 / 256][i % 4], "inserter", 400) for i in range(12)], origin=org(), ticks=2400))
    sc.append(scene("j-edge-f4-s", "r" * 10, "south", all_src(10, dx=124 / 256), origin=org(), ticks=2000))
    sc.append(scene("j-edge-junction", "yyyyyrrrrr", "east",
                    [(i, "n", [124 / 256, -125 / 256][i % 2], "inserter", 400) for i in range(10)], origin=org(), ticks=2400))
    # 5. две полосы с разной нарезкой, быстрые и длиннорукие источники
    sc.append(scene("j-two-a", "y" * 12, "east", all_src(12, "n", tiles=range(4)) + all_src(12, "s", tiles=range(5, 9)), origin=org(), ticks=3000))
    sc.append(scene("j-two-b", "r" * 12, "east", all_src(12, "n", tiles=[0, 1, 2, 3, 4, 5]) + all_src(12, "s", tiles=[2, 7, 8]), origin=org(), ticks=3000))
    sc.append(scene("j-two-all", "r" * 10, "west", all_src(10, "n") + all_src(10, "s"), origin=org(), ticks=2500, sink_name="fast-inserter"))
    sc.append(scene("j-fastsrc", "b" * 12, "east", all_src(12, iname="fast-inserter"), origin=org(), ticks=2000, sink_name="fast-inserter"))
    sc.append(scene("j-ht-south", "y" * 16, "south", all_src(16), origin=org(), ticks=2600, order="ht"))
    for s in sc:
        json.dump(s, open(os.path.join(out, s["name"] + ".scene.json"), "w"))
    print(len(sc), "scenes")


main()
