/*
 * Heimdal BFD-as-raid: Gelihast (entry 6243).
 *
 * V1 (Heimdal-original): dispels-matter fight.
 *   - Curse of Blackfathom (Curse of Agony R2 = 1014) on tank every
 *     ~18s. Decursable. Stacking shadow DoT that will overrun the
 *     tank if left up.
 *   - Fear (5782) on random raid member every ~25s. Dispellable
 *     (Tremor Totem, Berserker Rage, Fear Ward, mass dispel).
 *   - At 50% HP: spawn 3 Aku'mai Servant (4978) adds in a triangle
 *     around the boss. They despawn after 60s.
 *
 * Phase 1.1 Step 5 (V1). Replaces the V0 SoD intermission state
 * machine.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

#include <cmath>

namespace
{
    enum Spells
    {
        SPELL_CURSE_OF_BLACKFATHOM = 1014,   // Curse of Agony R2 (decursable)
        SPELL_FEAR                 = 5782,   // basic Fear R1 (dispellable)
    };

    enum Adds
    {
        NPC_AKU_MAI_SERVANT = 4978,
    };

    enum EventIds
    {
        EVENT_CURSE = 1,
        EVENT_FEAR,
    };

    constexpr float ADD_SPAWN_HP_PCT  = 0.50f;
    constexpr uint32 ADDS_PER_WAVE    = 3;
    constexpr uint32 ADD_DESPAWN_MS   = 60 * 1000;
}

class boss_gelihast_raid : public CreatureScript
{
public:
    boss_gelihast_raid() : CreatureScript("boss_gelihast_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_gelihast_raidAI(creature);
    }

    struct boss_gelihast_raidAI : public ScriptedAI
    {
        boss_gelihast_raidAI(Creature* creature)
            : ScriptedAI(creature), _addsSpawned(false) { }

        void Reset() override
        {
            _events.Reset();
            _addsSpawned = false;
        }

        void JustEngagedWith(Unit* /*who*/) override
        {
            _events.ScheduleEvent(EVENT_CURSE, 6s);
            _events.ScheduleEvent(EVENT_FEAR, 20s);
        }

        void DamageTaken(Unit* /*attacker*/, uint32& /*damage*/, DamageEffectType, SpellSchoolMask) override
        {
            if (_addsSpawned)
                return;
            if (me->GetHealth() <= uint64(me->GetMaxHealth() * ADD_SPAWN_HP_PCT))
            {
                _addsSpawned = true;
                SpawnAdds();
            }
        }

        void SpawnAdds()
        {
            for (uint32 i = 0; i < ADDS_PER_WAVE; ++i)
            {
                float angle = float(i) * (2.0f * float(M_PI) / float(ADDS_PER_WAVE));
                me->SummonCreature(NPC_AKU_MAI_SERVANT,
                    me->GetPositionX() + std::cos(angle) * 4.0f,
                    me->GetPositionY() + std::sin(angle) * 4.0f,
                    me->GetPositionZ(),
                    me->GetOrientation(),
                    TEMPSUMMON_TIMED_DESPAWN,
                    ADD_DESPAWN_MS);
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
                    case EVENT_CURSE:
                        // Curse the tank specifically -- forces the
                        // raid to decurse or watch the MT melt.
                        DoCastVictim(SPELL_CURSE_OF_BLACKFATHOM);
                        _events.ScheduleEvent(EVENT_CURSE, 16s, 22s);
                        break;
                    case EVENT_FEAR:
                        if (Unit* target = SelectTarget(SelectTargetMethod::Random, 0, 25.0f, true))
                            DoCast(target, SPELL_FEAR);
                        _events.ScheduleEvent(EVENT_FEAR, 22s, 30s);
                        break;
                    default:
                        break;
                }
            }

            DoMeleeAttackIfReady();
        }

    private:
        EventMap _events;
        bool     _addsSpawned;
    };
};

void AddSC_boss_gelihast_raid()
{
    new boss_gelihast_raid();
}
