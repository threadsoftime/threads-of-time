/*
 * Heimdal BFD-as-raid: stub for Lorgus Jett (rotating corrupted totems).
 * Mechanics ship in Phase 1.1 Step 4. This stub only registers
 * the script name "boss_lorgus_jett_raid" so creature_template.ScriptName can bind.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

class boss_lorgus_jett : public CreatureScript
{
public:
    boss_lorgus_jett() : CreatureScript("boss_lorgus_jett_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_lorgus_jettAI(creature);
    }

    struct boss_lorgus_jettAI : public ScriptedAI
    {
        boss_lorgus_jettAI(Creature* creature) : ScriptedAI(creature) { }
        // Mechanics added in Phase 1.1 Step 4.
    };
};

void AddSC_boss_lorgus_jett_raid()
{
    new boss_lorgus_jett();
}
