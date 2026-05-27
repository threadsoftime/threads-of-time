-- sql/bracket1/wipe_bots.sql
-- Delete all bot characters so mod-playerbots regenerates them at level 1.
-- Idempotent: subsequent runs find no bot characters and become no-ops.
--
-- Bot account prefix discovered from live acore_auth.account: RNDBOT (e.g. RNDBOT1, RNDBOT2, ...)
-- Run this with worldserver stopped. Restart worldserver after to trigger bot regeneration.

-- Collect all bot character guids.
-- Note: regular (non-TEMPORARY) MEMORY table because MySQL forbids re-opening
-- a TEMPORARY table multiple times in the same connection's statement chain.
-- Each DELETE below references _bot_guids — that's incompatible with TEMPORARY.
DROP TABLE IF EXISTS acore_characters._bot_guids;
CREATE TABLE acore_characters._bot_guids (guid INT UNSIGNED PRIMARY KEY) ENGINE=MEMORY;
INSERT INTO acore_characters._bot_guids (guid)
  SELECT c.guid
    FROM acore_characters.characters c
    JOIN acore_auth.account a ON c.account = a.id
    WHERE a.username LIKE 'RNDBOT%';

-- -----------------------------------------------------------------------
-- Pet sub-tables: must delete pet_aura/pet_spell before character_pet,
-- as pet_aura/pet_spell reference character_pet.id (not character guid).
-- -----------------------------------------------------------------------
DELETE pa FROM acore_characters.pet_aura pa
  JOIN acore_characters.character_pet cp ON pa.guid = cp.id
  WHERE cp.owner IN (SELECT guid FROM acore_characters._bot_guids);

DELETE ps FROM acore_characters.pet_spell ps
  JOIN acore_characters.character_pet cp ON ps.guid = cp.id
  WHERE cp.owner IN (SELECT guid FROM acore_characters._bot_guids);

DELETE psc FROM acore_characters.pet_spell_cooldown psc
  JOIN acore_characters.character_pet cp ON psc.guid = cp.id
  WHERE cp.owner IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Mail items: delete before mail and item_instance
-- -----------------------------------------------------------------------
DELETE mi FROM acore_characters.mail_items mi
  JOIN acore_characters.mail m ON mi.mail_id = m.id
  WHERE m.sender IN (SELECT guid FROM acore_characters._bot_guids)
     OR m.receiver IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Items in inventory/bank: delete item_instance rows owned by bots.
-- character_inventory references item_instance; delete child first.
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.character_inventory    WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.item_instance          WHERE owner_guid IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Group and arena cleanup
-- -----------------------------------------------------------------------
-- Remove bots from group membership
DELETE FROM acore_characters.group_member           WHERE memberGuid IN (SELECT guid FROM acore_characters._bot_guids);
-- Remove any groups whose leader is a bot (orphan groups)
DELETE FROM acore_characters.groups
  WHERE leaderGuid IN (SELECT guid FROM acore_characters._bot_guids);

-- Remove bots from arena teams
DELETE FROM acore_characters.arena_team_member      WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Guild cleanup
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.guild_member           WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.guild_member_withdraw  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Mail
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.mail                   WHERE sender   IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.mail                   WHERE receiver IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- All guid-keyed character corollary tables
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.character_account_data            WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_achievement             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_achievement_offline_updates WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_achievement_progress    WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_action                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_arena_stats             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_aura                    WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_banned                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_battleground_random     WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_brew_of_the_month       WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_declinedname            WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_entry_point             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_equipmentsets           WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_gifts                   WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_glyphs                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_homebind                WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_instance                WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_pet                     WHERE owner IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_pet_declinedname        WHERE owner IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus_daily       WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus_monthly     WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus_rewarded    WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus_seasonal    WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_queststatus_weekly      WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_reputation              WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_settings                WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_skills                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_social                  WHERE guid   IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_social                  WHERE friend IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_spell                   WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_spell_cooldown          WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_stats                   WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.character_talent                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Misc tables with guid referencing a character
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.arena_team_member                 WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.battleground_deserters            WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.calendar_invites                  WHERE sender IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.corpse                            WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.creature_respawn                  WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.gameobject_respawn                WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.gm_survey                         WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.instance_saved_go_state_data      WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.lag_reports                       WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.lfg_data                          WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.log_arena_memberstats             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.mail_server_character             WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.pvpstats_players                  WHERE character_guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.quest_tracker                     WHERE character_guid IN (SELECT guid FROM acore_characters._bot_guids);
DELETE FROM acore_characters.recovery_item                     WHERE Guid IN (SELECT guid FROM acore_characters._bot_guids);

-- -----------------------------------------------------------------------
-- Finally, the characters themselves
-- -----------------------------------------------------------------------
DELETE FROM acore_characters.characters WHERE guid IN (SELECT guid FROM acore_characters._bot_guids);

DROP TABLE acore_characters._bot_guids;
