#!/usr/bin/env python3
"""
Isolated Diagnostic Experiment: Allocator Arena Retention vs Live Leak
Testing glibc malloc_trim(0) on post-close Yomu process.
NOTE: This is strictly an isolated diagnostic experiment, NOT a production fix.
"""

import os
import sys
import time
import subprocess
import shutil
import json

HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))
STAGE8D_DIR = os.path.dirname(HARNESS_DIR)
RAW_DATA_DIR = os.path.join(STAGE8D_DIR, "raw_data")
YOMU_BIN = os.path.abspath("target/release/yomu")
BACKUP_DIR = "/tmp/yomu_backup_stage8d"

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid():
    res = run_cmd("pgrep -x yomu", check=False)
    for p in res.stdout.strip().split():
        if p.isdigit():
            return int(p)
    return None

def get_proc_mem(pid):
    smaps = {}
    try:
        with open(f"/proc/{pid}/smaps_rollup", "r") as f:
            for line in f:
                parts = line.split(":")
                if len(parts) == 2:
                    k = parts[0].strip()
                    v = parts[1].strip().split()[0]
                    if v.isdigit():
                        smaps[k] = int(v)
    except Exception as e:
        smaps["error"] = str(e)
    return smaps

def main():
    print("\n--- Starting Isolated Diagnostic Experiment: malloc_trim(0) ---")
    proc = subprocess.Popen(["devenv", "shell", YOMU_BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    pid = None
    for _ in range(30):
        time.sleep(0.5)
        pid = get_yomu_pid()
        if pid:
            break
            
    if not pid:
        print("Failed to launch Yomu")
        return

    try:
        time.sleep(5)
        m_base = get_proc_mem(pid)
        print(f"1. Baseline Idle Memory:")
        print(f"   RSS: {m_base.get('Rss', 0)/1024:.1f} MiB | Anon: {m_base.get('Anonymous', 0)/1024:.1f} MiB")

        # Open book
        print("Opening book...")
        run_cmd("umbriel msg window-focus-warp yomu", check=False)
        run_cmd("nix-shell -p wlrctl --run 'wlrctl toplevel focus app_id:yomu'", check=False)
        time.sleep(1)
        run_cmd("nix-shell -p wlrctl --run 'wlrctl pointer move -87 43; wlrctl pointer click'")
        time.sleep(3)

        # Verify book opened
        res = run_cmd("umbriel windows", check=False).stdout
        print("Window status:", [line for line in res.splitlines() if "yomu" in line])

        # Turn pages
        cmd = " ".join([f"-k Right -s 350" for _ in range(12)])
        run_cmd(f"nix-shell -p wtype --run 'wtype {cmd}'")
        time.sleep(5)

        m_peak = get_proc_mem(pid)
        print(f"2. Peak Reading Memory:")
        print(f"   RSS: {m_peak.get('Rss', 0)/1024:.1f} MiB | Anon: {m_peak.get('Anonymous', 0)/1024:.1f} MiB")

        # Close reader
        run_cmd("nix-shell -p wtype --run 'wtype -k Escape'")
        time.sleep(10)

        m_closed = get_proc_mem(pid)
        print(f"3. Post-Close Settled Memory (Normal glibc behavior):")
        print(f"   RSS: {m_closed.get('Rss', 0)/1024:.1f} MiB | Anon: {m_closed.get('Anonymous', 0)/1024:.1f} MiB")

        # Isolated experiment: Call malloc_trim(0) via gdb
        print("4. Executing isolated diagnostic malloc_trim(0) via GDB...")
        gdb_cmd = f"gdb -batch -ex 'attach {pid}' -ex 'call malloc_trim(0)' -ex 'detach' -ex 'quit'"
        run_cmd(f"devenv shell -- {gdb_cmd}", check=False)
        time.sleep(2)

        m_after_trim = get_proc_mem(pid)
        print(f"5. Memory IMMEDIATELY AFTER malloc_trim(0):")
        print(f"   RSS: {m_after_trim.get('Rss', 0)/1024:.1f} MiB | Anon: {m_after_trim.get('Anonymous', 0)/1024:.1f} MiB")
        
        diff_rss = (m_closed.get('Rss', 0) - m_after_trim.get('Rss', 0)) / 1024.0
        diff_anon = (m_closed.get('Anonymous', 0) - m_after_trim.get('Anonymous', 0)) / 1024.0
        print(f"\nDiagnostic Result: malloc_trim(0) reclaimed {diff_rss:.1f} MiB of RSS ({diff_anon:.1f} MiB Anonymous memory)!")
        
        # Save results to JSON
        res = {
            "baseline": m_base,
            "peak": m_peak,
            "post_close": m_closed,
            "after_trim": m_after_trim,
            "reclaimed_rss_mib": diff_rss,
            "reclaimed_anon_mib": diff_anon,
        }
        with open(os.path.join(RAW_DATA_DIR, "diagnostic_trim_experiment.json"), "w") as f:
            json.dump(res, f, indent=2)

    finally:
        run_cmd("nix-shell -p wtype --run 'wtype -M ctrl -k q -m ctrl'", check=False)
        time.sleep(1)
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=3)

if __name__ == "__main__":
    main()
