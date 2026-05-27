-- sql/bracket1/world_dk_stats_stub.sql
-- AzerothCore's Player Create Level Stats validator iterates all class+level
-- combos from StartPlayerLevel(1) to MaxPlayerLevel(N) and aborts if any are
-- missing. Death Knight is a heroic class with stats data only from level 55+.
-- When MaxPlayerLevel=25 (Bracket 1), the validator finds DK at level 1 has
-- no stats and refuses to boot the worldserver.
--
-- Fix: inject fake DK stats rows for levels 1-54 by copying Warrior's stats.
-- DK creation is disabled via CharacterCreating.Disabled.ClassMask=32 in
-- worldserver.conf so these stats are never used in practice — they exist
-- purely to satisfy the boot-time validator.

INSERT IGNORE INTO acore_world.player_class_stats
    (Class, Level, BaseHP, BaseMana, Strength, Agility, Stamina, Intellect, Spirit)
SELECT 6, Level, BaseHP, BaseMana, Strength, Agility, Stamina, Intellect, Spirit
FROM acore_world.player_class_stats
WHERE Class = 1 AND Level <= 54;
