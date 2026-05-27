#include "WarforgedConfig.h"
#include "Config.h"     // AC's global config reader
#include "Log.h"

namespace ModWarforged
{
    Config gConfig;

    void LoadConfig()
    {
        gConfig.enable           = sConfigMgr->GetOption<bool>  ("Warforged.Enable",            true);
        gConfig.procChance       = sConfigMgr->GetOption<uint8> ("Warforged.ProcChance",        10);
        gConfig.socketChance     = sConfigMgr->GetOption<uint8> ("Warforged.SocketChance",      10);
        gConfig.minQuality       = sConfigMgr->GetOption<uint8> ("Warforged.MinQuality",        2);
        gConfig.maxQuality       = sConfigMgr->GetOption<uint8> ("Warforged.MaxQuality",        4);
        gConfig.announceChannel  = sConfigMgr->GetOption<uint8> ("Warforged.AnnounceChannel",   2);
        gConfig.playLootSound    = sConfigMgr->GetOption<bool>  ("Warforged.PlayLootSound",     true);
        gConfig.lootSoundId      = sConfigMgr->GetOption<uint32>("Warforged.LootSoundId",       3175);
        gConfig.warnMissingPatch = sConfigMgr->GetOption<bool>  ("Warforged.WarnMissingPatch",  false);
        gConfig.socketMinCharLevel = sConfigMgr->GetOption<uint8>("Warforged.SocketMinCharLevel", 56);

        LOG_INFO("server.loading",
                 "[mod-warforged] loaded: enable={}, proc={}%, socket={}% (min char lvl {}), quality=[{},{}], announce={}",
                 gConfig.enable, gConfig.procChance, gConfig.socketChance, gConfig.socketMinCharLevel,
                 gConfig.minQuality, gConfig.maxQuality, gConfig.announceChannel);
    }
}
