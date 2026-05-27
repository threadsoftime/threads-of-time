#ifndef MOD_ROTATION_MODE_PLAYER_SCRIPT_H
#define MOD_ROTATION_MODE_PLAYER_SCRIPT_H

#include "ScriptMgr.h"

namespace RotationMode
{
    class PlayerScript : public ::PlayerScript
    {
    public:
        PlayerScript();

        // Re-apply the player's saved mode after they log in (mode persists
        // across logout/login via heimbot_settings).
        void OnPlayerLogin(Player* player) override;

        // Clean up any in-memory state we may attach to the player.
        void OnPlayerLogout(Player* player) override;
    };

    void RegisterPlayerScript();
}

#endif // MOD_ROTATION_MODE_PLAYER_SCRIPT_H
