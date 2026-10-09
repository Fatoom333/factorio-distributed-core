"""Поиск «запоздалых перезапусков» в эталоне игры (без модели).
Линия = подряд стоящие ленты на восток (y = 0.5), голова — максимальный x.
Событие на паре (передний, задний) при переходе t-1 -> t: на t-1 зазор ровно 64, передний сдвинулся,
задний — нет, хотя место было (лаг), или задний сдвинулся (норма).
python lag.py <dump.jsonl> <scene.json> [--verbose]"""
import json, sys
from collections import Counter, defaultdict

SPEED = {"transport-belt": 8, "fast-transport-belt": 16, "express-transport-belt": 24, "turbo-transport-belt": 32}


def load(scene_path, dump_path):
    sc = json.load(open(scene_path, encoding="utf-8"))
    belts = [e for e in sc["entities"] if e["name"] in SPEED]
    return sc, belts, [json.loads(l) for l in open(dump_path, encoding="utf-8")]


def lines_of(sc, belts):
    """Линии: группы подряд стоящих лент одного типа и направления (только east/west/north/south по прямой)."""
    pos = {(b["position"][0], b["position"][1]): b for b in belts}
    U = {"east": (1, 0), "west": (-1, 0), "north": (0, -1), "south": (0, 1)}
    nxt = {}
    for p, b in pos.items():
        u = U[b.get("direction", "north")]
        q = (p[0] + u[0], p[1] + u[1])
        if q in pos and pos[q]["name"] == b["name"] and pos[q].get("direction") == b.get("direction"):
            nxt[p] = q
    prev = {q: p for p, q in nxt.items()}
    out = []
    for p in pos:
        if p in nxt:
            continue
        line = [p]
        while line[-1] in prev:
            line.append(prev[line[-1]])
        out.append(line)  # голова -> хвост
    return out, pos


def analyze(scene_path, dump_path, verbose=False):
    sc, belts, dump = load(scene_path, dump_path)
    lines, pos = lines_of(sc, belts)
    bidx = {}
    for b in dump[0]["belts"]:
        bidx[(b["pos"][0], b["pos"][1])] = len(bidx)
    res = []
    for line in lines:
        v = SPEED[pos[line[0]]["name"]]
        n = len(line)
        idx = [bidx[(p[0], p[1])] for p in line]
        for lane in range(2):
            prev = None
            for d in dump:
                cur = []
                for k, bi in enumerate(idx):
                    for it in d["belts"][bi]["lines"][lane]:
                        cur.append(k * 256 + it["p"])
                cur.sort()
                if prev is not None:
                    # сопоставление: каждому a из prev — b из cur с a - v <= b <= a
                    m = {}
                    j = 0
                    for i, a in enumerate(prev):
                        while j < len(cur) and cur[j] < a - v:
                            j += 1
                        if j < len(cur) and cur[j] <= a:
                            m[i] = cur[j]
                            j += 1
                    surv = sorted(m)
                    for x, y in zip(surv, surv[1:]):
                        a0, a1 = prev[x], prev[y]
                        if y != x + 1:
                            continue
                        b0, b1 = m[x], m[y]
                        could = min(v, a1 - (b0 + 64))
                        if b0 >= a0 or could <= 0:
                            continue  # передний стоял или задний и так упирался
                        if a1 - a0 != 64 and a1 - b1 == could:
                            continue  # обычное свободное движение
                        lag = (a1 - b1) < could
                        res.append((d["t"], lane, a1, lag, n, a1 // 256, a1 % 256))
                        if verbose and lag:
                            print(f"t={d['t']} lane={lane} back={a1} (k={a1//256} p={a1%256}, от хвоста {n-1-a1//256}) front {a0}->{b0} back {a1}->{b1}")
                prev = cur
    return res, lines


def main():
    dump, scene = sys.argv[1], sys.argv[2]
    res, lines = analyze(scene, dump, "--verbose" in sys.argv)
    lag = Counter()
    ok = Counter()
    for t, lane, a1, l, n, k, p in res:
        (lag if l else ok)[(k, p)] += 1
    print("линия тайлов:", [len(x) for x in lines])
    print("лаг (k от головы, p):", sorted(lag.items()))
    print("норма на границах тайлов p=0:", sorted((k, c) for (k, p), c in ok.items() if p == 0))


if __name__ == "__main__":
    main()
