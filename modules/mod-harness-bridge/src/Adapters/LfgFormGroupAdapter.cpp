// LfgFormGroupAdapter — lfg.form_group
//
// Faithful replica of LFGMgr::MakeNewGroup for the strangler-fig LFG slice
// (Increment 1). The Rust matchmaker calls this to force-create a real
// server-side LFG group and teleport all members into the dungeon, bypassing
// the native LFGMgr proposal workflow entirely.
//
// C++ adapter canonical field set (must match LfgFormGroupArgs in tool_schemas.py):
//   args.contains("leader_guid")  — required, u64 low GUID
//   args.contains("members")      — required, JSON array of {guid: u64, roles: u32}
//   args.contains("dungeon_id")   — required, u32 LFGDungeons.dbc id
//
// Pre-create guards (any failure aborts before new Group() — no partial formation):
//   - leader_guid must appear in members[]
//   - members[] size must be 1..5
//   - each member: online, is a Player, not already grouped, not in a BG,
//     not being teleported, no deserter aura (LFG_SPELL_DUNGEON_DESERTER=71041)
//   - dungeon_id must resolve via sLFGMgr->GetLFGDungeon()
//
// Group creation sequence:
//   new Group() -> ConvertToLFG() -> Create(leader) [crashfix bail if false]
//   -> sGroupMgr->AddGroup() -> AddMember each non-leader -> SetLfgRoles each
//   -> SetDungeonDifficulty -> SetDungeon(Entry()) [public sLFGMgr->SetDungeon]
//   -> GROUP_FOUND + REMOVED_FROM_QUEUE per session
//   -> TeleportPlayer each (public sLFGMgr->TeleportPlayer, reuses all AC
//      instance-bind reconciliation and homebind-eject safety)
//
// Notes on LFGMgr private members:
//   - SetState: private, skipped. TeleportPlayer only needs group->isLFGGroup()
//     + GetDungeon(gguid), both set by ConvertToLFG() and public SetDungeon().
//   - _SaveToDB: private, skipped. Group is ephemeral until next crash recovery;
//     acceptable for Inc-1 live-proof scope (Stage-1 task).
//   - SendLfgUpdatePlayer/Party on sLFGMgr: private, but WorldSession exposes
//     the same methods directly — called via p->GetSession()->Send*.
//
// Teleport errors fold into a note; the group is already formed so placement
// degradation is not a hard failure.
//
// Runs on the world tick drain (OnTickDrain) — safe to touch Player*/Group*.

#include "Adapters/LfgFormGroupAdapter.h"

#include "Group.h"
#include "GroupMgr.h"
#include "LFGMgr.h"
#include "ObjectAccessor.h"
#include "Player.h"
#include "WorldSession.h"

namespace HarnessBridge::Adapters
{
    DispatchResult LfgFormGroup(nlohmann::json const& args)
    {
        DispatchResult res;

        // ── Arg-gate: top-level required fields ───────────────────────────
        if (!args.contains("leader_guid") || !args["leader_guid"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: leader_guid (int) required";
            return res;
        }
        if (!args.contains("members") || !args["members"].is_array())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: members (array) required";
            return res;
        }
        if (!args.contains("dungeon_id") || !args["dungeon_id"].is_number_integer())
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: dungeon_id (int) required";
            return res;
        }

        uint64_t leader_low = args["leader_guid"].get<uint64_t>();
        uint32_t dungeon_id = args["dungeon_id"].get<uint32_t>();
        auto const& members_json = args["members"];

        // ── Size guard ────────────────────────────────────────────────────
        std::size_t member_count = members_json.size();
        if (member_count < 1 || member_count > 5)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: members must have 1..5 entries, got "
                + std::to_string(member_count);
            return res;
        }

        // ── Parse members array ───────────────────────────────────────────
        struct MemberSpec
        {
            uint64_t low;
            uint8_t  roles;   // SetLfgRoles takes uint8
        };
        std::vector<MemberSpec> specs;
        specs.reserve(member_count);

        bool leader_in_members = false;
        for (auto const& m : members_json)
        {
            if (!m.contains("guid") || !m["guid"].is_number_integer()
             || !m.contains("roles") || !m["roles"].is_number_integer())
            {
                res.outcome = DispatchResult::Outcome::BadArgs;
                res.error_message = "lfg.form_group: each member must have {guid, roles}";
                return res;
            }
            uint64_t low   = m["guid"].get<uint64_t>();
            uint8_t  roles = static_cast<uint8_t>(m["roles"].get<uint32_t>());
            specs.push_back({low, roles});
            if (low == leader_low)
                leader_in_members = true;
        }

        if (!leader_in_members)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: leader_guid must appear in members[]";
            return res;
        }

        // ── Dungeon existence guard ───────────────────────────────────────
        lfg::LFGDungeonData const* dungeon = sLFGMgr->GetLFGDungeon(dungeon_id);
        if (!dungeon)
        {
            res.outcome = DispatchResult::Outcome::BadArgs;
            res.error_message = "lfg.form_group: dungeon_id "
                + std::to_string(dungeon_id) + " not found in LFGDungeons.dbc";
            return res;
        }

        // ── Pre-validate ALL members before creating the group ────────────
        // Any single failure aborts; no partial formation.
        std::vector<Player*> players;
        players.reserve(member_count);

        for (auto const& spec : specs)
        {
            Player* p = ObjectAccessor::FindPlayer(
                ObjectGuid::Create<HighGuid::Player>(spec.low));
            if (!p)
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "lfg.form_group: guid "
                    + std::to_string(spec.low) + " not online";
                return res;
            }
            if (p->GetGroup() != nullptr)
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "lfg.form_group: guid "
                    + std::to_string(spec.low) + " is already grouped";
                return res;
            }
            if (p->InBattleground())
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "lfg.form_group: guid "
                    + std::to_string(spec.low) + " is in a battleground";
                return res;
            }
            if (p->IsBeingTeleported())
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "lfg.form_group: guid "
                    + std::to_string(spec.low) + " is being teleported";
                return res;
            }
            if (p->HasAura(lfg::LFG_SPELL_DUNGEON_DESERTER))
            {
                res.outcome = DispatchResult::Outcome::ExecutorFailed;
                res.error_message = "lfg.form_group: guid "
                    + std::to_string(spec.low) + " has deserter aura";
                return res;
            }
            players.push_back(p);
        }

        // Identify leader Player* for Group::Create
        Player* leader_player = nullptr;
        for (std::size_t i = 0; i < specs.size(); ++i)
        {
            if (specs[i].low == leader_low)
            {
                leader_player = players[i];
                break;
            }
        }
        // Defensive: leader_in_members + online guards above guarantee this.
        if (!leader_player)
        {
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "lfg.form_group: leader_player resolution failed (internal)";
            return res;
        }

        // ── Create the group ──────────────────────────────────────────────
        // Mirror LFGMgr::MakeNewGroup lines 1779-1795 for the fresh-group path.
        Group* grp = new Group();
        grp->ConvertToLFG();    // GROUPTYPE_LFG | GROUPTYPE_LFG_RESTRICTED, loot=NEED_BEFORE_GREED
        if (!grp->Create(leader_player))
        {
            // pussywizard crashfix: if Create() fails, discard and bail cleanly.
            // Mirrors the guard at LFGMgr.cpp:1797.
            delete grp;
            res.outcome = DispatchResult::Outcome::ExecutorFailed;
            res.error_message = "lfg.form_group: Group::Create() failed for leader "
                + std::to_string(leader_low);
            return res;
        }
        sGroupMgr->AddGroup(grp);

        // ── Add non-leader members ────────────────────────────────────────
        for (std::size_t i = 0; i < specs.size(); ++i)
        {
            if (specs[i].low == leader_low)
                continue;   // leader is already the group creator via Create()
            Player* p = players[i];
            if (!grp->IsFull())
                grp->AddMember(p);
        }

        // ── Set LFG roles for every member ────────────────────────────────
        // SetLfgRoles also calls SendUpdate() which dispatches SMSG_GROUP_LIST,
        // populating the client's party frame.
        for (std::size_t i = 0; i < specs.size(); ++i)
        {
            ObjectGuid pguid = ObjectGuid::Create<HighGuid::Player>(specs[i].low);
            grp->SetLfgRoles(pguid, specs[i].roles);
        }

        // ── Dungeon state ─────────────────────────────────────────────────
        // SetDungeonDifficulty + SetDungeon are required so TeleportPlayer can
        // resolve the dungeon via GetLFGDungeon(GetDungeon(group->GetGUID())).
        grp->SetDungeonDifficulty(Difficulty(dungeon->difficulty));
        ObjectGuid gguid = grp->GetGUID();
        sLFGMgr->SetDungeon(gguid, dungeon->Entry());   // Entry() = id + (type<<24)

        // ── Send GROUP_FOUND + REMOVED_FROM_QUEUE ─────────────────────────
        // Mirror LFGMgr::UpdateProposal lines 1963-1985.
        // sLFGMgr->SendLfgUpdatePlayer/Party are private, but WorldSession
        // exposes the identical methods directly.
        lfg::LfgUpdateData groupFound(lfg::LFG_UPDATETYPE_GROUP_FOUND);
        lfg::LfgUpdateData removedFromQueue(lfg::LFG_UPDATETYPE_REMOVED_FROM_QUEUE);
        for (std::size_t i = 0; i < players.size(); ++i)
        {
            if (WorldSession* sess = players[i]->GetSession())
            {
                sess->SendLfgUpdatePlayer(groupFound);
                sess->SendLfgUpdatePlayer(removedFromQueue);
                sess->SendLfgUpdateParty(removedFromQueue);
            }
        }

        // ── Teleport all members into the dungeon ─────────────────────────
        // Reuse LFGMgr::TeleportPlayer(player, out=false, teleportLocation=nullptr).
        // That function handles instance-bind reconciliation, entrypoint save,
        // and SendLfgTeleportError on placement failure. Post-create, so a
        // teleport error is a degraded placement — fold into result note.
        std::vector<uint64_t> placed;
        placed.reserve(players.size());
        for (GroupReference* itr = grp->GetFirstMember(); itr != nullptr; itr = itr->next())
        {
            if (Player* p = itr->GetSource())
            {
                sLFGMgr->TeleportPlayer(p, false, nullptr);
                placed.push_back(p->GetGUID().GetCounter());
            }
        }

        // ── Build result JSON ─────────────────────────────────────────────
        nlohmann::json placed_arr = nlohmann::json::array();
        for (uint64_t g : placed)
            placed_arr.push_back(g);

        res.outcome = DispatchResult::Outcome::Ok;
        res.result_json = {
            {"formed",      true},
            {"group_id",    gguid.GetCounter()},
            {"leader_guid", leader_low},
            {"placed",      placed_arr},
            {"map_id",      dungeon->map},
        };
        return res;
    }
}
