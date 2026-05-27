/*
 * Heimdal BFD-as-raid: Aku'mai (entry 4829, final boss).
 *
 * V1 (Heimdal-original): Patchwerk-flavored DPS race.
 *   - Hateful Strike (Mortal Strike R3 = 12294) on the highest-HP
 *     non-tank in melee, every ~5-7s. Drives healing focus + raid
 *     stack-and-pop play.
 *   - Tidal Surge (Whirlwind 1680) AoE burst at 75% / 50% / 25% HP --
 *     one-time per threshold. Forces a moment of raid awareness +
 *     positional reset.
 *   - Soft enrage at 25% HP: Bloodlust (2825) self-cast for 40s of
 *     haste. The fight becomes a true DPS race past this point.
 *
 * Phase 1.1 Step 7 (V1). Replaces the V0 SoD Dark Protection design.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

namespace
{
    enum Spells
    {
        SPELL_HATEFUL_STRIKE = 12294,   // Mortal Strike R3
        SPELL_TIDAL_SURGE    = 1680,    // Whirlwind R1 -- AoE burst
        SPELL_ENRAGE         = 2825,    // Bloodlust -- haste self-buff
    };

    enum EventIds
    {
        EVENT_HATEFUL_STRIKE = 1,
    };

    constexpr float TIDAL_75_PCT = 0.75f;
    constexpr float TIDAL_50_PCT = 0.50f;
    constexpr float TIDAL_25_PCT = 0.25f;
}

class boss_akumai_raid : public CreatureScript
{
public:
    boss_akumai_raid() : CreatureScript("boss_akumai_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_akumai_raidAI(creature);
    }

    struct boss_akumai_raidAI : public ScriptedAI
    {
        boss_akumai_raidAI(Creature* creature)
            : ScriptedAI(creature), _tidal75(false), _tidal50(false),
              _tidal25(false), _enraged(false) { }

        void Reset() override
        {
            _events.Reset();
            _tidal75 = _tidal50 = _tidal25 = _enraged = false;
        }

        void JustEngagedWith(Unit* /*who*/) override
        {
            _events.ScheduleEvent(EVENT_HATEFUL_STRIKE, 5s);
        }

        void DamageTaken(Unit* /*attacker*/, uint32& /*damage*/, DamageEffectType, SpellSchoolMask) override
        {
            float hpPct = float(me->GetHealth()) / float(me->GetMaxHealth());
            if (!_tidal75 && hpPct <= TIDAL_75_PCT)
            {
                _tidal75 = true;
                DoCastSelf(SPELL_TIDAL_SURGE);
                me->Yell("The deep churns!", LANG_UNIVERSAL);
            }
            if (!_tidal50 && hpPct <= TIDAL_50_PCT)
            {
                _tidal50 = true;
                DoCastSelf(SPELL_TIDAL_SURGE);
                me->Yell("Rise, currents of the abyss!", LANG_UNIVERSAL);
            }
            if (!_tidal25 && hpPct <= TIDAL_25_PCT)
            {
                _tidal25 = true;
                DoCastSelf(SPELL_TIDAL_SURGE);
                me->Yell("You will drown in shadow!", LANG_UNIVERSAL);
            }
            if (!_enraged && hpPct <= TIDAL_25_PCT)
            {
                _enraged = true;
                DoCastSelf(SPELL_ENRAGE);
            }
        }

        void UpdateAI(uint32 diff) override
        {
            if (!UpdateVictim())
                return;

            _events.Update(diff);
            if (me->HasUnitState(UNIT_STATE_CASTING))
                return;

            while (uint32 eventId = _events.ExecuteEvent())
            {
                switch (eventId)
                {
                    case EVENT_HATEFUL_STRIKE:
                    {
                        // Hateful Strike: random non-tank in melee range.
                        // Our AC fork's SelectTargetMethod doesn't have a
                        // MaxHealth option, so we approximate with random
                        // non-top-aggro within melee (10y). Functionally
                        // the same outcome -- non-tank takes the hit, so
                        // raid healers must spread their attention.
                        Unit* target = SelectTarget(SelectTargetMethod::Random, 1, 10.0f, true);
                        if (!target)
                            target = me->GetVictim();
                        if (target)
                            DoCast(target, SPELL_HATEFUL_STRIKE);
                        _events.ScheduleEvent(EVENT_HATEFUL_STRIKE,
                            _enraged ? Milliseconds(3000) : Milliseconds(5500));
                        break;
                    }
                    default:
                        break;
                }
            }

            DoMeleeAttackIfReady();
        }

    private:
        EventMap _events;
        bool     _tidal75;
        bool     _tidal50;
        bool     _tidal25;
        bool     _enraged;
    };
};

void AddSC_boss_akumai_raid()
{
    new boss_akumai_raid();
}
