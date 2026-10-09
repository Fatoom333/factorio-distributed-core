"""Пробы перезапуска сжатой очереди. Лента на восток, N тайлов, источники с севера у хвоста,
сток в конце линии. Варианты: тип ленты, порядок постройки, число/место источников, боковой сток.
python gen.py <out_dir> <set>"""
import json, os, sys

BELTS = {"y": "transport-belt", "r": "fast-transport-belt", "b": "express-transport-belt", "t": "turbo-transport-belt"}


def scene(name, n, bt="y", order="th", src_tiles=None, sink="end", side_sinks=(), ticks=None,
          src_side="n", x0=None, belts_first=False, extra=None):
    x0 = -(n // 2) if x0 is None else x0  # хвост
    ents_belts = []
    xs = [x0 + i + 0.5 for i in range(n)]  # хвост -> голова
    for x in xs:
        ents_belts.append({"name": BELTS[bt], "position": [x, 0.5], "direction": "east"})
    if order == "ht":
        ents_belts = ents_belts[::-1]
    if src_tiles is None:
        src_tiles = list(range(min(n, 4)))  # номера от хвоста
    src = []
    for i in src_tiles:
        x = xs[i]
        if src_side in ("n", "ns"):
            src.append({"name": "wooden-chest", "position": [x, -1.5], "items": {"iron-plate": 400}})
            src.append({"name": "inserter", "position": [x, -0.5], "direction": "north"})
        if src_side in ("s", "ns"):
            src.append({"name": "wooden-chest", "position": [x, 2.5], "items": {"copper-plate": 400}})
            src.append({"name": "inserter", "position": [x, 1.5], "direction": "south"})
    sinks = []
    if sink == "end":
        hx = xs[-1] + 1
        sinks.append({"name": "inserter", "position": [hx, 0.5], "direction": "west"})
        sinks.append({"name": "wooden-chest", "position": [hx + 1, 0.5]})
    for k in side_sinks:  # номер тайла от головы; сток с юга
        x = xs[-1 - k]
        sinks.append({"name": "inserter", "position": [x, 1.5], "direction": "north"})
        sinks.append({"name": "wooden-chest", "position": [x, 2.5]})
    if belts_first:
        ents = ents_belts + src + sinks
    else:
        ents = src + ents_belts + sinks
    if extra:
        ents += extra
    if ticks is None:
        rate = max(1, len(src) // 2 - 1) * 0.8
        ticks = int(4 * n / rate * 60) + 1500
    return {"name": name, "ticks": min(ticks, 9000), "entities": ents}


def main():
    out, which = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    sc = []
    if which == "a":
        for n in [2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16, 20, 24, 32, 40]:
            for bt in ["y", "r"]:
                for order in ["th", "ht"]:
                    sc.append(scene(f"a-{bt}{order}-{n:02d}", n, bt, order))
    if which == "b":
        for n in [6, 8, 10, 14, 20, 30, 40]:
            for bt in ["y", "r"]:
                for bf in [False, True]:
                    sc.append(scene(f"b-{bt}{'B' if bf else 'S'}-{n:02d}", n, bt, "th", src_tiles=list(range(n)), belts_first=bf, ticks=3000))
    if which == "c":
        n = 14
        specs = {
            "t4": list(range(4)), "t6": list(range(6)), "t8": list(range(8)), "t10": list(range(10)),
            "all": list(range(14)), "ev3": [0, 3, 6, 9, 12], "ev2": list(range(0, 14, 2)),
            "t3h": [0, 1, 2] + list(range(6, 14)), "t1": [0, 1], "mid": [0, 1, 2, 3, 7, 8],
        }
        for key, tiles in specs.items():
            for bt in ["y", "r"]:
                sc.append(scene(f"c-{bt}-{key}", n, bt, "th", src_tiles=tiles, ticks=4000))
        # тихие манипуляторы: источники у хвоста + манипуляторы с пустыми сундуками на остальных тайлах
        for bt in ["y", "r"]:
            s0 = scene(f"c-{bt}-idle", n, bt, "th", src_tiles=[0, 1, 2, 3], ticks=4000)
            xs = [-(n // 2) + i + 0.5 for i in range(n)]
            for i in range(4, n):
                s0["entities"].append({"name": "wooden-chest", "position": [xs[i], -1.5]})
                s0["entities"].append({"name": "inserter", "position": [xs[i], -0.5], "direction": "north"})
            sc.append(s0)
    if which == "d":
        n = 14
        for bt in ["y", "r"]:
            for j in range(4, 14):
                sc.append(scene(f"d-{bt}-x{j:02d}", n, bt, "th", src_tiles=[0, 1, 2, 3, j], ticks=4000))
                s0 = scene(f"d-{bt}-l{j:02d}", n, bt, "th", src_tiles=[0, 1, 2, 3, j], ticks=4000)
                # у дополнительного источника — 3 предмета
                ch = [e for e in s0["entities"] if e["name"] == "wooden-chest" and "items" in e][-1]
                ch["items"] = {"iron-plate": 3}
                sc.append(s0)
    if which == "e":
        n = 20
        for j in range(4, 20):
            sc.append(scene(f"e-y-b4x{j:02d}", n, "y", "th", src_tiles=[0, 1, 2, 3, j], ticks=5000))
        for j in range(1, 20):
            sc.append(scene(f"e-y-b1x{j:02d}", n, "y", "th", src_tiles=[0, j], ticks=7000))
    if which == "f":
        n = 20
        xs = [-(n // 2) + i + 0.5 for i in range(n)]
        def extra(name, j, side, count, kind="src"):
            s0 = scene(name, n, "y", "th", src_tiles=[0, 1, 2, 3], ticks=5000)
            x = xs[j]
            if side == "n":
                ch = {"name": "wooden-chest", "position": [x, -1.5]}
                ins = {"name": "inserter", "position": [x, -0.5], "direction": "north"}
            else:
                ch = {"name": "wooden-chest", "position": [x, 2.5]}
                ins = {"name": "inserter", "position": [x, 1.5], "direction": "south"}
            if kind == "src":
                ch["items"] = {"copper-plate": count}
            else:
                ins["direction"] = "south" if side == "n" else "north"
            s0["entities"] += [ch, ins]
            sc.append(s0)
        extra("f-n10-1", 10, "n", 1)
        extra("f-n10-2", 10, "n", 2)
        extra("f-n10-5", 10, "n", 5)
        extra("f-s10-1", 10, "s", 1)
        extra("f-s10-50", 10, "s", 50)
        extra("f-n10-sink", 10, "n", 0, "sink")
        extra("f-s10-sink", 10, "s", 0, "sink")
        for j in (8, 12, 14):
            extra(f"f-n{j}-1", j, "n", 1)
        # базовая группа 0..6 и 0..9 без доп. источника
        for m in (7, 10, 13):
            sc.append(scene(f"f-base{m}", n, "y", "th", src_tiles=list(range(m)), ticks=5000))
    if which == "g":
        n = 20
        for m in range(1, 8):
            for off in (0, 2):
                s0 = scene(f"g-m{m}-o{off}", n, "y", "th", src_tiles=[off + i for i in range(m)] + [14], ticks=7000)
                ch = [e for e in s0["entities"] if e["name"] == "wooden-chest" and "items" in e][-1]
                ch["items"] = {"copper-plate": 1}
                sc.append(s0)
    if which == "h":
        n = 16
        cases = {"h1": ([0, 3], 10), "h2": ([0, 3], None), "h3": ([3], 10), "h4": ([0, 3, 10], None),
                 "h6": ([2, 3], 10), "h7": ([1, 3], 10), "h8": ([0, 4], 10), "h9": ([0, 5], 10),
                 "h10": ([0, 6], 10), "h11": ([0, 2], 10), "h12": ([0, 1, 3], 10), "h13": ([0, 3, 6], 11),
                 "h14": ([0, 1, 2], 10), "h15": ([0, 2, 3], 10)}
        for key, (tiles, far) in cases.items():
            t2 = tiles + ([far] if far is not None else [])
            s0 = scene(f"h-{key}", n, "y", "th", src_tiles=t2, ticks=9000)
            if far is not None:
                ch = [e for e in s0["entities"] if e["name"] == "wooden-chest" and "items" in e][-1]
                ch["items"] = {"copper-plate": 1}
            sc.append(s0)
    if which == "i":
        for n, ns, nk in ((24, 10, 10), (30, 12, 12)):
            s0 = scene(f"i-flow{n}", n, "y", "th", src_tiles=list(range(ns)), side_sinks=list(range(nk)), ticks=4000)
            sc.append(s0)
            s1 = scene(f"i-flowr{n}", n, "r", "th", src_tiles=list(range(ns)), side_sinks=list(range(nk)), ticks=4000)
            sc.append(s1)
    for s in sc:
        with open(os.path.join(out, s["name"] + ".scene.json"), "w", encoding="utf-8") as f:
            json.dump(s, f)
    print(len(sc), "scenes")


if __name__ == "__main__":
    main()
