#ifndef MOD_HARNESS_BRIDGE_H
#define MOD_HARNESS_BRIDGE_H

// Umbrella header for mod-harness-bridge.
// The loader (mod_harness_bridge_loader.cpp) calls AddHarnessBridgeScripts()
// which registers the WorldScript. Task 3 adds the cpp-httplib server
// lifecycle to this WorldScript; subsequent tasks wire dispatch + adapters.

void AddHarnessBridgeScripts();

// Stage 3 (Inc-1): LfgVetoScript — forward declared here so HarnessBridge.cpp
// can call it from AddHarnessBridgeScripts() without including the full TU.
void AddLfgVetoScript();

#endif // MOD_HARNESS_BRIDGE_H
