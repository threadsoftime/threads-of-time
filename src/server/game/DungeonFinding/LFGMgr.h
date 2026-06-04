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

#ifndef _LFGMGR_H
#define _LFGMGR_H

#include <utility>

#include "DBCStructure.h"
#include "Field.h"
#include "LFG.h"
#include "LFGGroupData.h"
#include "LFGPlayerData.h"
#include "Map.h"

class Group;
class Player;
class Quest;
class WorldLocation;

namespace lfg
{

    enum LfgOptions
    {
        LFG_OPTION_ENABLE_DUNGEON_FINDER             = 0x01,
        // LFG_OPTION_ENABLE_RAID_BROWSER = 0x02 — deleted in Inc-3 C3 (Raid Browser retired)
        LFG_OPTION_ENABLE_SEASONAL_BOSSES            = 0x04
    };

    enum LFGMgrEnum
    {
        LFG_TIME_ROLECHECK                           = 45 * IN_MILLISECONDS,
        LFG_TIME_BOOT                                = 120,
        LFG_TIME_PROPOSAL                            = 40,
        LFG_QUEUEUPDATE_INTERVAL                     = 8 * IN_MILLISECONDS,
        LFG_SPELL_DUNGEON_COOLDOWN                   = 71328,
        LFG_SPELL_DUNGEON_DESERTER                   = 71041,
        LFG_SPELL_LUCK_OF_THE_DRAW                   = 72221,
        LFG_GROUP_KICK_VOTES_NEEDED                  = 3
    };

    enum LfgFlags
    {
        LFG_FLAG_UNK1                                = 0x1,
        LFG_FLAG_UNK2                                = 0x2,
        LFG_FLAG_SEASONAL                            = 0x4,
        LFG_FLAG_UNK3                                = 0x8
    };

    /// Determines the type of instance
    enum LfgType
    {
        LFG_TYPE_NONE                                = 0,
        LFG_TYPE_DUNGEON                             = 1,
        LFG_TYPE_RAID                                = 2,
        LFG_TYPE_ZONE                                = 4,
        LFG_TYPE_HEROIC                              = 5,
        LFG_TYPE_RANDOM                              = 6
    };

    /// Proposal states
    enum LfgProposalState
    {
        LFG_PROPOSAL_INITIATING                      = 0,
        LFG_PROPOSAL_FAILED                          = 1,
        LFG_PROPOSAL_SUCCESS                         = 2
    };

    /// Teleport errors
    enum LfgTeleportError
    {
        // 7 = "You can't do that right now" | 5 = No client reaction
        LFG_TELEPORTERROR_OK                         = 0,      // Internal use
        LFG_TELEPORTERROR_PLAYER_DEAD                = 1,
        LFG_TELEPORTERROR_FALLING                    = 2,
        LFG_TELEPORTERROR_IN_VEHICLE                 = 3,
        LFG_TELEPORTERROR_FATIGUE                    = 4,
        LFG_TELEPORTERROR_INVALID_LOCATION           = 6,
        LFG_TELEPORTERROR_COMBAT                     = 8       // FIXME - It can be 7 or 8 (Need proper data)
    };

    /// Queue join results
    enum LfgJoinResult
    {
        // 3 = No client reaction | 18 = "Rolecheck failed"
        LFG_JOIN_OK                                  = 0,      // Joined (no client msg)
        LFG_JOIN_FAILED                              = 1,      // RoleCheck Failed
        LFG_JOIN_GROUPFULL                           = 2,      // Your group is full
        LFG_JOIN_INTERNAL_ERROR                      = 4,      // Internal LFG Error
        LFG_JOIN_NOT_MEET_REQS                       = 5,      // You do not meet the requirements for the chosen dungeons
        LFG_JOIN_PARTY_NOT_MEET_REQS                 = 6,      // One or more party members do not meet the requirements for the chosen dungeons
        LFG_JOIN_MIXED_RAID_DUNGEON                  = 7,      // You cannot mix dungeons, raids, and random when picking dungeons
        LFG_JOIN_MULTI_REALM                         = 8,      // The dungeon you chose does not support players from multiple realms
        LFG_JOIN_DISCONNECTED                        = 9,      // One or more party members are pending invites or disconnected
        LFG_JOIN_PARTY_INFO_FAILED                   = 10,     // Could not retrieve information about some party members
        LFG_JOIN_DUNGEON_INVALID                     = 11,     // One or more dungeons was not valid
        LFG_JOIN_DESERTER                            = 12,     // You can not queue for dungeons until your deserter debuff wears off
        LFG_JOIN_PARTY_DESERTER                      = 13,     // One or more party members has a deserter debuff
        LFG_JOIN_RANDOM_COOLDOWN                     = 14,     // You can not queue for random dungeons while on random dungeon cooldown
        LFG_JOIN_PARTY_RANDOM_COOLDOWN               = 15,     // One or more party members are on random dungeon cooldown
        LFG_JOIN_TOO_MUCH_MEMBERS                    = 16,     // You can not enter dungeons with more that 5 party members
        LFG_JOIN_USING_BG_SYSTEM                     = 17      // You can not use the dungeon system while in BG or arenas
    };

    /// Role check states
    enum LfgRoleCheckState
    {
        LFG_ROLECHECK_DEFAULT                        = 0,      // Internal use = Not initialized.
        LFG_ROLECHECK_FINISHED                       = 1,      // Role check finished
        LFG_ROLECHECK_INITIALITING                   = 2,      // Role check begins
        LFG_ROLECHECK_MISSING_ROLE                   = 3,      // Someone didn't selected a role after 2 mins
        LFG_ROLECHECK_WRONG_ROLES                    = 4,      // Can't form a group with that role selection
        LFG_ROLECHECK_ABORTED                        = 5,      // Someone leave the group
        LFG_ROLECHECK_NO_ROLE                        = 6       // Someone selected no role
    };

    // LfgUpdateFlag — deleted in Inc-3 C4 (only consumer was Raid Browser machinery, deleted in C3)

    enum LfgSeasonalDungeons
    {
        LFG_DUNGEON_HEADLESS_HORSEMAN   = 285,
        LFG_DUNGEON_FROST_LORD_AHUNE    = 286,
        LFG_DUNGEON_COREN_DIREBREW      = 287,
        LFG_DUNGEON_CROWN_CHEMICAL_CO   = 288
    };

    // RBEntryInfo — deleted in Inc-3 C3 (Raid Browser retired)
    // RBInternalInfo — deleted in Inc-3 C3

    // Forward declaration (just to have all typedef together)
    struct LFGDungeonData;
    struct LfgReward;
    // LfgQueueInfo — unused; forward decl only, no definition (Inc-3 C4 cleanup)
    // LfgRoleCheck — deleted in Inc-3 C2 (role-check machinery removed)
    // LfgProposal — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgProposalPlayer — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgPlayerBoot — deleted in Inc-3 C2 (boot vote machinery removed)

    typedef std::multimap<uint32, LfgReward const*> LfgRewardContainer;
    typedef std::pair<LfgRewardContainer::const_iterator, LfgRewardContainer::const_iterator> LfgRewardContainerBounds;
    typedef std::map<uint8, LfgDungeonSet> LfgCachedDungeonContainer;
    // LfgAnswerContainer — deleted in Inc-3 C4 (only consumer was LfgPlayerBoot, deleted in C2)
    // LfgRoleCheckContainer — deleted in Inc-3 C2 (role-check machinery removed)
    // LfgProposalContainer — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgProposalPlayerContainer — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgPlayerBootContainer — deleted in Inc-3 C2 (boot vote machinery removed)
    typedef std::map<ObjectGuid, LfgGroupData> LfgGroupDataContainer;
    typedef std::map<ObjectGuid, LfgPlayerData> LfgPlayerDataContainer;
    typedef std::unordered_map<uint32, LFGDungeonData> LFGDungeonContainer;

    // Data needed by SMSG_LFG_JOIN_RESULT
    struct LfgJoinResultData
    {
        LfgJoinResultData(LfgJoinResult _result = LFG_JOIN_OK, LfgRoleCheckState _state = LFG_ROLECHECK_DEFAULT):
            result(_result), state(_state) {}
        LfgJoinResult result;
        LfgRoleCheckState state;
        LfgLockPartyMap lockmap;
    };

    // Data needed by SMSG_LFG_UPDATE_PARTY and SMSG_LFG_UPDATE_PLAYER
    struct LfgUpdateData
    {
        LfgUpdateData(LfgUpdateType _type = LFG_UPDATETYPE_DEFAULT): updateType(_type),  comment("") { }
        LfgUpdateData(LfgUpdateType _type, LfgDungeonSet  _dungeons, std::string  _comment):
            updateType(_type), state(LFG_STATE_NONE), dungeons(std::move(_dungeons)), comment(std::move(_comment)) { }
        LfgUpdateData(LfgUpdateType _type, LfgState _state, LfgDungeonSet  _dungeons, std::string  _comment = ""):
            updateType(_type), state(_state), dungeons(std::move(_dungeons)), comment(std::move(_comment)) { }

        LfgUpdateType updateType;
        LfgState state{LFG_STATE_NONE};
        LfgDungeonSet dungeons;
        std::string comment;
    };

    // Data needed by SMSG_LFG_QUEUE_STATUS
    struct LfgQueueStatusData
    {
        LfgQueueStatusData(uint32 _dungeonId = 0, int32 _waitTime = -1, int32 _waitTimeAvg = -1, int32 _waitTimeTank = -1, int32 _waitTimeHealer = -1,
                           int32 _waitTimeDps = -1, uint32 _queuedTime = 0, uint8 _tanks = 0, uint8 _healers = 0, uint8 _dps = 0) :
            dungeonId(_dungeonId), waitTime(_waitTime), waitTimeAvg(_waitTimeAvg), waitTimeTank(_waitTimeTank), waitTimeHealer(_waitTimeHealer),
            waitTimeDps(_waitTimeDps), queuedTime(_queuedTime), tanks(_tanks), healers(_healers), dps(_dps) {}

        uint32 dungeonId;
        int32 waitTime;
        int32 waitTimeAvg;
        int32 waitTimeTank;
        int32 waitTimeHealer;
        int32 waitTimeDps;
        uint32 queuedTime;
        uint8 tanks;
        uint8 healers;
        uint8 dps;
    };

    struct LfgPlayerRewardData
    {
        LfgPlayerRewardData(uint32 random, uint32 current, bool _done, Quest const* _quest):
            rdungeonEntry(random), sdungeonEntry(current), done(_done), quest(_quest) { }
        uint32 rdungeonEntry;
        uint32 sdungeonEntry;
        bool done;
        Quest const* quest;
    };

    // Reward info
    struct LfgReward
    {
        LfgReward(uint32 _maxLevel = 0, uint32 _firstQuest = 0, uint32 _otherQuest = 0):
            maxLevel(_maxLevel), firstQuest(_firstQuest), otherQuest(_otherQuest) { }

        uint32 maxLevel;
        uint32 firstQuest;
        uint32 otherQuest;
    };

    // LfgProposalPlayer — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgProposal — deleted in Inc-3 C2 (proposal machinery removed)
    // LfgRoleCheck — deleted in Inc-3 C2 (role-check machinery removed)
    // LfgPlayerBoot — deleted in Inc-3 C2 (boot vote machinery removed)

    struct LFGDungeonData
    {
        LFGDungeonData():  name("")
        { }
        LFGDungeonData(LFGDungeonEntry const* dbc) : id(dbc->ID), name(dbc->Name[0]), map(dbc->MapID),
            type(dbc->TypeID), expansion(uint8(dbc->ExpansionLevel)), group(uint8(dbc->GroupID)),
            minlevel(uint8(dbc->MinLevel)), maxlevel(uint8(dbc->MaxLevel)), difficulty(Difficulty(dbc->Difficulty)),
            seasonal((dbc->Flags & LFG_FLAG_SEASONAL) != 0), x(0.0f), y(0.0f), z(0.0f), o(0.0f)
        { }

        uint32 id{0};
        std::string name;
        uint16 map{0};
        uint8 type{0};
        uint8 expansion{0};
        uint8 group{0};
        uint8 minlevel{0};
        uint8 maxlevel{0};
        Difficulty difficulty{REGULAR_DIFFICULTY};
        bool seasonal{false};
        float x{0.0f}, y{0.0f}, z{0.0f}, o{0.0f};

        // Helpers
        [[nodiscard]] uint32 Entry() const { return id + (type << 24); }
    };

    class LFGMgr
    {
    private:
        LFGMgr();
        ~LFGMgr();
        // RB typedefs/stores (RBEntryInfoMap, RBStoreMap, RaidBrowserStore[2], RBSearchersMap,
        // RBSearchersStore[2], RBCacheMap, RBCacheStore[2], RBInternalInfoMap,
        // RBInternalInfoMapMap, RBInternalInfoStorePrev[2], RBInternalInfoStoreCurr[2],
        // RBUsedDungeonsSet, RBUsedDungeonsStore[2]) — deleted in Inc-3 C3 (Raid Browser retired)

    public:
        static LFGMgr* instance();

        // World.cpp
        /// Finish the dungeon for the given group. All check are performed using internal lfg data
        void FinishDungeon(ObjectGuid gguid, uint32 dungeonId, const Map* currMap);
        /// Loads rewards for random dungeons
        void LoadRewards();
        /// Loads dungeons from dbc and adds teleport coords
        void LoadLFGDungeons(bool reload = false);
        /// Filters out recently completed dungeons from the proposal set for the given players
        LfgDungeonSet FilterCooldownDungeons(LfgDungeonSet const& dungeons, LfgRolesMap const& players);
        /// Clears all dungeon cooldowns for all players
        void ClearDungeonCooldowns();

        // Multiple files
        /// Check if given guid applied for random dungeon
        bool selectedRandomLfgDungeon(ObjectGuid guid);
        /// Check if given guid applied for given map and difficulty. Used to know
        bool inLfgDungeonMap(ObjectGuid guid, uint32 map, Difficulty difficulty);
        /// Get selected dungeons
        LfgDungeonSet const& GetSelectedDungeons(ObjectGuid guid);
        /// Get current lfg state
        LfgState GetState(ObjectGuid guid);
        /// Get current dungeon
        uint32 GetDungeon(ObjectGuid guid, bool asId = true);
        /// Get the map id of the current dungeon
        uint32 GetDungeonMapId(ObjectGuid guid);
        /// Get kicks left in current group
        uint8 GetKicksLeft(ObjectGuid gguid);
        /// Load Lfg group info from DB
        void _LoadFromDB(Field* fields, ObjectGuid guid);
        /// Initializes player data after loading group data from DB
        void SetupGroupMember(ObjectGuid guid, ObjectGuid gguid);
        /// Return Lfg dungeon entry for given dungeon id
        uint32 GetLFGDungeonEntry(uint32 id);

        // cs_lfg
        /// Get current player roles
        uint8 GetRoles(ObjectGuid guid);
        /// Get current player comment (used for LFR)
        std::string const& GetComment(ObjectGuid gguid);
        /// Gets current lfg options
        uint32 GetOptions();
        /// Sets new lfg options
        void SetOptions(uint32 options);
        /// Checks if given lfg option is enabled
        bool isOptionEnabled(uint32 option);
        /// Clears queue - Only for internal testing
        void Clean();

        // LFGScripts
        /// Get leader of the group (using internal data)
        ObjectGuid GetLeader(ObjectGuid guid);
        /// Initializes locked dungeons for given player (called at login or level change)
        void InitializeLockedDungeons(Player* player, Group const* group = nullptr);
        /// Sets player team
        void SetTeam(ObjectGuid guid, TeamId teamId);
        /// Sets player group
        void SetGroup(ObjectGuid guid, ObjectGuid group);
        /// Gets player group
        ObjectGuid GetGroup(ObjectGuid guid);
        /// Sets the leader of the group
        void SetLeader(ObjectGuid gguid, ObjectGuid leader);
        /// Removes saved group data
        void RemoveGroupData(ObjectGuid guid);
        /// Removes a player from a group
        uint8 RemovePlayerFromGroup(ObjectGuid gguid, ObjectGuid guid);
        /// Adds player to group
        void AddPlayerToGroup(ObjectGuid gguid, ObjectGuid guid);
        /// Store player that selected random queue to group
        void AddPlayerQueuedForRandomDungeonToGroup(ObjectGuid gguid, ObjectGuid guid);
        bool IsPlayerQueuedForRandomDungeon(ObjectGuid guid);
        /// Xinef: Set Random Players Count
        void SetRandomPlayersCount(ObjectGuid guid, uint8 count);
        /// Xinef: Get Random Players Count
        uint8 GetRandomPlayersCount(ObjectGuid guid);

        // LFGHandler
        /// Get locked dungeons
        LfgLockMap const& GetLockedDungeons(ObjectGuid guid);
        /// Returns current lfg status
        LfgUpdateData GetLfgStatus(ObjectGuid guid);
        /// Checks if Seasonal dungeon is active
        bool IsSeasonActive(uint32 dungeonId);
        /// Checks if given dungeon map is disabled
        bool IsDungeonDisabled(uint32 mapId, Difficulty difficulty) const;
        /// Gets the random dungeon reward corresponding to given dungeon and player level
        LfgReward const* GetRandomDungeonReward(uint32 dungeon, uint8 level);
        /// Returns all random and seasonal dungeons for given level and expansion
        LfgDungeonSet GetRandomAndSeasonalDungeons(uint8 level, uint8 expansion);
        /// Teleport a player to/from selected dungeon
        void TeleportPlayer(Player* player, bool out, WorldLocation const* teleportLocation = nullptr);
        // InitBoot/UpdateBoot/UpdateProposal — deleted in Inc-3 C2
        /// Sets player lfg roles
        void SetRoles(ObjectGuid guid, uint8 roles);
        /// Sets player lfr comment
        void SetComment(ObjectGuid guid, std::string const& comment);
        /// Join Lfg with selected roles, dungeons and comment
        void JoinLfg(Player* player, uint8 roles, LfgDungeonSet& dungeons, std::string const& comment);
        /// Leaves lfg
        void LeaveLfg(ObjectGuid guid);
        /// pussywizard: cleans all queues' data
        void LeaveAllLfgQueues(ObjectGuid guid, bool allowgroup, ObjectGuid groupguid = ObjectGuid::Empty);
        // JoinRaidBrowser/LeaveRaidBrowser/LfrSearchAdd/LfrSearchRemove/SendRaidBrowserCachedList/
        // UpdateRaidBrowser/LfrSetComment/SendRaidBrowserJoinedPacket/RBPacket* —
        // deleted in Inc-3 C3 (Raid Browser retired; NOT LFR)

        // LfgQueue
        /// Get last lfg state (NONE, DUNGEON or FINISHED_DUNGEON)
        LfgState GetOldState(ObjectGuid guid);
        /// Check if given group guid is lfg
        bool IsLfgGroup(ObjectGuid guid);
        /// Gets the player count of given group
        uint8 GetPlayerCount(ObjectGuid guid);
        // AddProposal — deleted in Inc-3 C2
        /// Checks if given players are ignoring each other
        static bool HasIgnore(ObjectGuid guid1, ObjectGuid guid2);
        /// Sends queue status to player
        static void SendLfgQueueStatus(ObjectGuid guid, LfgQueueStatusData const& data);
        // debug lfg command
        void ToggleTesting();
        /// For 1 player queue testing
        [[nodiscard]] bool IsTesting() const { return m_Testing; }

        void SetDungeon(ObjectGuid guid, uint32 dungeon);
        LFGDungeonData const* GetLFGDungeon(uint32 id);
        /// Set group and all tracked members to LFG_STATE_DUNGEON.
        /// Called by harness lfg.form_group after force-creating a group.
        void InitGroupForDungeon(ObjectGuid gguid);

        /// Sets the LFG state for a player or group. Public so external consumers
        /// (LFGHandler, LfgVetoScript) can mirror state changes without going
        /// through the full LFGMgr::JoinLfg / LeaveLfg paths.
        void SetState(ObjectGuid guid, LfgState state);

    private:
        TeamId GetTeam(ObjectGuid guid);
        void RestoreState(ObjectGuid guid, char const* debugMsg);
        void ClearState(ObjectGuid guid, char const* debugMsg);
        void SetSelectedDungeons(ObjectGuid guid, LfgDungeonSet const& dungeons);
        void SetLockedDungeons(ObjectGuid guid, LfgLockMap const& lock);
        void DecreaseKicksLeft(ObjectGuid guid);
        // SetCanOverrideRBState — deleted in Inc-3 C3 (Raid Browser retired)
        void _SaveToDB(ObjectGuid guid);

        // Proposals
        // RemoveProposal — deleted in Inc-3 C2
        // MakeNewGroup — deleted in Inc-3 C2

        // Generic
        LfgDungeonSet const& GetDungeonsByRandom(uint32 randomdungeon);
        LfgType GetDungeonType(uint32 dungeon);

        // SendLfgBootProposalUpdate — deleted in Inc-3 C2
        void SendLfgJoinResult(ObjectGuid guid, LfgJoinResultData const& data);
        // SendLfgRoleChosen — deleted in Inc-3 C2
        // SendLfgRoleCheckUpdate — deleted in Inc-3 C2
        void SendLfgUpdateParty(ObjectGuid guid, LfgUpdateData const& data);
        void SendLfgUpdatePlayer(ObjectGuid guid, LfgUpdateData const& data);
        // SendLfgUpdateProposal — deleted in Inc-3 C2

        LfgGuidSet const& GetPlayers(ObjectGuid guid);

        // General variables
        // m_lfgProposalId — deleted in Inc-3 C2 (used only by AddProposal, now removed)
        // m_raidBrowserUpdateTimer[2] — deleted in Inc-3 C3 (Raid Browser retired)
        // m_raidBrowserLastUpdatedDungeonId[2] — deleted in Inc-3 C3
        uint32 m_options;                                  ///< Stores config options

        LfgCachedDungeonContainer CachedDungeonMapStore;   ///< Stores all dungeons by groupType
        // Reward System
        LfgRewardContainer RewardMapStore;                 ///< Stores rewards for random dungeons
        LFGDungeonContainer  LfgDungeonStore;
        // RoleChecksStore — deleted in Inc-3 C2 (role-check machinery removed)
        // ProposalsStore — deleted in Inc-3 C2 (proposal machinery removed)
        // BootsStore — deleted in Inc-3 C2 (boot vote machinery removed)
        LfgPlayerDataContainer PlayersStore;               ///< Player data
        LfgGroupDataContainer GroupsStore;                 ///< Group data
        bool m_Testing;

        // Dungeon cooldown system - prevents same dungeon being assigned in a row
        typedef std::unordered_map<uint32 /*dungeonId*/, TimePoint /*completionTime*/> LfgDungeonCooldownMap;
        typedef std::unordered_map<ObjectGuid /*playerGuid*/, LfgDungeonCooldownMap> LfgDungeonCooldownContainer;
        LfgDungeonCooldownContainer DungeonCooldownStore;  ///< Stores dungeon cooldowns per player
        void AddDungeonCooldown(ObjectGuid guid, uint32 dungeonId);
        void CleanupDungeonCooldowns();
        [[nodiscard]] Seconds GetDungeonCooldownDuration() const;
    };

    template <typename T, FMT_ENABLE_IF(std::is_enum_v<T>)>
    auto format_as(T f) { return fmt::underlying(f); }

} // namespace lfg

#define sLFGMgr lfg::LFGMgr::instance()

#endif
