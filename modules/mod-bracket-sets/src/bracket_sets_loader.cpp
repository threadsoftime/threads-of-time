#include "BracketSets.h"
#include "BracketSetsClientItemHook.h"

// AzerothCore's CMake-generated script loader looks for a function named
// Addmod_<module_dir_name>Scripts() to discover and register the module.
// Our module dir is "mod-bracket-sets", so the function name has the dashes
// replaced with underscores: Addmod_bracket_setsScripts.

void Addmod_bracket_setsScripts()
{
    AddBracketSetsScripts();
    new BracketSetsClientItemHook();
}
