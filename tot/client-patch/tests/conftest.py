# SPDX-License-Identifier: GPL-2.0-or-later
import sys
from pathlib import Path

# parents[0]=tests, parents[1]=client-patch, parents[2]=tot, parents[3]=repo-root
ROOT = Path(__file__).resolve().parents[3]  # threads-of-time/
LIB = ROOT / "tot" / "client-patch" / "lib" / "dbc_compositor" / "src"
sys.path.insert(0, str(LIB))
