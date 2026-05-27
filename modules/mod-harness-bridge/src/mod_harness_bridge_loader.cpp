#include "HarnessBridge.h"

// AzerothCore's module discovery looks for Addmod_<dir_name>Scripts.
// Our module dir is "mod-harness-bridge" → dashes become underscores.

void Addmod_harness_bridgeScripts()
{
    AddHarnessBridgeScripts();
}
