--
-- Phase 1.1 -- BFD raid 3-day rolling lockout seed
--
-- The AC core patch 0003-bfd-raid-3day-lockout.patch makes the engine
-- compute "next reset" using a 3-day period for map 48. That patch
-- works on REGENERATION -- on each reset boundary the engine advances
-- the stored resettime by +3 days. It doesn't seed the first row.
--
-- This SQL seeds it: insert (or refresh) the row in instance_reset
-- for map 48 difficulty 0 (normal-10 raid difficulty) with resettime
-- set to NOW + 3 days. The engine reads this at next startup (or on
-- demand via SetResetTimeFor) and uses it as the first reset boundary.
--
-- The same SQL can be re-run safely; the ON DUPLICATE KEY UPDATE keeps
-- it idempotent. Operators can use this to "reset the reset clock"
-- mid-cycle if something has gone wrong.
--
-- Note: this lives under tot_characters, not tot_world. The
-- instance_reset table is per-realm character state.
--

INSERT INTO `instance_reset` (`mapid`, `difficulty`, `resettime`)
VALUES (48, 0, UNIX_TIMESTAMP() + 259200)
ON DUPLICATE KEY UPDATE `resettime` = UNIX_TIMESTAMP() + 259200;
