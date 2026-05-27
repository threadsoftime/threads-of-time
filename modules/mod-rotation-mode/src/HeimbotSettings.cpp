#include "HeimbotSettings.h"

#include "DatabaseEnv.h"
#include "Log.h"

#include <cstring>

namespace RotationMode
{
    std::optional<std::string> Get(uint64 /*character_guid*/, std::string const& /*key*/)
    {
        // v0 scaffold: stub. Step 2 wires:
        //   SELECT setting_value FROM heimbot_settings
        //   WHERE character_guid = ? AND setting_key = ?
        return std::nullopt;
    }

    void Set(uint64 /*character_guid*/, std::string const& /*key*/, std::string const& /*value*/)
    {
        // v0 scaffold: stub. Step 2 wires:
        //   INSERT INTO heimbot_settings (character_guid, setting_key, setting_value)
        //   VALUES (?, ?, ?)
        //   ON DUPLICATE KEY UPDATE setting_value = VALUES(setting_value)
    }

    void Unset(uint64 /*character_guid*/, std::string const& /*key*/)
    {
        // v0 scaffold: stub. Step 2 wires DELETE.
    }

    void ResetAll(uint64 /*character_guid*/)
    {
        // v0 scaffold: stub. Step 2 wires DELETE WHERE character_guid = ?.
    }

    Mode GetMode(uint64 character_guid)
    {
        auto raw = Get(character_guid, "mode");
        if (!raw)
            return Mode::Off;
        return ModeFromString(*raw);
    }

    void SetMode(uint64 character_guid, Mode mode)
    {
        Set(character_guid, "mode", ModeToString(mode));
    }

    char const* ModeToString(Mode mode)
    {
        switch (mode)
        {
            case Mode::Off:         return "off";
            case Mode::Rotation:    return "rotation";
            case Mode::Grind:       return "grind";
            case Mode::SquadMirror: return "squad-mirror";
        }
        return "off";
    }

    Mode ModeFromString(std::string const& s)
    {
        if (s == "rotation")     return Mode::Rotation;
        if (s == "grind")        return Mode::Grind;
        if (s == "squad-mirror") return Mode::SquadMirror;
        return Mode::Off;
    }
}
