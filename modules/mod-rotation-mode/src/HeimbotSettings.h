#ifndef MOD_ROTATION_MODE_HEIMBOT_SETTINGS_H
#define MOD_ROTATION_MODE_HEIMBOT_SETTINGS_H

#include "Common.h"

#include <optional>
#include <string>

namespace RotationMode
{
    // Per-character KV store backed by acore_characters.heimbot_settings.
    //
    // v0 scaffold: methods are stubs. Step 2 of PE2 implementation wires the
    // DB queries (CharacterDatabase prepared statements).

    // Read a setting for the given character. Returns nullopt if not set.
    std::optional<std::string> Get(uint64 character_guid, std::string const& key);

    // Write a setting. Upserts on (character_guid, key).
    void Set(uint64 character_guid, std::string const& key, std::string const& value);

    // Remove a single setting. No-op if not set.
    void Unset(uint64 character_guid, std::string const& key);

    // Wipe all settings for a character (revert to defaults).
    void ResetAll(uint64 character_guid);

    // Returns the typed "mode" setting with default "off" when unset or invalid.
    enum class Mode { Off, Rotation, Grind, SquadMirror };
    Mode GetMode(uint64 character_guid);
    void SetMode(uint64 character_guid, Mode mode);

    // Convert mode to/from the string used in chat commands and DB rows.
    char const* ModeToString(Mode mode);
    Mode ModeFromString(std::string const& s);
}

#endif // MOD_ROTATION_MODE_HEIMBOT_SETTINGS_H
