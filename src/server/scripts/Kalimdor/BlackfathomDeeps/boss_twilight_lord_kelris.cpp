/*
 * ToT BFD-as-raid: Twilight Lord Kelris (entry 4832).
 *
 * V1 (ToT-original): mind-games caster.
 *   - Mind Blast (8092) on current victim every 6-8s -- the main
 *     damage threat; high single-target burst on the MT.
 *   - Polymorph (118) on a random raid member every ~30s. One
 *     player effectively out of the fight for ~5s; raid compensates.
 *     (Healers especially: don't let a healer linger sheeped.)
 *   - Shadow Word: Pain (589) on a random raid member every ~10s
 *     for shadow flavor / steady raid damage.
 *
 * Phase 1.1 Step 6 (V1). Replaces the V0 SoD Sleep phasing design.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

namespace
{
    enum Spells
    {
        SPELL_MIND_BLAST    = 8092,    // Mind Blast R6
        SPELL_POLYMORPH     = 118,     // Polymorph R1 -- sheep, CC stand-in
        SPELL_SHADOW_PAIN   = 589,     // Shadow Word: Pain R1
    };

    enum EventIds
    {
        EVENT_MIND_BLAST = 1,
        EVENT_POLYMORPH,
        EVENT_SHADOW_PAIN,
    };
}

class boss_twilight_lord_kelris_raid : public CreatureScript
{
public:
    boss_twilight_lord_kelris_raid() : CreatureScript("boss_twilight_lord_kelris_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_twilight_lord_kelris_raidAI(creature);
    }

    struct boss_twilight_lord_kelris_raidAI : public ScriptedAI
    {
        boss_twilight_lord_kelris_raidAI(Creature* creature) : ScriptedAI(creature) { }

        void Reset() override
        {
            _events.Reset();
        }

        void JustEngagedWith(Unit* /*who*/) override
        {
            _events.ScheduleEvent(EVENT_MIND_BLAST,   4s);
            _events.ScheduleEvent(EVENT_SHADOW_PAIN,  8s);
            _events.ScheduleEvent(EVENT_POLYMORPH,   18s);
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
                    case EVENT_MIND_BLAST:
                        DoCastVictim(SPELL_MIND_BLAST);
                        _events.ScheduleEvent(EVENT_MIND_BLAST, 6s, 8s);
                        break;
                    case EVENT_POLYMORPH:
                        // Skip the tank (position 0 by threat); polymorph a
                        // random non-tank player. Bots may not break it
                        // gracefully but the raid sees one player out for ~5s.
                        if (Unit* target = SelectTarget(SelectTargetMethod::Random, 1, 30.0f, true))
                            DoCast(target, SPELL_POLYMORPH);
                        _events.ScheduleEvent(EVENT_POLYMORPH, 28s, 35s);
                        break;
                    case EVENT_SHADOW_PAIN:
                        if (Unit* target = SelectTarget(SelectTargetMethod::Random, 0, 30.0f, true))
                            DoCast(target, SPELL_SHADOW_PAIN);
                        _events.ScheduleEvent(EVENT_SHADOW_PAIN, 9s, 13s);
                        break;
                    default:
                        break;
                }
            }

            DoMeleeAttackIfReady();
        }

    private:
        EventMap _events;
    };
};

void AddSC_boss_twilight_lord_kelris_raid()
{
    new boss_twilight_lord_kelris_raid();
}
