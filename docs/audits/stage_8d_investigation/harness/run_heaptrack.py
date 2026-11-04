#!/usr/bin/env python3
"""
Heaptrack Profiling Harness for Yomu
Stage 8D - Comprehensive Memory Retention & Leak Investigation
"""

import os
import sys
import time
import subprocess
import glob

HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))
STAGE8D_DIR = os.path.dirname(HARNESS_DIR)
HEAPTRACK_DIR = os.path.join(STAGE8D_DIR, "heaptrack")
YOMU_BIN = os.path.abspath("target/release/yomu")

os.makedirs(HEAPTRACK_DIR, exist_ok=True)

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid():
    res = run_cmd("pgrep -x yomu", check=False)
    for p in res.stdout.strip().split():
        if p.isdigit():
            return int(p)
    return None

def focus_and_warp():
    run_cmd("umbriel msg window-focus-warp yomu", check=False)
    run_cmd("nix-shell -p wlrctl --run 'wlrctl toplevel focus app_id:yomu'", check=False)
    time.sleep(0.5)

def click_card1():
    focus_and_warp()
    run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -87 43; wlrctl pointer click'")
    time.sleep(2)

def close_reader():
    run_cmd("nix-shell -p wtype --run 'wtype -k Escape'")
    time.sleep(1)

def turn_pages(count=10, delay_ms=400):
    cmd = " ".join([f"-k Right -s {delay_ms}" for _ in range(count)])
    run_cmd(f"nix-shell -p wtype --run 'wtype {cmd}'")

def main():
    print("\n--- Starting Heaptrack Profiling Session ---")
    # Launch heaptrack via nix-shell and devenv shell
    # heaptrack command wraps execution
    heaptrack_out_prefix = os.path.join(HEAPTRACK_DIR, "heaptrack.yomu")
    cmd = f"devenv shell -- /nix/store/jic77735miyy3nccpykpalaqv97hnwha-heaptrack-1.5.0-unstable-2025-07-21/bin/heaptrack -o \"{heaptrack_out_prefix}\" \"{YOMU_BIN}\""
    print("Launching:", cmd)
    proc = subprocess.Popen(cmd, shell=True)
    
    # Wait for yomu PID
    pid = None
    for _ in range(30):
        time.sleep(0.5)
        pid = get_yomu_pid()
        if pid:
            break
            
    if not pid:
        print("Failed to find yomu PID under heaptrack")
        proc.kill()
        return

    print(f"Yomu running under heaptrack with PID {pid}")
    time.sleep(5) # Allow startup to settle
    
    print("Opening book...")
    click_card1()
    time.sleep(3)
    
    print("Navigating 10 pages...")
    turn_pages(10, 400)
    time.sleep(5)
    
    print("Closing reader...")
    close_reader()
    
    print("Post-close settle (30s)...")
    time.sleep(30)
    
    print("Quitting Yomu cleanly (Ctrl-Q)...")
    run_cmd("nix-shell -p wtype --run 'wtype -M ctrl -k q -m ctrl'", check=False)
    time.sleep(3)
    
    # Wait for heaptrack process to terminate and finalize file
    try:
        proc.wait(timeout=15)
    except Exception:
        proc.terminate()
        proc.wait(timeout=5)
        
    print("Heaptrack run finished.")
    
    # Locate generated heaptrack output file
    files = glob.glob(f"{heaptrack_out_prefix}*")
    if not files:
        print("No heaptrack output files found in", HEAPTRACK_DIR)
        return
        
    latest_file = max(files, key=os.path.getmtime)
    print("Analyzing latest heaptrack file:", latest_file)
    analysis_txt = os.path.join(HEAPTRACK_DIR, "heaptrack_analysis.txt")
    print_cmd = f"nix-shell -p heaptrack --run 'heaptrack_print \"{latest_file}\" -p -l -T' > \"{analysis_txt}\""
    run_cmd(print_cmd)
    print("Heaptrack analysis saved to", analysis_txt)

if __name__ == "__main__":
    main()
