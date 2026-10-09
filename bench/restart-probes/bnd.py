"""По каждой сцене и каждой границе тайла k: сколько перезапусков с лагом, сколько без лага при
движении в тайле k в тике t-1 (опровержение границы), сколько без движения.
python bnd.py W dirs..."""
import sys, glob, os
from collections import defaultdict
from lag import analyze, load
W = int(sys.argv[1])
for d in sys.argv[2:]:
    for sp in sorted(glob.glob(os.path.join(d, "*.scene.json"))):
        dp = sp.replace(".scene.json", ".dump.jsonl")
        res, lines = analyze(sp, dp)
        sc, belts, dump = load(sp, dp)
        line = lines[0]; n = len(line)
        bidx = {(b["pos"][0], b["pos"][1]): i for i, b in enumerate(dump[0]["belts"])}
        idx = [bidx[p] for p in line]
        cache = {}
        def items(t, lane):
            if (t, lane) not in cache:
                cache[(t, lane)] = sorted(k*256+it["p"] for k, bi in enumerate(idx) for it in dump[t]["belts"][bi]["lines"][lane])
            return cache[(t, lane)]
        st = defaultdict(lambda: [0, 0, 0])
        for t, lane, a1, lag, n_, k, p in res:
            if p != 0 or t < 2: continue
            # только настоящий перезапуск: передний стоял в тике t-1
            if (a1 - 64) not in items(t-2, lane) or a1 not in items(t-2, lane):
                continue  # передний или задний двигался в тике t-1 — не перезапуск
            lo, hi = 256*k, 256*(k+W)
            mv = [x for x in items(t-2, lane) if lo <= x < hi] != [x for x in items(t-1, lane) if lo <= x < hi]
            st[k][0 if lag else (1 if mv else 2)] += 1
        conf = [k for k in sorted(st) if st[k][0]]
        refu = [k for k in sorted(st) if not st[k][0] and st[k][1]]
        mixed = [(k, st[k][:2]) for k in sorted(st) if st[k][0] and st[k][1]]
        print(f"{os.path.basename(sp)[:-11]:14s} N={n:2d} граница(лаг): {[(k, n-k) for k in conf]}  нет(движение без лага): {[n-k for k in refu]}  смешано: {mixed}")
