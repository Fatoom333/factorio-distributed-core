import sys
from lag import analyze, load
sp, dp, K = sys.argv[1], sys.argv[2], int(sys.argv[3])
res, lines = analyze(sp, dp)
sc, belts, dump = load(sp, dp)
line = lines[0]
bidx = {(b["pos"][0], b["pos"][1]): i for i, b in enumerate(dump[0]["belts"])}
idx = [bidx[p] for p in line]
def items(t, lane):
    return sorted(k*256+it["p"] for k, bi in enumerate(idx) for it in dump[t]["belts"][bi]["lines"][lane])
out = []
for t, lane, a1, lag, n, k, p in res:
    if k != K or p: continue
    restart = (a1-64) in items(t-2, lane) and a1 in items(t-2, lane)
    out.append(f"{t}{'R' if restart else 'F'}{'L' if lag else ''}")
print(" ".join(out))
