# SPDX-License-Identifier: GPL-2.0-or-later
import pytest
from fastapi.testclient import TestClient
from tot_memory.app import create_app


@pytest.fixture
def client(tmp_path, monkeypatch):
    monkeypatch.setenv("MEMORY_DATA_DIR", str(tmp_path / "memory"))
    monkeypatch.setenv("BRAIN_EMBEDDINGS_URL", "http://stub.local/v1")
    monkeypatch.setenv("BRAIN_EMBEDDINGS_MODEL", "nomic-embed-text")
    app = create_app()
    return TestClient(app)
