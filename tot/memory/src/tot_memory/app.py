# SPDX-License-Identifier: GPL-2.0-or-later
from fastapi import FastAPI
from tot_memory.config import Settings
from tot_memory.routes import health, recall, write


def create_app() -> FastAPI:
    settings = Settings.from_env()
    settings.data_dir.mkdir(parents=True, exist_ok=True)

    app = FastAPI(title="ToT Memory", version="1.0.0-dev")
    app.state.settings = settings
    app.include_router(health.router)
    app.include_router(write.router)
    app.include_router(recall.router)
    return app
