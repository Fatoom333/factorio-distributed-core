"""Проверка гипотез «сегменты линии + список активных» на эталонах игры без модели манипуляторов.
Каждый тик: берём состояние игры t-1, двигаем ленты по гипотезе, сравниваем позиции
сопоставленных предметов с игрой t. Вставки/снятия манипуляторами берутся из игры (что появилось /
пропало) и будят сегмент после лент. Состояние гипотезы между тиками — только список активных.
python replay.py PART SLEEP dirs...   PART: none|tile|tail3|head3|tailN  SLEEP: r1|r2|r3"""
import sys, glob, os, json
from lag import load, lines_of, SPEED


def partition(n, part, o=0):
    """Номер сегмента для каждого тайла k (от головы). o — тайл первого источника от хвоста."""
    if part == "src3":
        # границы на o+3, o+6, ... от хвоста; f = n-1-k
        seg = []
        for k in range(n):
            f = n - 1 - k
            seg.append(0 if f < o else (f - o) // 3)
        c = max(seg)
        return [c - x for x in seg]
    if part == "none":
        return [0] * n
    if part == "tile":
        return list(range(n))
    if part.startswith("tail"):
        s = int(part[4:])
        c = (n - 1) // s
        return [c - (n - 1 - k) // s for k in range(n)]
    if part.startswith("head"):
        s = int(part[4:])
        return [k // s for k in range(n)]
    raise ValueError(part)


def match(prev, cur, v):
    m = {}
    j = 0
    for i, a in enumerate(prev):
        while j < len(cur) and cur[j] < a - v:
            j += 1
        if j < len(cur) and cur[j] <= a:
            m[i] = j
            j += 1
    return m


def run(sp, dp, part, sleep, verbose=False):
    sc, belts, dump = load(sp, dp)
    lines, pos = lines_of(sc, belts)
    bidx = {(b["pos"][0], b["pos"][1]): i for i, b in enumerate(dump[0]["belts"])}
    errs = 0
    first = None
    for line in lines:
        v = SPEED[pos[line[0]]["name"]]
        n = len(line)
        # первый источник от хвоста: манипулятор рядом с лентой, сундук с предметами
        xs_line = [p[0] for p in line]  # голова -> хвост
        srcx = [e["position"][0] for e in sc["entities"] if e["name"] == "wooden-chest" and e.get("items")]
        fs = [n - 1 - xs_line.index(x) for x in srcx if x in xs_line]
        o = min(fs) if fs else 0
        seg_of = partition(n, part, o)
        nseg = max(seg_of) + 1
        # границы сегмента s: [lo, hi) в позициях линии
        lo = [min(k for k in range(n) if seg_of[k] == s) * 256 for s in range(nseg)]
        hi = [(max(k for k in range(n) if seg_of[k] == s) + 1) * 256 for s in range(nseg)]
        idx = [bidx[p] for p in line]
        for lane in range(2):
            active = list(range(nseg)) if sleep.endswith("i") else []  # порядок обновления
            idle = {}
            def items(t):
                return sorted(k * 256 + it["p"] for k, bi in enumerate(idx) for it in dump[t]["belts"][bi]["lines"][lane])
            prev = items(0)
            for t in range(1, len(dump)):
                cur = items(t)
                # --- шаг лент по гипотезе от состояния prev
                st = list(prev)
                segs = [[i for i, x in enumerate(st) if lo[s] <= x < hi[s]] for s in range(nseg)]
                # предмет может перейти в другой сегмент в этом тике: храним списки индексов по месту
                owner = {}
                for s in range(nseg):
                    for i in segs[s]:
                        owner[i] = s
                moved_any = {}
                done = set()
                recv = set()
                q = 0
                while q < len(active):
                    s = active[q]
                    q += 1
                    mine = sorted((i for i in owner if owner[i] == s), key=lambda i: st[i])
                    # предел для первого: последний предмет сегмента впереди (s-1) или конец линии
                    ahead = sorted((i for i in owner if owner[i] < s), key=lambda i: st[i])
                    limit = st[ahead[-1]] + 64 if ahead else 0
                    mv = False
                    for i in mine:
                        if i in done:
                            limit = st[i] + 64
                            continue
                        done.add(i)
                        np = max(st[i] - v, limit)
                        if np < st[i]:
                            st[i] = np
                            mv = True
                        limit = st[i] + 64
                    for i in mine:
                        if st[i] < lo[s]:
                            owner[i] = seg_of[st[i] // 256]
                            recv.add(owner[i])
                            # переход в сегмент впереди будит его
                            if owner[i] not in active[q:] and owner[i] not in active:
                                active.append(owner[i])
                    if mv:
                        idle[s] = 0
                        up = s + 1
                        if up < nseg and up not in active:
                            active.append(up)
                    else:
                        idle[s] = idle.get(s, 0) + 1
                    moved_any[s] = mv
                # усыпление
                new_active = []
                for s in active:
                    if sleep.startswith("r4") and not any(owner[i] == s for i in owner):
                        new_active.append(s)
                        continue
                    if moved_any.get(s, True) or s in recv:
                        new_active.append(s)
                        continue
                    if sleep.startswith("r2"):
                        continue
                    if sleep.startswith("r3"):
                        if idle.get(s, 0) >= 2:
                            continue
                        new_active.append(s)
                        continue
                    # r1: спит, если впереди нет сегмента или он не активен
                    if s == 0 or (s - 1) not in active:
                        continue
                    new_active.append(s)
                active = new_active
                # --- сравнение с игрой
                m = match(prev, cur, v)
                bad = [(prev[i], st[i], cur[j]) for i, j in m.items() if st[i] != cur[j]]
                if bad:
                    errs += 1
                    if first is None:
                        first = (t, lane, bad[:3])
                # --- вставки/снятия игры будят сегменты
                got = set(m.values())
                for j, x in enumerate(cur):
                    if j not in got:
                        s = seg_of[min(x // 256, n - 1)]
                        if s not in active:
                            active.append(s)
                for i, x in enumerate(prev):
                    if i not in m:
                        s = seg_of[min(st[i] // 256, n - 1)]
                        for w in (s, s + 1):
                            if w < nseg and w not in active:
                                active.append(w)
                prev = cur
    return errs, first


def main():
    part, sleep = sys.argv[1], sys.argv[2]
    tot = 0
    for d in sys.argv[3:]:
        for sp in sorted(glob.glob(os.path.join(d, "*.scene.json"))):
            dp = sp.replace(".scene.json", ".dump.jsonl")
            e, f = run(sp, dp, part, sleep)
            tot += e > 0
            print(f"{os.path.basename(sp)[:-11]:28s} тиков с ошибкой {e:5d} первое {f}")
    print("сцен с ошибками:", tot)


if __name__ == "__main__":
    main()
