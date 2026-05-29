/*
 * ToT BFD-as-raid: stub for Lady Sarevess (Forked Lightning + add waves).
 * Mechanics ship in Phase 1.1 Step 4. This stub only registers
 * the script name "boss_lady_sarevess_raid" so creature_template.ScriptName can bind.
 */

#include "ScriptMgr.h"
#include "ScriptedCreature.h"

class boss_lady_sarevess : public CreatureScript
{
public:
    boss_lady_sarevess() : CreatureScript("boss_lady_sarevess_raid") { }

    CreatureAI* GetAI(Creature* creature) const override
    {
        return new boss_lady_sarevessAI(creature);
    }

    struct boss_lady_sarevessAI : public ScriptedAI
    {
        boss_lady_sarevessAI(Creature* creature) : ScriptedAI(creature) { }
        // Mechanics added in Phase 1.1 Step 4.
    };
};

void AddSC_boss_lady_sarevess_raid()
{
    new boss_lady_sarevess();
}
