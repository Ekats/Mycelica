"""
Simple file watcher that triggers server reload.
Run this alongside uvicorn to force reloads on file changes.
"""
import time
import os
import sys
from pathlib import Path

WATCH_DIR = Path(__file__).parent
WATCH_EXTENSIONS = {'.py'}

def get_mtimes():
    """Get modification times for all watched files."""
    mtimes = {}
    for ext in WATCH_EXTENSIONS:
        for f in WATCH_DIR.glob(f'*{ext}'):
            mtimes[f] = f.stat().st_mtime
    return mtimes

def watch():
    """Watch for file changes and exit when detected (uvicorn will restart)."""
    print("Watching for file changes...")
    last_mtimes = get_mtimes()

    while True:
        time.sleep(1)
        current_mtimes = get_mtimes()

        if current_mtimes != last_mtimes:
            changed = set(current_mtimes.keys()) ^ set(last_mtimes.keys())
            for f in current_mtimes:
                if f in last_mtimes and current_mtimes[f] != last_mtimes[f]:
                    changed.add(f)

            print(f"Files changed: {[f.name for f in changed]}")
            print("Triggering reload...")
            sys.exit(3)  # Exit code 3 tells uvicorn to reload

        last_mtimes = current_mtimes

if __name__ == '__main__':
    watch()
