/*
 * This file is part of the AzerothCore Project. See AUTHORS file for Copyright information
 *
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
 * FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
 * more details.
 *
 * You should have received a copy of the GNU General Public License along
 * with this program. If not, see <http://www.gnu.org/licenses/>.
 */

#include "LFGMgr.h"
#include "BattlegroundMgr.h"
#include "Chat.h"
#include "CharacterCache.h"
#include "Common.h"
#include "DBCStores.h"
#include "DisableMgr.h"
#include "GameEventMgr.h"
#include "GameTime.h"
#include "Group.h"
#include "GroupMgr.h"
#include "InstanceSaveMgr.h"
#include "LFGGroupData.h"
#include "LFGPlayerData.h"
#include "Language.h"
#include "ObjectMgr.h"
#include "Opcodes.h"
#include "Player.h"
#include "RBAC.h"
#include "ScriptMgr.h"
#include "SharedDefines.h"
#include "SocialMgr.h"
#include "SpellAuras.h"
#include "WorldSession.h"

namespace lfg
{
    LFGMgr::LFGMgr(): m_options(sWorld->getIntConfig(CONFIG_LFG_OPTIONSMASK)), m_Testing(sWorld->getBoolConfig(CONFIG_DEBUG_LFG))
    {
    }

    LFGMgr::~LFGMgr()
    {
        for (LfgRewardContainer::iterator itr = RewardMapStore.begin(); itr != RewardMapStore.end(); ++itr)
            delete itr->second;
    }

    LFGMgr* LFGMgr::instance()
    {
        static LFGMgr instance;
        return &instance;
    }

    void LFGMgr::_LoadFromDB(Field* fields, ObjectGuid guid)
    {
        if (!fields)
            return;

        if (!guid.IsGroup())
            return;

        SetLeader(guid, ObjectGuid::Create<HighGuid::Player>(fields[0].Get<uint32>()));

        uint32 dungeon = fields[17].Get<uint32>();
        uint8 state = fields[18].Get<uint8>();

        if (!dungeon || !state)
            return;

        SetDungeon(guid, dungeon);

        switch (state)
        {
            case LFG_STATE_DUNGEON:
            case LFG_STATE_FINISHED_DUNGEON:
                SetState(guid, (LfgState)state);
                break;
            default:
                break;
        }
    }

    void LFGMgr::_SaveToDB(ObjectGuid guid)
    {
        if (!guid.IsGroup())
            return;

        CharacterDatabasePreparedStatement* stmt = CharacterDatabase.GetPreparedStatement(CHAR_REP_LFG_DATA);
        stmt->SetData(0, guid.GetCounter());
        stmt->SetData(1, GetDungeon(guid));
        stmt->SetData(2, GetState(guid));
        CharacterDatabase.Execute(stmt);
    }

    /// Load rewards for completing dungeons
    void LFGMgr::LoadRewards()
    {
        uint32 oldMSTime = getMSTime();

        for (LfgRewardContainer::iterator itr = RewardMapStore.begin(); itr != RewardMapStore.end(); ++itr)
            delete itr->second;
        RewardMapStore.clear();

        // ORDER BY is very important for GetRandomDungeonReward!
        QueryResult result = WorldDatabase.Query("SELECT dungeonId, maxLevel, firstQuestId, otherQuestId FROM lfg_dungeon_rewards ORDER BY dungeonId, maxLevel ASC");

        if (!result)
        {
            LOG_ERROR("lfg", ">> Loaded 0 lfg dungeon rewards. DB table `lfg_dungeon_rewards` is empty!");
            return;
        }

        uint32 count = 0;

        Field* fields = nullptr;
        do
        {
            fields = result->Fetch();
            uint32 dungeonId = fields[0].Get<uint32>();
            uint32 maxLevel = fields[1].Get<uint8>();
            uint32 firstQuestId = fields[2].Get<uint32>();
            uint32 otherQuestId = fields[3].Get<uint32>();

            if (!GetLFGDungeonEntry(dungeonId))
            {
                LOG_ERROR("lfg", "Dungeon {} specified in table `lfg_dungeon_rewards` does not exist!", dungeonId);
                continue;
            }

            if (!maxLevel || maxLevel > sWorld->getIntConfig(CONFIG_MAX_PLAYER_LEVEL))
            {
                LOG_ERROR("lfg", "Level {} specified for dungeon {} in table `lfg_dungeon_rewards` can never be reached!", maxLevel, dungeonId);
                maxLevel = sWorld->getIntConfig(CONFIG_MAX_PLAYER_LEVEL);
            }

            if (!firstQuestId || !sObjectMgr->GetQuestTemplate(firstQuestId))
            {
                LOG_ERROR("lfg", "First quest {} specified for dungeon {} in table `lfg_dungeon_rewards` does not exist!", firstQuestId, dungeonId);
                continue;
            }

            if (otherQuestId && !sObjectMgr->GetQuestTemplate(otherQuestId))
            {
                LOG_ERROR("lfg", "Other quest {} specified for dungeon {} in table `lfg_dungeon_rewards` does not exist!", otherQuestId, dungeonId);
                otherQuestId = 0;
            }

            RewardMapStore.insert(LfgRewardContainer::value_type(dungeonId, new LfgReward(maxLevel, firstQuestId, otherQuestId)));
            ++count;
        } while (result->NextRow());

        LOG_INFO("server.loading", ">> Loaded {} LFG Dungeon Rewards in {} ms", count, GetMSTimeDiffToNow(oldMSTime));
        LOG_INFO("server.loading", " ");
    }

    void LFGMgr::AddDungeonCooldown(ObjectGuid guid, uint32 dungeonId)
    {
        if (!sWorld->getIntConfig(CONFIG_LFG_DUNGEON_SELECTION_COOLDOWN))
            return;

        DungeonCooldownStore[guid][dungeonId] = GameTime::Now();
    }

    void LFGMgr::CleanupDungeonCooldowns()
    {
        if (!sWorld->getIntConfig(CONFIG_LFG_DUNGEON_SELECTION_COOLDOWN))
            return;

        Seconds cooldownDuration = GetDungeonCooldownDuration();

        for (auto itPlayer = DungeonCooldownStore.begin(); itPlayer != DungeonCooldownStore.end(); )
        {
            for (auto itDungeon = itPlayer->second.begin(); itDungeon != itPlayer->second.end(); )
            {
                if (GameTime::HasElapsed(itDungeon->second, cooldownDuration))
                    itDungeon = itPlayer->second.erase(itDungeon);
                else
                    ++itDungeon;
            }

            if (itPlayer->second.empty())
                itPlayer = DungeonCooldownStore.erase(itPlayer);
            else
                ++itPlayer;
        }
    }

    void LFGMgr::ClearDungeonCooldowns()
    {
        DungeonCooldownStore.clear();
    }

    Seconds LFGMgr::GetDungeonCooldownDuration() const
    {
        return Seconds(sWorld->getIntConfig(CONFIG_LFG_DUNGEON_SELECTION_COOLDOWN) * MINUTE);
    }

    LfgDungeonSet LFGMgr::FilterCooldownDungeons(LfgDungeonSet const& dungeons, LfgRolesMap const& players)
    {
        if (!sWorld->getIntConfig(CONFIG_LFG_DUNGEON_SELECTION_COOLDOWN))
            return dungeons;

        Seconds cooldownDuration = GetDungeonCooldownDuration();

        LfgDungeonSet filtered;
        for (uint32 dungeonId : dungeons)
        {
            bool onCooldown = false;
            for (auto const& playerPair : players)
            {
                auto itPlayer = DungeonCooldownStore.find(playerPair.first);
                if (itPlayer != DungeonCooldownStore.end())
                {
                    auto itDungeon = itPlayer->second.find(dungeonId);
                    if (itDungeon != itPlayer->second.end() && !GameTime::HasElapsed(itDungeon->second, cooldownDuration))
                    {
                        onCooldown = true;
                        break;
                    }
                }
            }

            if (!onCooldown)
                filtered.insert(dungeonId);
        }

        // If all dungeons are on cooldown, return original set to avoid blocking the queue
        if (filtered.empty())
        {
            LOG_DEBUG("lfg", "LFGMgr::FilterCooldownDungeons: All {} dungeons on cooldown for group, bypassing cooldown filter", dungeons.size());
            return dungeons;
        }

        return filtered;
    }

    LFGDungeonData const* LFGMgr::GetLFGDungeon(uint32 id)
    {
        LFGDungeonContainer::const_iterator itr = LfgDungeonStore.find(id);
        if (itr != LfgDungeonStore.end())
            return &(itr->second);

        return nullptr;
    }

    void LFGMgr::LoadLFGDungeons(bool reload /* = false */)
    {
        uint32 oldMSTime = getMSTime();

        LfgDungeonStore.clear();

        // Initialize Dungeon map with data from dbcs
        for (uint32 i = 0; i < sLFGDungeonStore.GetNumRows(); ++i)
        {
            LFGDungeonEntry const* dungeon = sLFGDungeonStore.LookupEntry(i);
            if (!dungeon)
                continue;

            switch (dungeon->TypeID)
            {
                case LFG_TYPE_DUNGEON:
                case LFG_TYPE_HEROIC:
                case LFG_TYPE_RAID:
                case LFG_TYPE_RANDOM:
                    LfgDungeonStore[dungeon->ID] = LFGDungeonData(dungeon);
                    break;
            }
        }

        // Fill teleport locations from DB
        //                                                   0          1           2           3            4
        QueryResult result = WorldDatabase.Query("SELECT dungeonId, position_x, position_y, position_z, orientation FROM lfg_dungeon_template");

        if (!result)
        {
            LOG_ERROR("lfg", ">> Loaded 0 LFG Entrance Positions. DB Table `lfg_dungeon_template` Is Empty!");
            LOG_INFO("server.loading", " ");
            return;
        }

        uint32 count = 0;

        do
        {
            Field* fields = result->Fetch();
            uint32 dungeonId = fields[0].Get<uint32>();
            LFGDungeonContainer::iterator dungeonItr = LfgDungeonStore.find(dungeonId);
            if (dungeonItr == LfgDungeonStore.end())
            {
                LOG_ERROR("lfg", "table `lfg_dungeon_template` contains coordinates for wrong dungeon {}", dungeonId);
                continue;
            }

            LFGDungeonData& data = dungeonItr->second;
            data.x = fields[1].Get<float>();
            data.y = fields[2].Get<float>();
            data.z = fields[3].Get<float>();
            data.o = fields[4].Get<float>();

            ++count;
        } while (result->NextRow());

        LOG_INFO("server.loading", ">> Loaded {} LFG Entrance Positions in {} ms", count, GetMSTimeDiffToNow(oldMSTime));
        LOG_INFO("server.loading", " ");

        // Fill all other teleport coords from areatriggers
        for (LFGDungeonContainer::iterator itr = LfgDungeonStore.begin(); itr != LfgDungeonStore.end(); ++itr)
        {
            LFGDungeonData& dungeon = itr->second;

            // No teleport coords in database, load from areatriggers
            if (dungeon.type != LFG_TYPE_RANDOM && dungeon.x == 0.0f && dungeon.y == 0.0f && dungeon.z == 0.0f)
            {
                AreaTriggerTeleport const* at = sObjectMgr->GetMapEntranceTrigger(dungeon.map);
                if (!at)
                {
                    LOG_ERROR("lfg", "LFGMgr::LoadLFGDungeons: Failed to load dungeon {}, cant find areatrigger for map {}", dungeon.name, dungeon.map);
                    continue;
                }

                dungeon.map = at->target_mapId;
                dungeon.x = at->target_X;
                dungeon.y = at->target_Y;
                dungeon.z = at->target_Z;
                dungeon.o = at->target_Orientation;
            }

            if (dungeon.type != LFG_TYPE_RANDOM)
                CachedDungeonMapStore[dungeon.group].insert(dungeon.id);
            CachedDungeonMapStore[0].insert(dungeon.id);
        }

        if (reload)
        {
            CachedDungeonMapStore.clear();
            // Recalculate locked dungeons
            for (LfgPlayerDataContainer::const_iterator it = PlayersStore.begin(); it != PlayersStore.end(); ++it)
                if (Player* player = ObjectAccessor::FindConnectedPlayer(it->first))
                    InitializeLockedDungeons(player, nullptr);
        }
    }

    /**
        Generate the dungeon lock map for a given player

       @param[in]     player Player we need to initialize the lock status map
    */
    void LFGMgr::InitializeLockedDungeons(Player* player, Group const* group)
    {
        ObjectGuid guid = player->GetGUID();

        uint8 level = player->GetLevel();
        uint8 expansion = player->GetSession()->Expansion();
        LfgDungeonSet const& dungeons = GetDungeonsByRandom(0);
        LfgLockMap lock;

        bool onlySeasonalBosses = m_options == LFG_OPTION_ENABLE_SEASONAL_BOSSES;

        float avgItemLevel = player->GetAverageItemLevelForDF();

        for (LfgDungeonSet::const_iterator it = dungeons.begin(); it != dungeons.end(); ++it)
        {
            LFGDungeonData const* dungeon = GetLFGDungeon(*it);
            if (!dungeon) // should never happen - We provide a list from sLFGDungeonStore
                continue;
            MapEntry const* mapEntry = sMapStore.LookupEntry(dungeon->map);
            DungeonProgressionRequirements const* ar = sObjectMgr->GetAccessRequirement(dungeon->map, Difficulty(dungeon->difficulty));

            uint32 lockData = 0;

            if (!player->GetSession()->HasPermission(rbac::RBAC_PERM_JOIN_DUNGEON_FINDER))
                lockData = LFG_LOCKSTATUS_RAID_LOCKED;
            else if (dungeon->expansion > expansion || (onlySeasonalBosses && !dungeon->seasonal))
                lockData = LFG_LOCKSTATUS_INSUFFICIENT_EXPANSION;
            else if (IsDungeonDisabled(dungeon->map, dungeon->difficulty))
                lockData = LFG_LOCKSTATUS_RAID_LOCKED;
            else if (dungeon->difficulty > DUNGEON_DIFFICULTY_NORMAL && (!mapEntry || !mapEntry->IsRaid()) && sInstanceSaveMgr->PlayerIsPermBoundToInstance(player->GetGUID(), dungeon->map, Difficulty(dungeon->difficulty)))
                lockData = LFG_LOCKSTATUS_RAID_LOCKED;
            else if ((dungeon->minlevel > level && !sWorld->getBoolConfig(CONFIG_DUNGEON_ACCESS_REQUIREMENTS_LFG_DBC_LEVEL_OVERRIDE)) || (sWorld->getBoolConfig(CONFIG_DUNGEON_ACCESS_REQUIREMENTS_LFG_DBC_LEVEL_OVERRIDE) && ar && ar->levelMin > 0 && ar->levelMin > level))
                lockData = LFG_LOCKSTATUS_TOO_LOW_LEVEL;
            else if ((dungeon->maxlevel < level && !sWorld->getBoolConfig(CONFIG_DUNGEON_ACCESS_REQUIREMENTS_LFG_DBC_LEVEL_OVERRIDE)) || (sWorld->getBoolConfig(CONFIG_DUNGEON_ACCESS_REQUIREMENTS_LFG_DBC_LEVEL_OVERRIDE) && ar && ar->levelMax > 0 && ar->levelMax < level))
                lockData = LFG_LOCKSTATUS_TOO_HIGH_LEVEL;
            else if (dungeon->seasonal && !IsSeasonActive(dungeon->id))
                lockData = LFG_LOCKSTATUS_NOT_IN_SEASON;
            else if (player->IsClass(CLASS_DEATH_KNIGHT) && !player->IsGameMaster() &&!(player->IsQuestRewarded(13188) || player->IsQuestRewarded(13189)))
                lockData = LFG_LOCKSTATUS_QUEST_NOT_COMPLETED;
            else if (ar)
            {
                // Check required items
                for (const ProgressionRequirement* itemRequirement : ar->items)
                {
                    if (!itemRequirement->checkLeaderOnly || !group || group->GetLeaderGUID() == player->GetGUID())
                    {
                        if (itemRequirement->faction == TEAM_NEUTRAL || itemRequirement->faction == player->GetTeamId(true))
                        {
                            if (!player->HasItemCount(itemRequirement->id, 1))
                            {
                                lockData = LFG_LOCKSTATUS_MISSING_ITEM;
                                break;
                            }
                        }
                    }
                }

                //Check for quests
                for (const ProgressionRequirement* questRequirement : ar->quests)
                {
                    if (!questRequirement->checkLeaderOnly || !group || group->GetLeaderGUID() == player->GetGUID())
                    {
                        if (questRequirement->faction == TEAM_NEUTRAL || questRequirement->faction == player->GetTeamId(true))
                        {
                            if (!player->GetQuestRewardStatus(questRequirement->id))
                            {
                                lockData = LFG_LOCKSTATUS_QUEST_NOT_COMPLETED;
                                break;
                            }
                        }
                    }
                }

                //Check for ilvl
                if (ar->reqItemLevel && (float)ar->reqItemLevel > avgItemLevel)
                {
                    lockData = LFG_LOCKSTATUS_TOO_LOW_GEAR_SCORE;
                }

                //Check if player has the required achievements
                for (const ProgressionRequirement* achievementRequirement : ar->achievements)
                {
                    if (!achievementRequirement->checkLeaderOnly || !group || group->GetLeaderGUID() == player->GetGUID())
                    {
                        if (achievementRequirement->faction == TEAM_NEUTRAL || achievementRequirement->faction == player->GetTeamId(true))
                        {
                            if (!player->HasAchieved(achievementRequirement->id))
                            {
                                lockData = LFG_LOCKSTATUS_MISSING_ACHIEVEMENT;
                                break;
                            }
                        }
                    }
                }
            }

            sScriptMgr->OnInitializeLockedDungeons(player, level, lockData, dungeon);

            /* TODO VoA closed if WG is not under team control (LFG_LOCKSTATUS_RAID_LOCKED)
                lockData = LFG_LOCKSTATUS_TOO_LOW_GEAR_SCORE;
                lockData = LFG_LOCKSTATUS_TOO_HIGH_GEAR_SCORE;
                lockData = LFG_LOCKSTATUS_ATTUNEMENT_TOO_LOW_LEVEL;
                lockData = LFG_LOCKSTATUS_ATTUNEMENT_TOO_HIGH_LEVEL;
            */

            if (lockData)
                lock[dungeon->Entry()] = lockData;
        }

        sScriptMgr->OnAfterInitializeLockedDungeons(player);

        SetLockedDungeons(guid, lock);
    }

    /**
        Adds the player/group to lfg queue. If player is in a group then it is the leader
        of the group tying to join the group. Join conditions are checked before adding
        to the new queue.

       @param[in]     player Player trying to join (or leader of group trying to join)
       @param[in]     roles Player selected roles
       @param[in]     dungeons Dungeons the player/group is applying for
       @param[in]     comment Player selected comment
    */
    void LFGMgr::JoinLfg(Player* player, uint8 roles, LfgDungeonSet& dungeons, const std::string& comment)
    {
        if (!player || dungeons.empty())
            return;

        Group* grp = player->GetGroup();

        if (grp && (grp->isBGGroup() || grp->isBFGroup()))
            return;

        if (!sScriptMgr->OnPlayerCanJoinLfg(player, roles, dungeons, comment))   // Inc-1 veto/record seam
            return;
        // Native matching body deleted in Inc-3 C1. The slice owns matchmaking.
    }

    void LFGMgr::ToggleTesting()
    {
        if (sWorld->getBoolConfig(CONFIG_DEBUG_LFG))
        {
            m_Testing = true;
            ChatHandler(nullptr).SendWorldText(LANG_DEBUG_LFG_CONF);
        }
        else
        {
            m_Testing = !m_Testing;
            ChatHandler(nullptr).SendWorldText(m_Testing ? LANG_DEBUG_LFG_ON : LANG_DEBUG_LFG_OFF);
        }
    }

    /**
        Leaves Dungeon System. Player/Group is removed from queue, rolechecks, proposals
        or votekicks. Player or group needs to be not nullptr and using Dungeon System

       @param[in]     guid Player or group guid
    */
    void LFGMgr::LeaveLfg(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::Leave: [{}]", guid.ToString());
        ObjectGuid gguid = guid.IsGroup() ? guid : GetGroup(guid);
        LfgState state = GetState(guid);
        switch (state)
        {
            case LFG_STATE_QUEUED:
                // Inc-3 C1: GetQueue/RemoveFromQueue deleted with QueuesStore; preserve state-clear + UI updates
                if (gguid)
                {
                    uint32 dungeonId = GetDungeon(gguid);
                    SetState(gguid, LFG_STATE_NONE);
                    const LfgGuidSet& players = GetPlayers(gguid);
                    for (LfgGuidSet::const_iterator it = players.begin(); it != players.end(); ++it)
                    {
                        SetState(*it, LFG_STATE_NONE);
                        SendLfgUpdateParty(*it, LfgUpdateData(LFG_UPDATETYPE_REMOVED_FROM_QUEUE));
                    }
                    if (Group* group = sGroupMgr->GetGroupByGUID(gguid.GetCounter()))
                    {
                        if (group->isLFGGroup())
                        {
                            SetDungeon(gguid, dungeonId);
                        }
                    }
                }
                else
                {
                    SendLfgUpdatePlayer(guid, LfgUpdateData(LFG_UPDATETYPE_REMOVED_FROM_QUEUE));
                    SetState(guid, LFG_STATE_NONE);
                }
                break;
            case LFG_STATE_ROLECHECK:
                // Inc-3 C1: UpdateRoleCheck deleted; state transitions owned by slice
                break;
            case LFG_STATE_PROPOSAL:
                // Inc-3 C2: ProposalsStore/RemoveProposal deleted; no proposals created
                // post-C1 so GetState never returns PROPOSAL in practice. Dead branch.
                break;
            case LFG_STATE_NONE:
                break;
            case LFG_STATE_DUNGEON:
            case LFG_STATE_FINISHED_DUNGEON:
            case LFG_STATE_BOOT:
                if (guid != gguid) // Player
                    SetState(guid, LFG_STATE_NONE);
                break;
            case LFG_STATE_RAIDBROWSER:
                // Inc-3 C3: LeaveRaidBrowser/SetCanOverrideRBState deleted with Raid Browser.
                // Raid Browser is retired; this case is now a no-op.
                break;
        }
    }

    // JoinRaidBrowser — deleted in Inc-3 C3 (Raid Browser retired; NOT LFR)
    // LeaveRaidBrowser — deleted in Inc-3 C3

    // SendRaidBrowserJoinedPacket — deleted in Inc-3 C3 (Raid Browser retired)
    // LfrSearchAdd — deleted in Inc-3 C3
    // LfrSearchRemove — deleted in Inc-3 C3
    // SendRaidBrowserCachedList — deleted in Inc-3 C3

    // UpdateRaidBrowser — deleted in Inc-3 C3 (Raid Browser retired; NOT LFR)
    // RBPacketAppendGroup — deleted in Inc-3 C3
    // RBPacketAppendPlayer — deleted in Inc-3 C3
    // RBPacketBuildDifference — deleted in Inc-3 C3
    // RBPacketBuildFull — deleted in Inc-3 C3

    // pussywizard: gutted in Inc-3 C1 — QueuesStore deleted; no-op shim retained for call-site compat
    void LFGMgr::LeaveAllLfgQueues(ObjectGuid /*guid*/, bool /*allowgroup*/, ObjectGuid /*groupguid*/)
    {
    }

    // MakeNewGroup — deleted in Inc-3 C2 (proposal machinery removed;
    //   the harness uses LfgFormGroupAdapter + InitGroupForDungeon instead)

    // AddProposal — deleted in Inc-3 C2 (proposal machinery removed)

    // UpdateProposal — deleted in Inc-3 C2 (proposal machinery removed)

    // RemoveProposal — deleted in Inc-3 C2 (proposal machinery removed)

    // InitBoot — deleted in Inc-3 C2 (boot vote machinery removed)
    // UpdateBoot — deleted in Inc-3 C2 (boot vote machinery removed)

    /**
       Set group and all tracked members to LFG_STATE_DUNGEON.

       Called by the harness lfg.form_group adapter after force-creating a group via
       the MakeNewGroup-faithful path.  Mirrors the SetState calls at the bottom of
       MakeNewGroup that could not be reached through a public path before this
       method was added.

       @param[in]     gguid Group GUID
    */
    void LFGMgr::InitGroupForDungeon(ObjectGuid gguid)
    {
        SetState(gguid, LFG_STATE_DUNGEON);
        LfgGuidSet const& players = GetPlayers(gguid);
        for (LfgGuidSet::const_iterator it = players.begin(); it != players.end(); ++it)
            SetState(*it, LFG_STATE_DUNGEON);
    }

    /**
       Teleports the player in or out the dungeon

       @param[in]     player Player to teleport
       @param[in]     out Teleport out (true) or in (false)
       @param[in]     fromOpcode Function called from opcode handlers? (Default false)
    */
    void LFGMgr::TeleportPlayer(Player* player, bool out, WorldLocation const* teleportLocation /*= nullptr*/)
    {
        LFGDungeonData const* dungeon = nullptr;
        Group* group = player->GetGroup();

        if (group && group->isLFGGroup())
            dungeon = GetLFGDungeon(GetDungeon(group->GetGUID()));

        if (!dungeon)
        {
            player->GetSession()->SendLfgTeleportError(uint8(LFG_TELEPORTERROR_INVALID_LOCATION));
            return;
        }

        LfgTeleportError error = LFG_TELEPORTERROR_OK;

        if (!player->IsAlive())
        {
            error = LFG_TELEPORTERROR_PLAYER_DEAD;
        }
        else if (player->IsFalling() || player->HasUnitState(UNIT_STATE_JUMPING))
        {
            error = LFG_TELEPORTERROR_FALLING;
        }
        else if (player->IsMirrorTimerActive(FATIGUE_TIMER))
        {
            error = LFG_TELEPORTERROR_FATIGUE;
        }
        else if (player->GetVehicle())
        {
            error = LFG_TELEPORTERROR_IN_VEHICLE;
        }
        else if (player->GetCharmGUID() || player->IsInCombat())
        {
            error = LFG_TELEPORTERROR_COMBAT;
        }
        else if (out && error == LFG_TELEPORTERROR_OK)
        {
            if (player->GetMapId() == uint32(dungeon->map))
                player->TeleportToEntryPoint();

            return;
        }
        else
        {
            uint32 mapid = dungeon->map;
            float x = dungeon->x;
            float y = dungeon->y;
            float z = dungeon->z;
            float orientation = dungeon->o;

            if (teleportLocation)
            {
                teleportLocation->GetWorldLocation(mapid, x, y, z, orientation);
            }

            if (!player->GetMap()->IsDungeon() || player->GetEntryPoint().GetMapId() == MAPID_INVALID)
            {
                player->SetEntryPoint();
            }

            if (!player->TeleportTo(mapid, x, y, z, orientation, 0, nullptr, mapid == player->GetMapId()))
            {
                error = LFG_TELEPORTERROR_INVALID_LOCATION;
            }
        }

        if (error != LFG_TELEPORTERROR_OK)
        {
            player->GetSession()->SendLfgTeleportError(uint8(error));

            LOG_DEBUG("lfg", "Player [{}] could NOT be teleported in to map [{}] (x: {}, y: {}, z: {}) Error: {}",
            player->GetName(), dungeon->map, dungeon->x, dungeon->y, dungeon->z, error);
        }
        else
        {
            LOG_DEBUG("lfg", "Player [{}] is being teleported in to map [{}] (x: {}, y: {}, z: {})",
            player->GetName(), dungeon->map, dungeon->x, dungeon->y, dungeon->z);
        }

    }

    /**
       Finish a dungeon and give reward, if any.

       @param[in]     guid Group guid
       @param[in]     dungeonId Dungeonid
    */
    void LFGMgr::FinishDungeon(ObjectGuid gguid, const uint32 dungeonId, const Map* currMap)
    {
        uint32 gDungeonId = GetDungeon(gguid);
        if (gDungeonId != dungeonId)
        {
            LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] Finished dungeon {} but group queued for {}. Ignoring", gguid.ToString(), dungeonId, gDungeonId);
            return;
        }

        if (GetState(gguid) == LFG_STATE_FINISHED_DUNGEON) // Shouldn't happen. Do not reward multiple times
        {
            LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] Already rewarded group. Ignoring", gguid.ToString());
            return;
        }

        SetState(gguid, LFG_STATE_FINISHED_DUNGEON);
        _SaveToDB(gguid); // pussywizard

        const LfgGuidSet& players = GetPlayers(gguid);
        for (LfgGuidSet::const_iterator it = players.begin(); it != players.end(); ++it)
        {
            ObjectGuid guid = (*it);
            if (GetState(guid) == LFG_STATE_FINISHED_DUNGEON)
            {
                LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] Already rewarded player. Ignoring", guid.ToString());
                continue;
            }

            uint32 rDungeonId = 0;
            const LfgDungeonSet& dungeons = GetSelectedDungeons(guid);
            if (!dungeons.empty())
                rDungeonId = (*dungeons.begin());

            SetState(guid, LFG_STATE_FINISHED_DUNGEON);

            // Give rewards only if its a random dungeon
            LFGDungeonData const* dungeon = GetLFGDungeon(rDungeonId);

            if (!dungeon || (dungeon->type != LFG_TYPE_RANDOM && !dungeon->seasonal))
            {
                LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] dungeon {} is not random or seasonal", guid.ToString(), rDungeonId);
                continue;
            }

            // Record dungeon cooldown for this player (the actual dungeon completed, not the random entry)
            AddDungeonCooldown(guid, dungeonId);

            Player* player = ObjectAccessor::FindPlayer(guid);
            if (!player || player->FindMap() != currMap) // pussywizard: currMap - multithreading crash if on other map (map id check is not enough, binding system is not reliable)
            {
                LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] not found in world", guid.ToString());
                continue;
            }

            LFGDungeonData const* dungeonDone = GetLFGDungeon(dungeonId);
            uint32 mapId = dungeonDone ? uint32(dungeonDone->map) : 0;

            if (player->GetMapId() != mapId)
            {
                LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] is in map {} and should be in {} to get reward", guid.ToString(), player->GetMapId(), mapId);
                continue;
            }

            // Remove Dungeon Finder Cooldown if still exists
            if (player->HasAura(LFG_SPELL_DUNGEON_COOLDOWN))
            {
                player->RemoveAurasDueToSpell(LFG_SPELL_DUNGEON_COOLDOWN);
            }

            // Xinef: Update achievements, set correct amount of randomly grouped players
            if (dungeon->difficulty == DUNGEON_DIFFICULTY_HEROIC)
                if (uint8 count = GetRandomPlayersCount(player->GetGUID()))
                    player->UpdateAchievementCriteria(ACHIEVEMENT_CRITERIA_TYPE_USE_LFD_TO_GROUP_WITH_PLAYERS, count);

            LfgReward const* reward = GetRandomDungeonReward(rDungeonId, player->GetLevel());
            if (!reward)
                continue;

            bool done = false;
            Quest const* quest = sObjectMgr->GetQuestTemplate(reward->firstQuest);
            if (!quest)
                continue;

            // if we can take the quest, means that we haven't done this kind of "run", IE: First Heroic Random of Day.
            if (player->CanRewardQuest(quest, false))
                player->RewardQuest(quest, 0, nullptr, false, true);
            else
            {
                done = true;
                quest = sObjectMgr->GetQuestTemplate(reward->otherQuest);
                if (!quest)
                    continue;
                // we give reward without informing client (retail does this)
                player->RewardQuest(quest, 0, nullptr, false, true);
            }

            // Give rewards
            LOG_DEBUG("lfg", "LFGMgr::FinishDungeon: [{}] done dungeon {}, {} previously done.", player->GetGUID().ToString(), GetDungeon(gguid), done ? " " : " not");
            LfgPlayerRewardData data = LfgPlayerRewardData(dungeon->Entry(), GetDungeon(gguid, false), done, quest);
            player->GetSession()->SendLfgPlayerReward(data);
        }
    }

    // --------------------------------------------------------------------------//
    // Auxiliar Functions
    // --------------------------------------------------------------------------//

    /**
       Get the dungeon list that can be done given a random dungeon entry.

       @param[in]     randomdungeon Random dungeon id (if value = 0 will return all dungeons)
       @returns Set of dungeons that can be done.
    */
    LfgDungeonSet const& LFGMgr::GetDungeonsByRandom(uint32 randomdungeon)
    {
        LFGDungeonData const* dungeon = GetLFGDungeon(randomdungeon);
        uint32 group = dungeon ? dungeon->group : 0;
        return CachedDungeonMapStore[group];
    }

    /**
       Get the reward of a given random dungeon at a certain level

       @param[in]     dungeon dungeon id
       @param[in]     level Player level
       @returns Reward
    */
    LfgReward const* LFGMgr::GetRandomDungeonReward(uint32 dungeon, uint8 level)
    {
        LfgReward const* rew = nullptr;
        LfgRewardContainerBounds bounds = RewardMapStore.equal_range(dungeon & 0x00FFFFFF);
        for (LfgRewardContainer::const_iterator itr = bounds.first; itr != bounds.second; ++itr)
        {
            rew = itr->second;
            // ordered properly at loading
            if (itr->second->maxLevel >= level)
                break;
        }

        return rew;
    }

    /**
       Given a Dungeon id returns the dungeon Type

       @param[in]     dungeon dungeon id
       @returns Dungeon type
    */
    LfgType LFGMgr::GetDungeonType(uint32 dungeonId)
    {
        LFGDungeonData const* dungeon = GetLFGDungeon(dungeonId);
        if (!dungeon)
            return LFG_TYPE_NONE;

        return LfgType(dungeon->type);
    }

    LfgState LFGMgr::GetState(ObjectGuid guid)
    {
        LfgState state;
        if (guid.IsGroup())
            state = GroupsStore[guid].GetState();
        else
            state = PlayersStore[guid].GetState();

        LOG_DEBUG("lfg", "LFGMgr::GetState: [{}] = {}", guid.ToString(), state);
        return state;
    }

    LfgState LFGMgr::GetOldState(ObjectGuid guid)
    {
        LfgState state;
        if (guid.IsGroup())
            state = GroupsStore[guid].GetOldState();
        else
            state = PlayersStore[guid].GetOldState();

        LOG_DEBUG("lfg", "LFGMgr::GetOldState: [{}] = {}", guid.ToString(), state);
        return state;
    }

    uint32 LFGMgr::GetDungeon(ObjectGuid guid, bool asId /*= true */)
    {
        uint32 dungeon = GroupsStore[guid].GetDungeon(asId);
        LOG_DEBUG("lfg", "LFGMgr::GetDungeon: [{}] asId: {} = {}", guid.ToString(), asId, dungeon);
        return dungeon;
    }

    uint32 LFGMgr::GetDungeonMapId(ObjectGuid guid)
    {
        uint32 dungeonId = GroupsStore[guid].GetDungeon(true);
        uint32 mapId = 0;
        if (dungeonId)
            if (LFGDungeonData const* dungeon = GetLFGDungeon(dungeonId))
                mapId = dungeon->map;

        LOG_DEBUG("lfg", "LFGMgr::GetDungeonMapId: [{}] = {} (DungeonId = {})", guid.ToString(), mapId, dungeonId);
        return mapId;
    }

    uint8 LFGMgr::GetRoles(ObjectGuid guid)
    {
        uint8 roles = PlayersStore[guid].GetRoles();
        LOG_DEBUG("lfg", "LFGMgr::GetRoles: [{}] = {}", guid.ToString(), roles);
        return roles;
    }

    const std::string& LFGMgr::GetComment(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::GetComment: [{}] = {}", guid.ToString(), PlayersStore[guid].GetComment());
        return PlayersStore[guid].GetComment();
    }

    LfgDungeonSet const& LFGMgr::GetSelectedDungeons(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::GetSelectedDungeons: [{}]", guid.ToString());
        return PlayersStore[guid].GetSelectedDungeons();
    }

    LfgLockMap const& LFGMgr::GetLockedDungeons(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::GetLockedDungeons: [{}]", guid.ToString());
        return PlayersStore[guid].GetLockedDungeons();
    }

    uint8 LFGMgr::GetKicksLeft(ObjectGuid guid)
    {
        uint8 kicks = GroupsStore[guid].GetKicksLeft();
        LOG_DEBUG("lfg", "LFGMgr::GetKicksLeft: [{}] = {}", guid.ToString(), kicks);
        return kicks;
    }

    void LFGMgr::RestoreState(ObjectGuid guid, char const*  /*debugMsg*/)
    {
        if (guid.IsGroup())
        {
            LfgGroupData& data = GroupsStore[guid];
            /*if (sLog->ShouldLog(LOG_FILTER_LFG, LOG_LEVEL_DEBUG))
            {
                std::string const& ps = GetStateString(data.GetState());
                std::string const& os = GetStateString(data.GetOldState());
                LOG_TRACE("lfg", "LFGMgr::RestoreState: Group: [{}] ({}) State: {}, oldState: {}",
                    guid.ToString(), debugMsg, ps, os);
            }*/

            data.RestoreState();
        }
        else
        {
            LfgPlayerData& data = PlayersStore[guid];
            /*if (sLog->ShouldLog(LOG_FILTER_LFG, LOG_LEVEL_DEBUG))
            {
                std::string const& ps = GetStateString(data.GetState());
                std::string const& os = GetStateString(data.GetOldState());
                LOG_TRACE("lfg", "LFGMgr::RestoreState: Player: [{}] ({}) State: {}, oldState: {}",
                    guid.ToString(), debugMsg, ps, os);
            }*/
            data.RestoreState();
        }
    }

    void LFGMgr::SetState(ObjectGuid guid, LfgState state)
    {
        if (guid.IsGroup())
        {
            LfgGroupData& data = GroupsStore[guid];
            std::string ns = GetStateString(state);
            std::string ps = GetStateString(data.GetState());
            std::string os = GetStateString(data.GetOldState());
            LOG_DEBUG("lfg", "LFGMgr::SetState: Group: [{}] newState: {}, previous: {}, oldState: {}", guid.ToString(), ns, ps, os);
            data.SetState(state);
        }
        else
        {
            LfgPlayerData& data = PlayersStore[guid];
            std::string ns = GetStateString(state);
            std::string ps = GetStateString(data.GetState());
            std::string os = GetStateString(data.GetOldState());
            LOG_DEBUG("lfg", "LFGMgr::SetState: Player: [{}] newState: {}, previous: {}, oldState: {}", guid.ToString(), ns, ps, os);
            data.SetState(state);
        }
    }

    // SetCanOverrideRBState — deleted in Inc-3 C3 (Raid Browser retired)

    void LFGMgr::SetDungeon(ObjectGuid guid, uint32 dungeon)
    {
        LOG_DEBUG("lfg", "LFGMgr::SetDungeon: [{}] dungeon {}", guid.ToString(), dungeon);
        GroupsStore[guid].SetDungeon(dungeon);
    }

    void LFGMgr::SetRoles(ObjectGuid guid, uint8 roles)
    {
        LOG_DEBUG("lfg", "LFGMgr::SetRoles: [{}] roles: {}", guid.ToString(), roles);
        PlayersStore[guid].SetRoles(roles);
    }

    void LFGMgr::SetComment(ObjectGuid guid, std::string const& comment)
    {
        LOG_DEBUG("lfg", "LFGMgr::SetComment: [{}] comment: {}", guid.ToString(), comment);
        PlayersStore[guid].SetComment(comment);
    }

    // LfrSetComment — deleted in Inc-3 C3 (Raid Browser retired)

    void LFGMgr::SetSelectedDungeons(ObjectGuid guid, LfgDungeonSet const& dungeons)
    {
        LOG_DEBUG("lfg", "LFGMgr::SetLockedDungeons: [{}]", guid.ToString());
        PlayersStore[guid].SetSelectedDungeons(dungeons);
    }

    void LFGMgr::SetLockedDungeons(ObjectGuid guid, LfgLockMap const& lock)
    {
        LOG_DEBUG("lfg", "LFGMgr::SetLockedDungeons: [{}]", guid.ToString());
        PlayersStore[guid].SetLockedDungeons(lock);
    }

    void LFGMgr::DecreaseKicksLeft(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::DecreaseKicksLeft: [{}]", guid.ToString());
        GroupsStore[guid].DecreaseKicksLeft();
    }

    void LFGMgr::RemoveGroupData(ObjectGuid guid)
    {
        LOG_DEBUG("lfg", "LFGMgr::RemoveGroupData: [{}]", guid.ToString());
        LfgGroupDataContainer::iterator it = GroupsStore.find(guid);
        if (it == GroupsStore.end())
            return;

        LfgState state = GetState(guid);
        // If group is being formed after proposal success do nothing more
        LfgGuidSet const& players = it->second.GetPlayers();
        for (auto iterator = players.begin(); iterator != players.end(); ++iterator)
        {
            ObjectGuid objectGuid = (*iterator);
            SetGroup(*iterator, ObjectGuid::Empty);
            if (state != LFG_STATE_PROPOSAL)
            {
                SetState(*iterator, LFG_STATE_NONE);
                SendLfgUpdateParty(objectGuid, LfgUpdateData(LFG_UPDATETYPE_REMOVED_FROM_QUEUE));
            }
        }
        GroupsStore.erase(it);
    }

    TeamId LFGMgr::GetTeam(ObjectGuid guid)
    {
        return PlayersStore[guid].GetTeam();
    }

    uint8 LFGMgr::RemovePlayerFromGroup(ObjectGuid gguid, ObjectGuid guid)
    {
        return GroupsStore[gguid].RemovePlayer(guid);
    }

    void LFGMgr::AddPlayerToGroup(ObjectGuid gguid, ObjectGuid guid)
    {
        GroupsStore[gguid].AddPlayer(guid);
    }

    void LFGMgr::AddPlayerQueuedForRandomDungeonToGroup(ObjectGuid gguid, ObjectGuid guid)
    {
        const LfgDungeonSet& dungeons = GetSelectedDungeons(guid);
        if (dungeons.empty())
            return;

        uint32 dungeonId = *dungeons.begin();
        LFGDungeonData const* dungeon = GetLFGDungeon(dungeonId);
        if (dungeon && (dungeon->type == LFG_TYPE_RANDOM))
            GroupsStore[gguid].AddRandomQueuedPlayer(guid);
    }

    void LFGMgr::SetLeader(ObjectGuid gguid, ObjectGuid leader)
    {
        GroupsStore[gguid].SetLeader(leader);
    }

    void LFGMgr::SetTeam(ObjectGuid guid, TeamId teamId)
    {
        if (sWorld->getBoolConfig(CONFIG_ALLOW_TWO_SIDE_INTERACTION_GROUP))
            teamId = TEAM_ALLIANCE; // @Not Sure About That TeamId is supposed to be uint8 Team = 0(@TrinityCore)

        PlayersStore[guid].SetTeam(teamId);
    }

    ObjectGuid LFGMgr::GetGroup(ObjectGuid guid)
    {
        return PlayersStore[guid].GetGroup();
    }

    void LFGMgr::SetGroup(ObjectGuid guid, ObjectGuid group)
    {
        PlayersStore[guid].SetGroup(group);
    }

    LfgGuidSet const& LFGMgr::GetPlayers(ObjectGuid guid)
    {
        return GroupsStore[guid].GetPlayers();
    }

    uint8 LFGMgr::GetPlayerCount(ObjectGuid guid)
    {
        return GroupsStore[guid].GetPlayerCount();
    }

    ObjectGuid LFGMgr::GetLeader(ObjectGuid guid)
    {
        return GroupsStore[guid].GetLeader();
    }

    void LFGMgr::SetRandomPlayersCount(ObjectGuid guid, uint8 count)
    {
        PlayersStore[guid].SetRandomPlayersCount(count);
    }

    uint8 LFGMgr::GetRandomPlayersCount(ObjectGuid guid)
    {
        return PlayersStore[guid].GetRandomPlayersCount();
    }

    bool LFGMgr::HasIgnore(ObjectGuid guid1, ObjectGuid guid2)
    {
        Player* plr1 = ObjectAccessor::FindConnectedPlayer(guid1);
        Player* plr2 = ObjectAccessor::FindConnectedPlayer(guid2);
        return plr1 && plr2 && (plr1->GetSocial()->HasIgnore(guid2) || plr2->GetSocial()->HasIgnore(guid1));
    }

    // SendLfgRoleChosen — deleted in Inc-3 C2 (role-check machinery removed)
    // SendLfgRoleCheckUpdate — deleted in Inc-3 C2 (role-check machinery removed)

    void LFGMgr::SendLfgUpdatePlayer(ObjectGuid guid, LfgUpdateData const& data)
    {
        if (Player* player = ObjectAccessor::FindConnectedPlayer(guid))
            player->GetSession()->SendLfgUpdatePlayer(data);
    }

    void LFGMgr::SendLfgUpdateParty(ObjectGuid guid, LfgUpdateData const& data)
    {
        if (Player* player = ObjectAccessor::FindConnectedPlayer(guid))
            player->GetSession()->SendLfgUpdateParty(data);
    }

    void LFGMgr::SendLfgJoinResult(ObjectGuid guid, LfgJoinResultData const& data)
    {
        if (Player* player = ObjectAccessor::FindConnectedPlayer(guid))
            player->GetSession()->SendLfgJoinResult(data);
    }

    // SendLfgBootProposalUpdate — deleted in Inc-3 C2 (boot machinery removed)
    // SendLfgUpdateProposal — deleted in Inc-3 C2 (proposal machinery removed)

    void LFGMgr::SendLfgQueueStatus(ObjectGuid guid, LfgQueueStatusData const& data)
    {
        if (Player* player = ObjectAccessor::FindConnectedPlayer(guid))
            player->GetSession()->SendLfgQueueStatus(data);
    }

    bool LFGMgr::IsLfgGroup(ObjectGuid guid)
    {
        return guid && guid.IsGroup() && GroupsStore[guid].IsLfgGroup();
    }

    // Only for debugging purposes
    void LFGMgr::Clean()
    {
        /* QueuesStore deleted in Inc-3 C1; no-op shim */
    }

    bool LFGMgr::isOptionEnabled(uint32 option)
    {
        return m_options & option;
    }

    uint32 LFGMgr::GetOptions()
    {
        return m_options;
    }

    void LFGMgr::SetOptions(uint32 options)
    {
        m_options = options;
    }

    LfgUpdateData LFGMgr::GetLfgStatus(ObjectGuid guid)
    {
        LfgPlayerData& playerData = PlayersStore[guid];
        return LfgUpdateData(LFG_UPDATETYPE_UPDATE_STATUS, playerData.GetState(), playerData.GetSelectedDungeons());
    }

    bool LFGMgr::IsSeasonActive(uint32 dungeonId)
    {
        switch (dungeonId)
        {
            case LFG_DUNGEON_HEADLESS_HORSEMAN:
                return IsHolidayActive(HOLIDAY_HALLOWS_END);
            case LFG_DUNGEON_FROST_LORD_AHUNE:
                return IsHolidayActive(HOLIDAY_FIRE_FESTIVAL);
            case LFG_DUNGEON_COREN_DIREBREW:
                return IsHolidayActive(HOLIDAY_BREWFEST);
            case LFG_DUNGEON_CROWN_CHEMICAL_CO:
                return IsHolidayActive(HOLIDAY_LOVE_IS_IN_THE_AIR);
        }
        return false;
    }

    void LFGMgr::SetupGroupMember(ObjectGuid guid, ObjectGuid gguid)
    {
        LfgDungeonSet dungeons;
        dungeons.insert(GetDungeon(gguid));
        SetSelectedDungeons(guid, dungeons);
        SetState(guid, GetState(gguid));
        SetGroup(guid, gguid);
        AddPlayerToGroup(gguid, guid);
    }

    bool LFGMgr::selectedRandomLfgDungeon(ObjectGuid guid)
    {
        if (GetState(guid) != LFG_STATE_NONE)
        {
            LfgDungeonSet const& dungeons = GetSelectedDungeons(guid);
            if (!dungeons.empty())
            {
                LFGDungeonData const* dungeon = GetLFGDungeon(*dungeons.begin());
                if (dungeon && (dungeon->type == LFG_TYPE_RANDOM || dungeon->seasonal))
                    return true;
            }
        }

        return false;
    }

    bool LFGMgr::inLfgDungeonMap(ObjectGuid guid, uint32 map, Difficulty difficulty)
    {
        if (!guid.IsGroup())
            guid = GetGroup(guid);

        if (uint32 dungeonId = GetDungeon(guid, true))
            if (LFGDungeonData const* dungeon = GetLFGDungeon(dungeonId))
                if (uint32(dungeon->map) == map && dungeon->difficulty == difficulty)
                    return true;

        return false;
    }

    uint32 LFGMgr::GetLFGDungeonEntry(uint32 id)
    {
        if (id)
            if (LFGDungeonData const* dungeon = GetLFGDungeon(id))
                return dungeon->Entry();

        return 0;
    }

    LfgDungeonSet LFGMgr::GetRandomAndSeasonalDungeons(uint8 level, uint8 expansion)
    {
        LfgDungeonSet randomDungeons;
        for (lfg::LFGDungeonContainer::const_iterator itr = LfgDungeonStore.begin(); itr != LfgDungeonStore.end(); ++itr)
        {
            lfg::LFGDungeonData const& dungeon = itr->second;
            if ((dungeon.type == lfg::LFG_TYPE_RANDOM || (dungeon.seasonal && sLFGMgr->IsSeasonActive(dungeon.id)))
                    && dungeon.expansion <= expansion && dungeon.minlevel <= level && level <= dungeon.maxlevel)
                randomDungeons.insert(dungeon.Entry());
        }
        return randomDungeons;
    }

    bool LFGMgr::IsDungeonDisabled(uint32 mapId, Difficulty difficulty) const
    {
        return sDisableMgr->IsDisabledFor(DISABLE_TYPE_MAP, mapId, nullptr, difficulty) ||
            sDisableMgr->IsDisabledFor(DISABLE_TYPE_LFG_MAP, mapId, nullptr);
    }

    bool LFGMgr::IsPlayerQueuedForRandomDungeon(ObjectGuid guid)
    {
        auto gguid = GetGroup(guid);
        if (!gguid)
            return false;

        return GroupsStore[gguid].IsRandomQueuedPlayer(guid);
    }
} // namespace lfg
