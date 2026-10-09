"""Пробы «кладка за хвост линии» (docs/mechanics/belts.md, правило 5 кладки).

Лента в 1 тайл на восток; источник сбоку кладёт в середину (очередь [0, 64, 128]), второй
источник с кастомным вектором (нужен тестовый мод fd-test-vectors) кладёт на p = 200/230/250 —
сначала на 192, потом точка вставки уходит за хвост. Вариант -two: сзади обычная лента (в линию
впадает другая линия).

    python bench/tail-probes/gen.py <папка>
    cargo run --release --bin fuzz_capture -- --scenes <папка> --out <папка-эталонов>
    cargo run --release --bin fuzz_report -- --run <папка-эталонов>
"""
import json
import os
import sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
belts = {"b8": "transport-belt", "b16": "fast-transport-belt", "b24": "express-transport-belt", "b32": "turbo-transport-belt"}
for bk, bn in belts.items():
    for p in (200, 230, 250):
        for two in ([False, True] if bk == "b16" else [False]):
            x = 0.5 + (128 - p) / 256  # точка выгрузки: p = 128 - (x - центр)·256
            ents = [{"name": bn, "position": [0.5, 0.5], "direction": "east"}]
            if two:
                ents.append({"name": "transport-belt", "position": [-0.5, 0.5], "direction": "east"})
            ents += [
                {"name": "inserter", "position": [0.5, -0.5], "direction": "north"},
                {"name": "wooden-chest", "position": [0.5, -1.5], "items": {"iron-plate": 10}},
                {"name": "inserter", "position": [0.5, 1.5], "direction": "south",
                 "pickup": [0, 1], "drop": [x - 0.5, 0.703125 - 1.5]},
                {"name": "wooden-chest", "position": [0.5, 2.5], "items": {"copper-plate": 10}},
            ]
            name = f"tail-{bk}-p{p}" + ("-two" if two else "")
            with open(os.path.join(out, name + ".scene.json"), "w") as f:
                json.dump({"name": name, "ticks": 600, "entities": ents}, f)
