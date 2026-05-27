/*
 * Heimdal BFD-as-raid: stub for Baron Aquanis (positional, pools + Bubble Beam).
 * Mechanics ship in Phase 1.1 Step 4. This stub only registers
 * the script name "boss_baron_aquanis_raid" so creature_template.ScriptName can bind.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

class boss_baron_aquanis : public CreatureScript
{
public:
    boss_baron_aquanis() : CreatureScript("boss_baron_aquanis_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_baron_aquanisAI(creature);
    }

    struct boss_baron_aquanisAI : public ScriptedAI
    {
        boss_baron_aquanisAI(Creature* creature) : ScriptedAI(creature) { }
        // Mechanics added in Phase 1.1 Step 4.
    };
};

void AddSC_boss_baron_aquanis_raid()
{
    new boss_baron_aquanis();
}
