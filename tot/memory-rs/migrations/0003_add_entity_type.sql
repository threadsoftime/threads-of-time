-- v0.2: entities.type was already present in v0.1 baseline as a
-- nullable column (db.py:21). This migration is a marker only —
-- it sets schema_version=3 confirming v0.2's expected schema floor.
-- helpers.upsert_entity() picks up a type_hint param to populate
-- new rows; existing 87K NULL rows stay NULL until a separate
-- backfill script post-deploy.
SELECT 1;
