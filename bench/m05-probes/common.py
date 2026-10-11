"""Общие функции генераторов проб M0.5 (хвосты правил лент и манипуляторов).

Сцена — как в tests/fidelity/README.md. Лента идёт на восток по строке y = 0.5; тайл номер i
(от хвоста) стоит в x = X0 + i + 0.5. Хвост-источник: сундук + манипулятор с севера/юга от тайла.
"""
import json
import os

BELTS = {"y": "transport-belt", "r": "fast-transport-belt", "b": "express-transport-belt", "t": "turbo-transport-belt"}


class Scene:
    def __init__(self, name, ticks, x0=0, y0=0, btype="y"):
        self.name, self.ticks, self.x0, self.y0, self.bt = name, ticks, x0, y0, btype
        self.ents = []
        self.belt_ents = []

    def belts(self, n, btype=None):
        bn = BELTS[btype or self.bt]
        for i in range(n):
            self.belt_ents.append({"name": bn, "position": [self.x0 + i + 0.5, self.y0 + 0.5], "direction": "east"})
        return self

    def source(self, i, side="n", count=400, iname="inserter", drop=None, chest="steel-chest"):
        """Источник на тайл i: сундук -> манипулятор -> лента. side n/s; drop — [dx, dy] кастомный вектор."""
        s = -1 if side == "n" else 1
        item = "iron-plate" if side == "n" else "copper-plate"
        self.ents.append({"name": chest, "position": [self.x0 + i + 0.5, self.y0 + 0.5 + 2 * s], "items": {item: count}})
        e = {"name": iname, "position": [self.x0 + i + 0.5, self.y0 + 0.5 + s], "direction": "north" if side == "n" else "south"}
        if drop is not None:
            e["drop"] = drop
        self.ents.append(e)
        return self

    def sink(self, i, side="n", iname="inserter", chest="steel-chest", pickup=None):
        """Сток, берущий с тайла i сбоку (side n/s): манипулятор у ленты, сундук за ним."""
        s = -1 if side == "n" else 1
        e = {"name": iname, "position": [self.x0 + i + 0.5, self.y0 + 0.5 + s], "direction": "south" if side == "n" else "north"}
        self.ents.append(e)
        self.ents.append({"name": chest, "position": [self.x0 + i + 0.5, self.y0 + 0.5 + 2 * s]})
        return self

    def end_sink(self, n, iname="inserter", chest="steel-chest"):
        """Сток в голове ленты из n тайлов (за последним тайлом, берёт с запада)."""
        self.ents.append({"name": iname, "position": [self.x0 + n + 0.5, self.y0 + 0.5], "direction": "west"})
        self.ents.append({"name": chest, "position": [self.x0 + n + 1.5, self.y0 + 0.5]})
        return self

    def json(self):
        return {"name": self.name, "ticks": self.ticks, "entities": self.ents + self.belt_ents}

    def write(self, out):
        os.makedirs(out, exist_ok=True)
        with open(os.path.join(out, self.name + ".scene.json"), "w", encoding="utf-8") as f:
            json.dump(self.json(), f)
