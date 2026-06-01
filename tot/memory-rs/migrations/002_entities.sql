-- SPDX-License-Identifier: GPL-2.0-or-later
-- Migration: 002_entities.sql
-- Per design subspec §2 (Entity Schema).

CREATE TABLE entities (
    entity_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_kind     TEXT    NOT NULL
                    CHECK  (entity_kind IN (
                        'player',    -- human-controlled character (has game GUID)
                        'npc',       -- non-player character (has game GUID / name)
                        'item',      -- item template (has item_entry_id)
                        'location',  -- map zone or named place (has map_id + zone_id)
                        'faction'    -- in-game faction (has faction_id)
                    )),
    entity_key      TEXT    NOT NULL,  -- canonical key: low-GUID for player/npc, entry for item,
                                       -- "mapId:zoneId" for location, faction_id for faction
    display_name    TEXT    NOT NULL,  -- human-readable label
    summary         TEXT,              -- agent-maintained running summary (updated by PATCH)
    first_seen_at   INTEGER NOT NULL   -- unix epoch ms
                    DEFAULT (unixepoch('now') * 1000),
    last_seen_at    INTEGER,           -- unix epoch ms; updated on each episode link
    total_episodes  INTEGER NOT NULL DEFAULT 0,  -- denormalized count; updated on insert/delete
    metadata        TEXT,              -- JSON; e.g., {"class": "warrior", "level": 20}
    UNIQUE (entity_kind, entity_key)
);

CREATE INDEX ix_entities_kind_key        ON entities(entity_kind, entity_key);
CREATE INDEX ix_entities_display_name    ON entities(display_name);
CREATE INDEX ix_entities_last_seen       ON entities(last_seen_at DESC);

-- M:N join: which entities appear in which episodes
CREATE TABLE episode_entities (
    episode_id  INTEGER NOT NULL REFERENCES episodes(episode_id) ON DELETE CASCADE,
    entity_id   INTEGER NOT NULL REFERENCES entities(entity_id)  ON DELETE CASCADE,
    role        TEXT    NOT NULL DEFAULT 'participant'
                CHECK  (role IN (
                    'subject',      -- the entity the episode is primarily about
                    'participant',  -- present but not the main subject
                    'target',       -- entity on the receiving end of an action
                    'witness'       -- observed but not interacted with
                )),
    PRIMARY KEY (episode_id, entity_id, role)
);

CREATE INDEX ix_ee_episode    ON episode_entities(episode_id);
CREATE INDEX ix_ee_entity     ON episode_entities(entity_id);
CREATE INDEX ix_ee_entity_role ON episode_entities(entity_id, role);

-- Triggers to maintain entities.total_episodes (per §2.3)
CREATE TRIGGER ee_ai AFTER INSERT ON episode_entities BEGIN
    UPDATE entities SET total_episodes = total_episodes + 1
    WHERE entity_id = NEW.entity_id;
END;

CREATE TRIGGER ee_ad AFTER DELETE ON episode_entities BEGIN
    UPDATE entities SET total_episodes = MAX(0, total_episodes - 1)
    WHERE entity_id = OLD.entity_id;
END;
