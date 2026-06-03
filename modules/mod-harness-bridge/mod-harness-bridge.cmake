#
# mod-harness-bridge.cmake
#
# This file is discovered and included by modules/CMakeLists.txt via:
#
#   include("${CMAKE_SOURCE_DIR}/modules/${SOURCE_MODULE}/${SOURCE_MODULE}.cmake" OPTIONAL)
#
# That include fires AFTER src/server/game/CMakeLists.txt has already created the
# game-interface INTERFACE target (and after CU_RUN_HOOK(BEFORE_GAME_LIBRARY) has
# fired), so we can call target_include_directories on game-interface directly here.
# Using CU_ADD_HOOK("BEFORE_GAME_LIBRARY", ...) is not applicable: that hook fires
# before game-interface exists.
#
# WHY THIS FILE EXISTS (Inc-2 leave seam):
#   The Inc-2 leave seam added #include "LfgIntentStore.h" and calls to
#   HarnessBridge::RecordLfgCancel in worldserver CORE translation units:
#     src/server/game/.../LFGHandler.cpp
#     src/server/game/.../LFGScripts.cpp
#     src/server/game/.../BattleGroundHandler.cpp  (game target)
#     src/server/scripts/Commands/cs_misc.cpp       (scripts target)
#   Both game and scripts link PUBLIC game-interface, so adding the module src/
#   directory to game-interface's INTERFACE_INCLUDE_DIRECTORIES propagates the
#   include path to both targets without modifying any core CMakeLists.txt.
#

target_include_directories(game-interface
  INTERFACE
    "${CMAKE_SOURCE_DIR}/modules/mod-harness-bridge/src")
