# SPDX-License-Identifier: GPL-2.0-or-later
"""Schema constants.

The single source of truth for the embedding dimension. The value here MUST
match the ``float[N]`` declaration in
``tot_memory/db/migrations/003_embeddings_vec.sql`` and the model named in
``BRAIN_EMBEDDINGS_MODEL`` per design subspec §3 + §4.

Default: 768, matching ``nomic-embed-text`` (the model documented in the
parent spec §6.3 and the ToT operator ``.env.example``). Changing this
constant requires updating the SQL migration AND rebuilding any existing
per-bot DBs (the dimension is baked into the sqlite-vec table at creation
time; ALTER VIRTUAL TABLE is not supported).
"""

EMBEDDING_DIM: int = 768  # nomic-embed-text; see design subspec §3 + §4
