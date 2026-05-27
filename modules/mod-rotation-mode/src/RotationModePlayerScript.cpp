#include "RotationModePlayerScript.h"

#include "HeimbotSettings.h"
#include "Player.h"
#include "RotationModeConfig.h"

namespace RotationMode
{
    PlayerScript::PlayerScript()
        : ::PlayerScript("RotationModePlayerScript", {
              PLAYERHOOK_ON_LOGIN,
              PLAYERHOOK_ON_LOGOUT
          })
    {
    }

    void PlayerScript::OnPlayerLogin(Player* player)
    {
        if (!GetConfig().Enabled || !player)
            return;

        Mode mode = GetMode(player->GetGUID().GetCounter());
        if (mode == Mode::Off)
            return;

        // v0 scaffold: read the saved mode but don't act on it. Step 3 of
        // PE2 reads (class, spec, mode) from StrategySetComposer and
        // applies the ON/OFF strategy lists via .playerbots bot self +
        // strategy commands.
    }

    void PlayerScript::OnPlayerLogout(Player* /*player*/)
    {
        // v0 scaffold: nothing to clean up since nothing is applied yet.
    }

    void RegisterPlayerScript()
    {
        new PlayerScript();
    }
}
