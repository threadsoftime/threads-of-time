# SPDX-License-Identifier: GPL-2.0-or-later
import os
from pathlib import Path
from pydantic import BaseModel


class Settings(BaseModel):
    data_dir: Path
    embeddings_url: str
    embeddings_model: str
    embeddings_api_key: str = ""

    @classmethod
    def from_env(cls) -> "Settings":
        return cls(
            data_dir=Path(os.environ["MEMORY_DATA_DIR"]),
            embeddings_url=os.environ["BRAIN_EMBEDDINGS_URL"],
            embeddings_model=os.environ["BRAIN_EMBEDDINGS_MODEL"],
            embeddings_api_key=os.environ.get("BRAIN_EMBEDDINGS_API_KEY", ""),
        )
