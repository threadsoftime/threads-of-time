# SPDX-License-Identifier: GPL-2.0-or-later
from fastapi import APIRouter

router = APIRouter()


@router.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}
