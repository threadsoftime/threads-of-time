# SPDX-License-Identifier: GPL-2.0-or-later
"""dbc_compositor — shared library for composing ToT client DBC patches.

Generic, mod-agnostic building blocks:
  - dbc_io          : DBC binary read/write primitives
  - itemset_dbc     : ItemSet.dbc row encoder
  - spell_dbc       : Spell.dbc override encoder
  - bonus_map_parser: parser for bracket_set_bonus_map seed SQL
  - mpq_pack        : StormLib MPQ packer (ctypes)
  - manifest        : MANIFEST.toml loader + RangeRegistry (Task 2, check A)
  - merge           : one-producer-per-DBC-file merge (Task 4)
  - dbc_audit       : read record IDs from a WDBC blob (Task 7, check B / §10.3)
  - framexml        : priority-based FrameXML override resolver (Task 5)

Per-module composition lives in modules/<mod>/client/build_dbc.py recipes,
which import from this library.
"""
