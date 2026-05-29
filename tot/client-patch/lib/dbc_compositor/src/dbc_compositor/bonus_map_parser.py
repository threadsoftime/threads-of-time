# SPDX-License-Identifier: GPL-2.0-or-later
"""Parse the bracket_set_bonus_map seed SQL into structured rows."""
from __future__ import annotations
import re
from dataclasses import dataclass
from pathlib import Path
from typing import List


@dataclass(frozen=True)
class BonusRow:
    itemset_id: int
    threshold: int
    class_id: int
    spec_id: int
    spell_id: int
    bracket_min: int
    bracket_max: int
    display_name: str


# Matches: (90101, 2, 1, 1, 64938, 25, 34, 'Sundered Resolve'),
# Single OR double quotes around display_name.
_ROW_RE = re.compile(
    r"\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*"
    r"(?P<q>['\"])(?P<name>.+?)(?P=q)\s*\)"
)


def parse_bonus_map_seed(sql_path: Path) -> List[BonusRow]:
    text = Path(sql_path).read_text(encoding="utf-8")
    rows: List[BonusRow] = []
    for m in _ROW_RE.finditer(text):
        rows.append(BonusRow(
            itemset_id  = int(m.group(1)),
            threshold   = int(m.group(2)),
            class_id    = int(m.group(3)),
            spec_id     = int(m.group(4)),
            spell_id    = int(m.group(5)),
            bracket_min = int(m.group(6)),
            bracket_max = int(m.group(7)),
            display_name= m.group("name"),
        ))
    return rows
