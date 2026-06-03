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

#include "DBCStores.h"
#include "GameTime.h"
#include "Group.h"
#include "LFGMgr.h"
#include "LfgIntentStore.h"
#include "LFGPackets.h"
#include "ObjectMgr.h"
#include "Opcodes.h"
#include "Player.h"
#include "ScriptMgr.h"
#include "WorldPacket.h"
#include "WorldSession.h"

void BuildPlayerLockDungeonBlock(WorldPacket& data, lfg::LfgLockMap const& lock)
{
    data << uint32(lock.size());                           // Size of lock dungeons
    for (lfg::LfgLockMap::const_iterator it = lock.begin(); it != lock.end(); ++it)
    {
        data << uint32(it->first);                         // Dungeon entry (id + type)
        data << uint32(it->second);                        // Lock status
    }
}

void BuildPartyLockDungeonBlock(WorldPacket& data, const lfg::LfgLockPartyMap& lockMap)
{
    data << uint8(lockMap.size());
    for (lfg::LfgLockPartyMap::const_iterator it = lockMap.begin(); it != lockMap.end(); ++it)
    {
        data << it->first;                                 // Player guid
        BuildPlayerLockDungeonBlock(data, it->second);
    }
}

void WorldSession::HandleLfgJoinOpcode(WorldPackets::LFG::LFGJoin& packet)
{
    if (!sLFGMgr->isOptionEnabled(lfg::LFG_OPTION_ENABLE_DUNGEON_FINDER | lfg::LFG_OPTION_ENABLE_RAID_BROWSER | lfg::LFG_OPTION_ENABLE_SEASONAL_BOSSES) ||
        (GetPlayer()->GetGroup() && GetPlayer()->GetGroup()->GetLeaderGUID() != GetPlayer()->GetGUID() &&
         (GetPlayer()->GetGroup()->GetMembersCount() == MAXGROUPSIZE || !GetPlayer()->GetGroup()->isLFGGroup())))
        return;

    if (packet.Slots.empty())
    {
        LOG_DEBUG("lfg", "CMSG_LFG_JOIN {} no dungeons selected", GetPlayerInfo());
        return;
    }

    lfg::LfgDungeonSet newDungeons;
    for (uint32 slot : packet.Slots)
    {
        uint32 dungeon = slot & 0x00FFFFFF;                             // remove the type from the dungeon entry
        if (sLFGDungeonStore.LookupEntry(dungeon))
            newDungeons.insert(dungeon);
    }

    LOG_DEBUG("network", "CMSG_LFG_JOIN [{}] roles: {}, Dungeons: {}, Comment: {}",
                 GetPlayerInfo(), packet.Roles, newDungeons.size(), packet.Comment);

    sLFGMgr->JoinLfg(GetPlayer(), uint8(packet.Roles), newDungeons, packet.Comment);
    GetPlayer()->UpdateLFGChannel();
}

void WorldSession::HandleLfgLeaveOpcode(WorldPackets::LFG::LFGLeave& /*packet*/)
{
    Group* group = GetPlayer()->GetGroup();
    ObjectGuid guid = GetPlayer()->GetGUID();
    ObjectGuid gguid = group ? group->GetGUID() : guid;

    LOG_DEBUG("network", "CMSG_LFG_LEAVE [{}] in group: {}", guid.ToString(), group ? 1 : 0);

    // Cheating guard: only the leader (or ungrouped player) may leave the queue.
    if (!group || group->GetLeaderGUID() == guid)
    {
        lfg::LfgState const state = sLFGMgr->GetState(guid);

        if (state == lfg::LFG_STATE_QUEUED)
        {
            // Strangler-fig leave seam (Inc-2): enqueue a POD cancel intent for
            // the Rust slice to drain next tick (re-resolve Player* on the drain,
            // never here), send REMOVED_FROM_QUEUE so the client eye stops, and
            // clear local LFG state.
            //
            // Inc-4 deferral note: only the leader's cancel is recorded here, and
            // only this session receives REMOVED_FROM_QUEUE. For a premade of
            // multiple REAL players queued as a group the legacy LFGMgr path
            // would broadcast REMOVED_FROM_QUEUE to all members; that broadcast is
            // intentionally not replicated here because the Inc-2 slice fills SOLO
            // real players with bots — a multi-real-player premade is out of scope
            // until Inc-4. (The legacy per-member broadcast is available via the
            // LeaveLfg / LeaveAllLfgQueues path below if ever needed.)
            HarnessBridge::RecordLfgCancel(guid.GetCounter());

            lfg::LfgUpdateData removed(lfg::LFG_UPDATETYPE_REMOVED_FROM_QUEUE);
            SendLfgUpdatePlayer(removed);
            if (group)
                SendLfgUpdateParty(removed);

            sLFGMgr->SetState(guid, lfg::LFG_STATE_NONE);
        }
        else
        {
            // DUNGEON / FINISHED_DUNGEON / BOOT / RAIDBROWSER / NONE: preserve
            // legacy behavior (R6 — in-dungeon leave is a state-clear no-op).
            sLFGMgr->LeaveLfg(state == lfg::LFG_STATE_RAIDBROWSER ? guid : gguid);
            sLFGMgr->LeaveAllLfgQueues(guid, true, group ? group->GetGUID() : ObjectGuid::Empty);
        }
    }

    GetPlayer()->UpdateLFGChannel();
}

// HandleLfgProposalResultOpcode — deleted in Inc-3 C2 (proposal machinery removed)

// HandleLfgSetRolesOpcode — deleted in Inc-3 C2 (role-check machinery removed)

void WorldSession::HandleLfgSetCommentOpcode(WorldPacket&  recvData)
{
    std::string comment;
    recvData >> comment;
    ObjectGuid guid = GetPlayer()->GetGUID();
    LOG_DEBUG("network", "CMSG_LFG_SET_COMMENT [{}] comment: {}", guid.ToString(), comment);

    sLFGMgr->SetComment(GetPlayer()->GetGUID(), comment);
    sLFGMgr->LfrSetComment(GetPlayer(), comment);
}

// HandleLfgSetBootVoteOpcode — deleted in Inc-3 C2 (boot machinery removed)

void WorldSession::HandleLfgTeleportOpcode(WorldPacket& recvData)
{
    bool out;
    recvData >> out;

    LOG_DEBUG("network", "CMSG_LFG_TELEPORT [{}] out: {}", GetPlayer()->GetGUID().ToString(), out ? 1 : 0);
    sLFGMgr->TeleportPlayer(GetPlayer(), out);
}

void WorldSession::HandleLfgPlayerLockInfoRequestOpcode(WorldPacket& /*recvData*/)
{
    ObjectGuid guid = GetPlayer()->GetGUID();
    LOG_DEBUG("network", "CMSG_LFG_PLAYER_LOCK_INFO_REQUEST [{}]", guid.ToString());

    // Get Random dungeons that can be done at a certain level and expansion
    uint8 level = GetPlayer()->GetLevel();
    lfg::LfgDungeonSet const& randomDungeons =
        sLFGMgr->GetRandomAndSeasonalDungeons(level, GetPlayer()->GetSession()->Expansion());

    // Get player locked Dungeons
    sLFGMgr->InitializeLockedDungeons(GetPlayer(), GetPlayer()->GetGroup()); // pussywizard
    lfg::LfgLockMap const& lock = sLFGMgr->GetLockedDungeons(guid);
    uint32 rsize = uint32(randomDungeons.size());
    uint32 lsize = uint32(lock.size());

    LOG_DEBUG("network", "SMSG_LFG_PLAYER_INFO [{}]", guid.ToString());
    WorldPacket data(SMSG_LFG_PLAYER_INFO, 1 + rsize * (4 + 1 + 4 + 4 + 4 + 4 + 1 + 4 + 4 + 4) + 4 + lsize * (1 + 4 + 4 + 4 + 4 + 1 + 4 + 4 + 4));

    data << uint8(randomDungeons.size());                  // Random Dungeon count
    for (lfg::LfgDungeonSet::const_iterator it = randomDungeons.begin(); it != randomDungeons.end(); ++it)
    {
        data << uint32(*it);                               // Dungeon Entry (id + type)
        lfg::LfgReward const* reward = sLFGMgr->GetRandomDungeonReward(*it, level);
        Quest const* quest = nullptr;
        bool done = false;
        if (reward)
        {
            quest = sObjectMgr->GetQuestTemplate(reward->firstQuest);
            if (quest)
            {
                done = !GetPlayer()->CanRewardQuest(quest, false);
                if (done)
                    quest = sObjectMgr->GetQuestTemplate(reward->otherQuest);
            }
        }

        if (quest)
        {
            uint8 playerLevel = GetPlayer() ? GetPlayer()->GetLevel() : 0;
            uint8 playerLevelForXP = playerLevel;
            sScriptMgr->OnPlayerBeforeGetLevelForXPGain(GetPlayer(), playerLevelForXP);

            data << uint8(done);
            data << uint32(quest->GetRewOrReqMoney(playerLevel));
            if (playerLevelForXP < GetPlayer()->GetUInt32Value(PLAYER_FIELD_MAX_LEVEL))
                data << uint32(quest->XPValue(playerLevelForXP));
            else
                data << uint32(0);
            data << uint32(0);
            data << uint32(0);
            data << uint8(quest->GetRewItemsCount());
            if (quest->GetRewItemsCount())
            {
                for (uint8 i = 0; i < QUEST_REWARDS_COUNT; ++i)
                    if (uint32 itemId = quest->RewardItemId[i])
                    {
                        ItemTemplate const* item = sObjectMgr->GetItemTemplate(itemId);
                        data << uint32(itemId);
                        data << uint32(item ? item->DisplayInfoID : 0);
                        data << uint32(quest->RewardItemIdCount[i]);
                    }
            }
        }
        else
        {
            data << uint8(0);
            data << uint32(0);
            data << uint32(0);
            data << uint32(0);
            data << uint32(0);
            data << uint8(0);
        }
    }
    BuildPlayerLockDungeonBlock(data, lock);
    SendPacket(&data);
}

void WorldSession::HandleLfgPartyLockInfoRequestOpcode(WorldPacket&  /*recvData*/)
{
    ObjectGuid guid = GetPlayer()->GetGUID();
    LOG_DEBUG("network", "CMSG_LFG_PARTY_LOCK_INFO_REQUEST [{}]", guid.ToString());

    Group* group = GetPlayer()->GetGroup();
    if (!group)
        return;

    // Get the locked dungeons of the other party members
    lfg::LfgLockPartyMap lockMap;
    for (GroupReference* itr = group->GetFirstMember(); itr != nullptr; itr = itr->next())
    {
        Player* plrg = itr->GetSource();
        if (!plrg)
            continue;

        ObjectGuid pguid = plrg->GetGUID();
        if (pguid == guid)
            continue;

        sLFGMgr->InitializeLockedDungeons(plrg, group); // pussywizard
        lockMap[pguid] = sLFGMgr->GetLockedDungeons(pguid);
    }

    uint32 size = 0;
    for (lfg::LfgLockPartyMap::const_iterator it = lockMap.begin(); it != lockMap.end(); ++it)
        size += 8 + 4 + uint32(it->second.size()) * (4 + 4);

    LOG_DEBUG("network", "SMSG_LFG_PARTY_INFO [{}]", guid.ToString());
    WorldPacket data(SMSG_LFG_PARTY_INFO, 1 + size);
    BuildPartyLockDungeonBlock(data, lockMap);
    SendPacket(&data);
}

void WorldSession::HandleLfrSearchJoinOpcode(WorldPacket& recvData)
{
    uint32 dungeonId;
    recvData >> dungeonId;
    dungeonId = (dungeonId & 0x00FFFFFF); // remove the type from the dungeon entry
    sLFGMgr->LfrSearchAdd(GetPlayer(), dungeonId);
    sLFGMgr->SendRaidBrowserCachedList(GetPlayer(), dungeonId);
}

void WorldSession::HandleLfrSearchLeaveOpcode(WorldPacket& recvData)
{
    uint32 dungeonId;
    recvData >> dungeonId;
    sLFGMgr->LfrSearchRemove(GetPlayer());
}

void WorldSession::HandleLfgGetStatus(WorldPacket& /*recvData*/)
{
    LOG_DEBUG("lfg", "CMSG_LFG_GET_STATUS {}", GetPlayerInfo());

    ObjectGuid guid = GetPlayer()->GetGUID();
    lfg::LfgUpdateData updateData = sLFGMgr->GetLfgStatus(guid);

    if (GetPlayer()->GetGroup())
    {
        SendLfgUpdateParty(updateData);
        updateData.dungeons.clear();
        SendLfgUpdatePlayer(updateData);
    }
    else
    {
        SendLfgUpdatePlayer(updateData);
        updateData.dungeons.clear();
        SendLfgUpdateParty(updateData);
    }
}

void WorldSession::SendLfgUpdatePlayer(lfg::LfgUpdateData const& updateData)
{
    bool queued = false;
    uint8 size = uint8(updateData.dungeons.size());

    switch (updateData.updateType)
    {
        case lfg::LFG_UPDATETYPE_JOIN_QUEUE:
        case lfg::LFG_UPDATETYPE_ADDED_TO_QUEUE:
            queued = true;
            break;
        case lfg::LFG_UPDATETYPE_UPDATE_STATUS:
            queued = updateData.state == lfg::LFG_STATE_QUEUED;
            break;
        default:
            break;
    }

    LOG_DEBUG("lfg", "SMSG_LFG_UPDATE_PLAYER {} updatetype: {}",
                   GetPlayerInfo(), updateData.updateType);
    WorldPacket data(SMSG_LFG_UPDATE_PLAYER, 1 + 1 + (size > 0 ? 1 : 0) * (1 + 1 + 1 + 1 + size * 4 + updateData.comment.length()));
    data << uint8(updateData.updateType);                  // Lfg Update type
    data << uint8(size > 0);                               // Extra info
    if (size)
    {
        data << uint8(queued);                             // Join the queue
        data << uint8(0);                                  // unk - Always 0
        data << uint8(0);                                  // unk - Always 0

        data << uint8(size);
        for (lfg::LfgDungeonSet::const_iterator it = updateData.dungeons.begin(); it != updateData.dungeons.end(); ++it)
            data << uint32(*it);
        data << updateData.comment;
    }
    SendPacket(&data);
}

void WorldSession::SendLfgUpdateParty(lfg::LfgUpdateData const& updateData)
{
    bool join = false;
    bool queued = false;
    uint8 size = uint8(updateData.dungeons.size());

    switch (updateData.updateType)
    {
        case lfg::LFG_UPDATETYPE_ADDED_TO_QUEUE:                // Rolecheck Success
            queued = true;
            [[fallthrough]];
        case lfg::LFG_UPDATETYPE_PROPOSAL_BEGIN:
            join = true;
            break;
        case lfg::LFG_UPDATETYPE_UPDATE_STATUS:
            join = updateData.state != lfg::LFG_STATE_ROLECHECK && updateData.state != lfg::LFG_STATE_NONE;
            queued = updateData.state == lfg::LFG_STATE_QUEUED;
            break;
        default:
            break;
    }

    LOG_DEBUG("lfg", "SMSG_LFG_UPDATE_PARTY {} updatetype: {}",
                   GetPlayerInfo(), updateData.updateType);
    WorldPacket data(SMSG_LFG_UPDATE_PARTY, 1 + 1 + (size > 0 ? 1 : 0) * (1 + 1 + 1 + 1 + 1 + size * 4 + updateData.comment.length()));
    data << uint8(updateData.updateType);                  // Lfg Update type
    data << uint8(size > 0);                               // Extra info
    if (size)
    {
        data << uint8(join);                               // LFG Join
        data << uint8(queued);                             // Join the queue
        data << uint8(0);                                  // unk - Always 0
        data << uint8(0);                                  // unk - Always 0
        for (uint8 i = 0; i < 3; ++i)
            data << uint8(0);                              // unk - Always 0

        data << uint8(size);
        for (lfg::LfgDungeonSet::const_iterator it = updateData.dungeons.begin(); it != updateData.dungeons.end(); ++it)
            data << uint32(*it);
        data << updateData.comment;
    }
    SendPacket(&data);
}

// SendLfgRoleChosen — deleted in Inc-3 C2 (role-check machinery removed)
// SendLfgRoleCheckUpdate — deleted in Inc-3 C2 (role-check machinery removed)

void WorldSession::SendLfgJoinResult(lfg::LfgJoinResultData const& joinData)
{
    uint32 size = 0;
    for (lfg::LfgLockPartyMap::const_iterator it = joinData.lockmap.begin(); it != joinData.lockmap.end(); ++it)
        size += 8 + 4 + uint32(it->second.size()) * (4 + 4);

    LOG_DEBUG("network", "SMSG_LFG_JOIN_RESULT [{}] checkResult: {} checkValue: {}", GetPlayer()->GetGUID().ToString(), joinData.result, joinData.state);
    WorldPacket data(SMSG_LFG_JOIN_RESULT, 4 + 4 + size);
    data << uint32(joinData.result);                       // Check Result
    data << uint32(joinData.state);                        // Check Value
    if (!joinData.lockmap.empty())
        BuildPartyLockDungeonBlock(data, joinData.lockmap);
    SendPacket(&data);
}

void WorldSession::SendLfgQueueStatus(lfg::LfgQueueStatusData const& queueData)
{
    LOG_DEBUG("network", "SMSG_LFG_QUEUE_STATUS [{}] dungeon: {} - waitTime: {} - avgWaitTime: {} - waitTimeTanks: {} - waitTimeHealer: {} - waitTimeDps: {} - queuedTime: {} - tanks: {} - healers: {} - dps: {}",
                   GetPlayer()->GetGUID().ToString(), queueData.dungeonId, queueData.waitTime, queueData.waitTimeAvg, queueData.waitTimeTank,
                   queueData.waitTimeHealer, queueData.waitTimeDps, queueData.queuedTime, queueData.tanks, queueData.healers, queueData.dps);
    WorldPacket data(SMSG_LFG_QUEUE_STATUS, 4 + 4 + 4 + 4 + 4 + 4 + 1 + 1 + 1 + 4);
    data << uint32(queueData.dungeonId);                   // Dungeon
    data << int32(queueData.waitTimeAvg);                  // Average Wait time
    data << int32(queueData.waitTime);                     // Wait Time
    data << int32(queueData.waitTimeTank);                 // Wait Tanks
    data << int32(queueData.waitTimeHealer);               // Wait Healers
    data << int32(queueData.waitTimeDps);                  // Wait Dps
    data << uint8(queueData.tanks);                        // Tanks needed
    data << uint8(queueData.healers);                      // Healers needed
    data << uint8(queueData.dps);                          // Dps needed
    data << uint32(queueData.queuedTime);                  // Player wait time in queue
    SendPacket(&data);
}

void WorldSession::SendLfgPlayerReward(lfg::LfgPlayerRewardData const& rewardData)
{
    if (!rewardData.rdungeonEntry || !rewardData.sdungeonEntry || !rewardData.quest)
        return;

    LOG_DEBUG("lfg", "SMSG_LFG_PLAYER_REWARD {} rdungeonEntry: {}, sdungeonEntry: {}, done: {}",
                   GetPlayerInfo(), rewardData.rdungeonEntry, rewardData.sdungeonEntry, rewardData.done);

    uint8 itemNum = rewardData.quest->GetRewItemsCount();

    uint8 playerLevel = GetPlayer() ? GetPlayer()->GetLevel() : 0;
    uint8 playerLevelForXP = playerLevel;
    sScriptMgr->OnPlayerBeforeGetLevelForXPGain(GetPlayer(), playerLevelForXP);

    WorldPacket data(SMSG_LFG_PLAYER_REWARD, 4 + 4 + 1 + 4 + 4 + 4 + 4 + 4 + 1 + itemNum * (4 + 4 + 4));
    data << uint32(rewardData.rdungeonEntry);              // Random Dungeon Finished
    data << uint32(rewardData.sdungeonEntry);              // Dungeon Finished
    data << uint8(rewardData.done);
    data << uint32(1);
    data << uint32(rewardData.quest->GetRewOrReqMoney(playerLevel));
    data << uint32(rewardData.quest->XPValue(playerLevelForXP));
    data << uint32(0);
    data << uint32(0);
    data << uint8(itemNum);
    if (itemNum)
    {
        for (uint8 i = 0; i < QUEST_REWARDS_COUNT; ++i)
            if (uint32 itemId = rewardData.quest->RewardItemId[i])
            {
                ItemTemplate const* item = sObjectMgr->GetItemTemplate(itemId);
                data << uint32(itemId);
                data << uint32(item ? item->DisplayInfoID : 0);
                data << uint32(rewardData.quest->RewardItemIdCount[i]);
            }
    }
    SendPacket(&data);
}

// SendLfgBootProposalUpdate — deleted in Inc-3 C2 (boot machinery removed)
// SendLfgUpdateProposal — deleted in Inc-3 C2 (proposal machinery removed)

void WorldSession::SendLfgLfrList(bool update)
{
    LOG_DEBUG("network", "SMSG_LFG_LFR_LIST [{}] update: {}", GetPlayer()->GetGUID().ToString(), update ? 1 : 0);
    WorldPacket data(SMSG_LFG_UPDATE_SEARCH, 1);
    data << uint8(update);                                 // In Lfg Queue?
    SendPacket(&data);
}

void WorldSession::SendLfgDisabled()
{
    LOG_DEBUG("network", "SMSG_LFG_DISABLED [{}]", GetPlayer()->GetGUID().ToString());
    WorldPacket data(SMSG_LFG_DISABLED, 0);
    SendPacket(&data);
}

void WorldSession::SendLfgOfferContinue(uint32 dungeonEntry)
{
    LOG_DEBUG("network", "SMSG_LFG_OFFER_CONTINUE [{}] dungeon entry: {}", GetPlayer()->GetGUID().ToString(), dungeonEntry);
    WorldPacket data(SMSG_LFG_OFFER_CONTINUE, 4);
    data << uint32(dungeonEntry);
    SendPacket(&data);
}

void WorldSession::SendLfgTeleportError(uint8 err)
{
    LOG_DEBUG("network", "SMSG_LFG_TELEPORT_DENIED [{}] reason: {}", GetPlayer()->GetGUID().ToString(), err);
    WorldPacket data(SMSG_LFG_TELEPORT_DENIED, 4);
    data << uint32(err);                                   // Error
    SendPacket(&data);
}
