#include "WarforgedCommandScript.h"
#include "WarforgedConfig.h"
#include "WarforgedLootScript.h"
#include "WarforgedSentinelScript.h"
#include "ScriptMgr.h"

using namespace ModWarforged;

// AC calls this on worldserver startup (via the AddSc_<module> convention).
void Addmod_warforgedScripts()
{
    LoadConfig();
    new WarforgedLootScript();
    new WarforgedCommandScript();
    new WarforgedSentinelScript();
}
