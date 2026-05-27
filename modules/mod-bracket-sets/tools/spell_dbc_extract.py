#!/usr/bin/env python3
"""Broaden the tier-bonus regex: spec name is optional (Hunter/Mage/Rogue/Warlock
tier sets are often spec-unified)."""
import struct, sys, re
path = sys.argv[1]
with open(path, "rb") as f: blob = f.read()
rc, fc, rs, sb_size = struct.unpack("<IIII", blob[4:20])
records_start = 20
sb_start = 20 + rc * rs
sb = blob[sb_start : sb_start + sb_size]
def read_str(off):
    if off == 0 or off >= sb_size: return None
    e = sb.find(b"\x00", off)
    if e == -1: return None
    try: return sb[off:e].decode("utf-8")
    except UnicodeDecodeError: return None

F_ID, F_EFFECT1, F_AURA1, F_NAME0 = 0, 71, 95, 136

# Loosened pattern: spec is optional. Examples:
#   Item - Hunter T9 2P Bonus            (no spec)
#   Item - Mage T8 4P Bonus              (no spec)
#   Item - Rogue T10 2P Bonus            (no spec)
#   Item - Druid T10 Balance 4P Bonus    (with spec)
tier_re = re.compile(
    r"\bItem\s*-\s*(.+?)\s+T(\d+)\s+(?:(.+?)\s+)?([24])P\s+Bonus\b"
)

candidates = []
for i in range(rc):
    off = records_start + i * rs
    fields = struct.unpack_from(f"<{fc}I", blob, off)
    spell_id = fields[F_ID]
    name = read_str(fields[F_NAME0])
    if not name: continue
    m = tier_re.search(name)
    if not m: continue
    klass, tier_n, spec, piece = m.groups()
    effect1 = fields[F_EFFECT1]
    if effect1 != 6: continue   # APPLY_AURA only
    candidates.append({
        "id": spell_id, "name": name,
        "class": klass.strip(), "tier": int(tier_n),
        "spec": (spec or "").strip(), "piece": int(piece),
        "aura": fields[F_AURA1],
    })

candidates.sort(key=lambda c: (c["class"], c["tier"], c["spec"], c["piece"]))

print(f"# total_candidates={len(candidates)}", file=sys.stderr)
print("spell_id\tclass\ttier\tspec\tpiece\taura_type\tname")
for c in candidates:
    print(f"{c['id']}\t{c['class']}\t{c['tier']}\t{c['spec']}\t{c['piece']}\t{c['aura']}\t{c['name']}")
