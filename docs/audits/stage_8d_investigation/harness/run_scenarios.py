#!/usr/bin/env python3
"""
Orchestrator for Yomu Memory Retention Investigation (Stage 8D)
Executes Tests A, B, C, D, E, F, G using real GUI automation on Wayland.
"""

import os
import sys
import time
import subprocess
import json
import shutil

HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))
STAGE8D_DIR = os.path.dirname(HARNESS_DIR)
RAW_DATA_DIR = os.path.join(STAGE8D_DIR, "raw_data")
YOMU_BIN = os.path.abspath("target/release/yomu")
BACKUP_DIR = "/tmp/yomu_backup_stage8d"

os.makedirs(RAW_DATA_DIR, exist_ok=True)
os.makedirs(BACKUP_DIR, exist_ok=True)

def backup_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    lib = os.path.join(data_dir, "library.json")
    prog = os.path.join(data_dir, "progress.json")
    if os.path.exists(lib):
        shutil.copy2(lib, os.path.join(BACKUP_DIR, "library.json"))
    if os.path.exists(prog):
        shutil.copy2(prog, os.path.join(BACKUP_DIR, "progress.json"))
    print("User data backed up to", BACKUP_DIR)

def restore_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    lib = os.path.join(BACKUP_DIR, "library.json")
    prog = os.path.join(BACKUP_DIR, "progress.json")
    if os.path.exists(lib):
        shutil.copy2(lib, os.path.join(data_dir, "library.json"))
    if os.path.exists(prog):
        shutil.copy2(prog, os.path.join(data_dir, "progress.json"))
    print("User data restored from", BACKUP_DIR)

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid():
    res = run_cmd("pgrep -f 'target/release/yomu'", check=False)
    for p in res.stdout.strip().split():
        if p.isdigit():
            return int(p)
    return None

def launch_yomu():
    # Launch yomu within devenv shell
    proc = subprocess.Popen(["devenv", "shell", YOMU_BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # Wait for yomu to appear
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

def click_card1():
    # In umbriel, card 1 (Berserk v40) is at relative offset (-87, +43) from window center
    focus_and_warp()
    run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -87 43; wlrctl pointer click'")
    time.sleep(2)

def click_hero_continue():
    # In continue reading, the hero button is at (+63, -247) from center
    focus_and_warp()
    run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move 63 -247; wlrctl pointer click'")
    time.sleep(2)

def close_reader():
    run_cmd("nix-shell -p wtype --run 'wtype -k Escape'")
    time.sleep(1)

def turn_pages_forward(count=5, delay_ms=400):
    cmd = " ".join([f"-k Right -s {delay_ms}" for _ in range(count)])
    run_cmd(f"nix-shell -p wtype --run 'wtype {cmd}'")

def turn_pages_backward(count=5, delay_ms=400):
    cmd = " ".join([f"-k Left -s {delay_ms}" for _ in range(count)])
    run_cmd(f"nix-shell -p wtype --run 'wtype {cmd}'")

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

def run_test_a():
    """Test A: Application Idle Baseline for 60s."""
    print("\n--- Starting Test A: Application Idle Baseline ---")
    proc, pid = launch_yomu()
    try:
        focus_and_warp()
        time.sleep(3) # Settle startup
        out_base = os.path.join(RAW_DATA_DIR, "test_a_idle_baseline")
        monitor_cmd = f"python3 {HARNESS_DIR}/monitor.py {pid} 60 1.0 {out_base} idle_baseline"
        run_cmd(monitor_cmd)
        print("Test A complete.")
    finally:
        quit_yomu(proc)

def run_test_b():
    """Test B & G: Open and Close One Manga with 300s (5 min) extended idle monitoring."""
    print("\n--- Starting Test B & G: Open, Close, and 5-Minute Extended Idle ---")
    proc, pid = launch_yomu()
    out_base = os.path.join(RAW_DATA_DIR, "test_b_g_open_close_extended")
    
    # Start background monitor for 360 seconds total
    import threading
    from monitor import monitor_loop
    current_label = ["baseline"]
    
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, 360, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    
    try:
        time.sleep(10) # 10s baseline
        current_label[0] = "opening_book"
        click_card1()
        time.sleep(3)
        
        current_label[0] = "reading_turn_forward"
        turn_pages_forward(12, 350)
        time.sleep(3)
        
        current_label[0] = "reading_turn_backward"
        turn_pages_backward(5, 350)
        time.sleep(5)
        
        current_label[0] = "peak_reading"
        time.sleep(5)
        
        current_label[0] = "closing_book"
        close_reader()
        
        current_label[0] = "post_close_0_30s"
        time.sleep(30)
        
        current_label[0] = "post_close_30_60s"
        time.sleep(30)
        
        current_label[0] = "post_close_60_120s"
        time.sleep(60)
        
        current_label[0] = "post_close_120_180s"
        time.sleep(60)

        current_label[0] = "post_close_180_300s_extended"
        time.sleep(120)
        
        print("Test B & G actions complete, waiting for monitor thread...")
        t.join(timeout=10)
        print("Test B & G complete.")
    finally:
        quit_yomu(proc)

def run_test_c():
    """Test C: Repeated Reader Lifecycle (5 cycles)."""
    print("\n--- Starting Test C: Repeated Reader Lifecycle (5 Cycles) ---")
    proc, pid = launch_yomu()
    out_base = os.path.join(RAW_DATA_DIR, "test_c_repeated_cycles")
    
    import threading
    from monitor import monitor_loop
    current_label = ["init"]
    
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, 300, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    
    try:
        time.sleep(5)
        for cycle in range(1, 6):
            print(f"  Cycle {cycle}/5: Opening book...")
            current_label[0] = f"cycle_{cycle}_open"
            click_card1()
            time.sleep(2)
            
            print(f"  Cycle {cycle}/5: Turning pages...")
            current_label[0] = f"cycle_{cycle}_turn"
            turn_pages_forward(8, 300)
            time.sleep(2)
            
            current_label[0] = f"cycle_{cycle}_settle"
            time.sleep(3)
            
            print(f"  Cycle {cycle}/5: Closing book...")
            current_label[0] = f"cycle_{cycle}_closing"
            close_reader()
            
            current_label[0] = f"cycle_{cycle}_post_close"
            time.sleep(20) # 20s rest between cycles
            
        print("Test C cycles finished.")
        t.join(timeout=10)
        print("Test C complete.")
    finally:
        quit_yomu(proc)

def main():
    backup_user_data()
    try:
        run_test_a()
        run_test_b()
        run_test_c()
    finally:
        restore_user_data()

if __name__ == "__main__":
    main()
