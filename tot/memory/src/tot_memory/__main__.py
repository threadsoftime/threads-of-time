# SPDX-License-Identifier: GPL-2.0-or-later
import uvicorn


def main():
    uvicorn.run("tot_memory.app:create_app", factory=True, host="0.0.0.0", port=8090)


if __name__ == "__main__":
    main()
