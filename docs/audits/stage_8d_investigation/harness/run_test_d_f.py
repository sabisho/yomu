#!/usr/bin/env python3
"""
Test D (Archive Comparison) & Test F (Cover Cache Isolation)
Stage 8D - Comprehensive Memory Retention & Leak Investigation
"""

import os
import sys
import time
import subprocess
import json
import shutil
import threading
from monitor import monitor_loop

HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))
STAGE8D_DIR = os.path.dirname(HARNESS_DIR)
RAW_DATA_DIR = os.path.join(STAGE8D_DIR, "raw_data")
YOMU_BIN = os.path.abspath("target/release/yomu")

os.makedirs(RAW_DATA_DIR, exist_ok=True)

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid():
    res = run_cmd("pgrep -f 'target/release/yomu'", check=False)
    for p in res.stdout.strip().split():
        if p.isdigit():
            return int(p)
    return None

def launch_yomu():
    proc = subprocess.Popen(["devenv", "shell", YOMU_BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(30):
        time.sleep(0.5)
        pid = get_yomu_pid()
        if pid:
            return proc, pid
    raise RuntimeError("Failed to launch yomu")

def focus_and_warp():
    run_cmd("umbriel msg window-focus-warp yomu", check=False)
    run_cmd("nix-shell -p wlrctl --run 'wlrctl toplevel focus app_id:yomu'", check=False)
    time.sleep(0.5)

def quit_yomu(proc):
    run_cmd("nix-shell -p wtype --run 'wtype -M ctrl -k q -m ctrl'", check=False)
    time.sleep(1)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except Exception:
            proc.kill()
    time.sleep(1)

def run_test_f_cover_cache():
    """Test F: Cover Cache Isolation."""
    print("\n--- Starting Test F: Cover Cache Isolation ---")
    proc, pid = launch_yomu()
    out_base = os.path.join(RAW_DATA_DIR, "test_f_cover_cache")
    current_label = ["init"]
    
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, 180, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    
    try:
        time.sleep(8) # Baseline on ContinueReading (3 covers)
        current_label[0] = "baseline_3_covers"
        time.sleep(5)
        
        # Click "All Manga" in sidebar: x = 1050, y = 185
        print("Clicking All Manga sidebar button...")
        current_label[0] = "switch_to_all_manga"
        run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -5000 -5000; wlrctl pointer move 1050 185; wlrctl pointer click'")
        time.sleep(3)
        
        # Scroll through library grid to populate all 118 covers into cache
        print("Scrolling through 118 books to populate cover cache...")
        current_label[0] = "scrolling_populate_covers"
        run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -5000 -5000; wlrctl pointer move 1400 500'")
        for _ in range(12):
            run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer scroll 150 0'")
            time.sleep(0.4)
            
        current_label[0] = "all_118_covers_loaded"
        time.sleep(8)
        
        # Now click a card to open reader
        print("Opening a book from All Manga grid...")
        current_label[0] = "opening_reader_from_grid"
        run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -5000 -5000; wlrctl pointer move 1350 400; wlrctl pointer click'")
        time.sleep(3)
        
        # Turn 10 pages
        current_label[0] = "reading_10_pages"
        cmd = " ".join([f"-k Right -s 350" for _ in range(10)])
        run_cmd(f"nix-shell -p wtype --run 'wtype {cmd}'")
        time.sleep(5)
        
        # Close reader
        print("Closing reader...")
        current_label[0] = "post_close_reader"
        run_cmd("nix-shell -p wtype --run 'wtype -k Escape'")
        time.sleep(30)
        
        print("Test F actions complete, waiting for monitor...")
        t.join(timeout=10)
        print("Test F complete.")
    finally:
        quit_yomu(proc)

if __name__ == "__main__":
    run_test_f_cover_cache()
