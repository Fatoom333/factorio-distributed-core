"""Мини-библиотека для сцен с поворотами и боковой загрузкой (M0.5).

Координаты — целые тайлы (x, y), центр тайла (x + 0.5, y + 0.5); y растёт вниз (север — вверх).
Манипулятор, поставленный сбоку от ленты, смотрит (direction) в сторону сундука-источника
(то есть в сторону взятия), а сток — в сторону ленты.
"""
import json
import os

D = {"N": "north", "E": "east", "S": "south", "W": "west"}
V = {"north": (0, -1), "east": (1, 0), "south": (0, 1), "west": (-1, 0)}
OPP = {"north": "south", "south": "north", "east": "west", "west": "east"}
LEFT = {"north": "west", "west": "south", "south": "east", "east": "north"}  # поворот налево
RIGHT = {v: k for k, v in LEFT.items()}
BELTS = {"y": "transport-belt", "r": "fast-transport-belt", "b": "express-transport-belt", "t": "turbo-transport-belt"}
ITEM_N = "iron-plate"
ITEM_S = "copper-plate"


class Sc:
    def __init__(self, name, ticks=900, kind="y"):
        self.name, self.ticks, self.kind = name, ticks, kind
        self.ents, self.occ = [], {}

    def put(self, name, x, y, direction=None, **kw):
        if (x, y) in self.occ:
            raise ValueError(f"{self.name}: тайл {x},{y} занят ({self.occ[(x, y)]})")
        self.occ[(x, y)] = name
        e = {"name": name, "position": [x + 0.5, y + 0.5]}
        if direction:
            e["direction"] = direction
        e.update(kw)
        self.ents.append(e)

    def belt(self, x, y, d, kind=None):
        self.put(BELTS[kind or self.kind], x, y, d)

    def path(self, x, y, spec):
        """spec: 'E4 N3:r W2' — отрезки (направление, число тайлов[, :тип ленты]); первый тайл
        нового отрезка, отличающийся направлением от прошлого, — угол (поворот)."""
        tiles = []
        for tok in spec.split():
            kind = None
            if ":" in tok:
                tok, kind = tok.split(":")
            d, n = D[tok[0]], int(tok[1:])
            for _ in range(n):
                self.belt(x, y, d, kind)
                tiles.append((x, y, d))
                dx, dy = V[d]
                x, y = x + dx, y + dy
        return tiles

    def feed(self, x, y, side, item=None, count=400, ins="inserter", chest="steel-chest"):
        """Источник у тайла (x, y) со стороны side: сундук -> манипулятор -> лента."""
        side = D.get(side, side)
        item = item or (ITEM_N if side in ("north", "west") else ITEM_S)
        dx, dy = V[side]
        self.put(chest, x + 2 * dx, y + 2 * dy, items={item: count})
        self.put(ins, x + dx, y + dy, side)

    def sink(self, x, y, side, ins="inserter", chest="steel-chest"):
        """Сток: манипулятор у тайла (x, y) со стороны side берёт с ленты, сундук за ним."""
        side = D.get(side, side)
        dx, dy = V[side]
        self.put(ins, x + dx, y + dy, OPP[side])
        self.put(chest, x + 2 * dx, y + 2 * dy)

    def end_sink(self, x, y, d, ins="inserter", chest="steel-chest"):
        """Сток за последним тайлом (x, y) ленты, идущей в сторону d."""
        d = D.get(d, d)
        dx, dy = V[d]
        self.put(ins, x + dx, y + dy, OPP[d])
        self.put(chest, x + 2 * dx, y + 2 * dy)

    def json(self):
        # как в прошлых пробах: сундуки и манипуляторы раньше лент
        belts = [e for e in self.ents if e["name"].endswith("transport-belt")]
        rest = [e for e in self.ents if not e["name"].endswith("transport-belt")]
        return {"name": self.name, "ticks": self.ticks, "entities": rest + belts}

    def write(self, out):
        os.makedirs(out, exist_ok=True)
        with open(os.path.join(out, self.name + ".scene.json"), "w", encoding="utf-8") as f:
            json.dump(self.json(), f)
            f.write("\n")
