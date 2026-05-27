#!/usr/bin/env bash
# V3.6 spec §10.2 — round-trip a 9-field PersonalityCard against live
# memory-sidecar at port 8090. Sentinel bot_id = 999999; cleaned up
# at end.
#
# Usage:
#   MEM_BEARER=<bearer> ./probe_v36_personality_roundtrip.sh
#
# Run from anywhere with access to 192.168.1.3:8090.
set -euo pipefail

: "${MEM_BEARER:?MEM_BEARER env var required}"
MEM_URL="${MEM_URL:-http://192.168.1.3:8090}"
BOT_ID="999999"

PERSONA_JSON='{"name":"V36ProbeBot","race":"Human","class":"Mage","backstory":"Probe bot for V3.6 round-trip","talkativeness":0.5,"courage":0.5,"greed":0.5,"attitude_to_master":0.0,"party_invite_policy":"accept_from_known","pvp_appetite":0.4,"raid_appetite":0.7,"completionist_streak":0.6,"gold_motivation":0.5,"profession_appetite":0.3}'

echo "=== persona JSON length: ${#PERSONA_JSON} chars (must be < 4000) ==="
[[ ${#PERSONA_JSON} -lt 4000 ]] || { echo "PERSONA TOO LARGE"; exit 1; }

echo ""
echo "=== POST memory.personality_set ==="
SET_BODY=$(jq -n --arg id "$BOT_ID" --arg p "$PERSONA_JSON" '{bot_id:$id,persona:$p}')
curl -sS -X POST "$MEM_URL/memory/personality/set" \
  -H "Authorization: Bearer $MEM_BEARER" \
  -H "Content-Type: application/json" \
  -d "$SET_BODY" | tee /tmp/v36_probe_set.json
echo ""

echo ""
echo "=== POST memory.personality_get ==="
GET_BODY=$(jq -n --arg id "$BOT_ID" '{bot_id:$id}')
curl -sS -X POST "$MEM_URL/memory/personality/get" \
  -H "Authorization: Bearer $MEM_BEARER" \
  -H "Content-Type: application/json" \
  -d "$GET_BODY" | tee /tmp/v36_probe_get.json
echo ""

echo ""
echo "=== Verify round-trip ==="
ECHOED=$(jq -r '.result.persona // .persona' /tmp/v36_probe_get.json)
python3 -c "
import json, sys
got = json.loads('''$ECHOED''')
want = json.loads('''$PERSONA_JSON''')
all_match = True
for k in want:
    g, w = got.get(k), want[k]
    if g != w:
        print(f'  mismatch: {k}={g} (got) vs {w} (want)')
        all_match = False
if all_match:
    print(f'OK all {len(want)} fields preserved')
else:
    sys.exit(1)
"

echo ""
echo "=== Cleanup: overwrite sentinel with empty persona ==="
curl -sS -X POST "$MEM_URL/memory/personality/set" \
  -H "Authorization: Bearer $MEM_BEARER" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg id "$BOT_ID" '{bot_id:$id,persona:"{}"}')" > /dev/null
echo "OK cleanup done"
