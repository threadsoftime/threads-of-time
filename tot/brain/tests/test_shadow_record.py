import json
from brain_sidecar.decide import _shadow_record
from brain_sidecar.models import Decision, DecisionKind, PersonalityCard


def _card():
    return PersonalityCard(
        name="Kael", race="Blood Elf", class_="Paladin",
        backstory="x", talkativeness=0.7, courage=0.8, greed=0.3,
        attitude_to_master=0.0, party_invite_policy="accept_from_known",
    )


def test_shadow_record_writes_one_jsonl_line(tmp_path, monkeypatch):
    out = tmp_path / "shadow.jsonl"
    monkeypatch.setenv("BRAIN_SHADOW_RECORD_PATH", str(out))
    dec = Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                   confidence=0.0, reasoning="r")
    _shadow_record(
        bot_guid=1001, card=_card(), state={"self": {"level": 25}},
        triage_reason="organic_wakeup", max_level=25,
        raw='{"kind":"no_op","tool":null,"args":null,"confidence":0.0,"reasoning":"r"}',
        system_prompt="SYS", decision=dec,
    )
    line = out.read_text().strip()
    rec = json.loads(line)
    assert rec["bot_guid"] == 1001
    assert rec["expected_system_prompt"] == "SYS"
    assert rec["max_player_level"] == 25
    assert rec["card"]["class"] == "Paladin"          # by_alias serialization
    assert rec["expected_decision"]["kind"] == "no_op"


def test_shadow_record_noop_when_env_unset(tmp_path, monkeypatch):
    monkeypatch.delenv("BRAIN_SHADOW_RECORD_PATH", raising=False)
    # Must not raise and must not create any file.
    _shadow_record(bot_guid=1, card=_card(), state={}, triage_reason=None,
                   max_level=25, raw="{}", system_prompt="", decision=Decision(
                       kind=DecisionKind.NO_OP, tool=None, args=None,
                       confidence=0.0, reasoning=""))
