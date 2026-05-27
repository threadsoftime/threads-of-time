/*
 * Heimdal BFD-as-raid: stub for Ghamoo-ra (Aqua Shell + tank-swap).
 * Mechanics ship in Phase 1.1 Step 4. This stub only registers
 * the script name "boss_ghamoo_ra_raid" so creature_template.ScriptName can bind.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

class boss_ghamoo_ra : public CreatureScript
{
public:
    boss_ghamoo_ra() : CreatureScript("boss_ghamoo_ra_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_ghamoo_raAI(creature);
    }

    struct boss_ghamoo_raAI : public ScriptedAI
    {
        boss_ghamoo_raAI(Creature* creature) : ScriptedAI(creature) { }
        // Mechanics added in Phase 1.1 Step 4.
    };
};

void AddSC_boss_ghamoo_ra_raid()
{
    new boss_ghamoo_ra();
}
