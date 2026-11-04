#!/usr/bin/env python3
"""
Comprehensive glibc Diagnostics & malloc_trim(0) Verification (Stage 8E)
"""

import os
import sys
import time
import subprocess
import json
import shutil
import glob

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
INVESTIGATION_DIR = os.path.dirname(SCRIPT_DIR)
DIAG_DIR = os.path.join(INVESTIGATION_DIR, "glibc_diagnostics")
RAW_DATA_DIR = os.path.join(INVESTIGATION_DIR, "raw_data")
YOMU_BIN = os.path.abspath("target/release/yomu")
SO_PATH = os.path.abspath(os.path.join(DIAG_DIR, "diag_glibc_helper.so"))
BACKUP_DIR = "/tmp/yomu_backup_stage8e"

os.makedirs(DIAG_DIR, exist_ok=True)
os.makedirs(RAW_DATA_DIR, exist_ok=True)
os.makedirs(BACKUP_DIR, exist_ok=True)

def backup_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    for f in ["library.json", "progress.json"]:
        src = os.path.join(data_dir, f)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(BACKUP_DIR, f))
    print("[+] User data backed up to", BACKUP_DIR)

def restore_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    for f in ["library.json", "progress.json"]:
        src = os.path.join(BACKUP_DIR, f)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(data_dir, f))
    print("[+] User data restored from", BACKUP_DIR)

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

def get_arena_mappings(pid):
    """Scan /proc/<pid>/smaps for 64 MiB anonymous mappings (glibc arenas)."""
    arenas = []
    try:
        with open(f"/proc/{pid}/smaps", "r") as f:
            current_mapping = None
            for line in f:
                if "---p" in line or "rw-p" in line or "r--p" in line:
                    parts = line.split()
                    addr = parts[0]
                    perms = parts[1]
                    current_mapping = {"addr": addr, "perms": perms, "size_kb": 0, "rss_kb": 0, "pss_kb": 0, "anon_kb": 0}
                elif current_mapping and line.startswith("Size:"):
                    current_mapping["size_kb"] = int(line.split()[1])
                elif current_mapping and line.startswith("Rss:"):
                    current_mapping["rss_kb"] = int(line.split()[1])
                elif current_mapping and line.startswith("Pss:"):
                    current_mapping["pss_kb"] = int(line.split()[1])
                elif current_mapping and line.startswith("Anonymous:"):
                    current_mapping["anon_kb"] = int(line.split()[1])
                    if current_mapping["size_kb"] == 65536 or current_mapping["size_kb"] == 65472 or (current_mapping["size_kb"] >= 60000 and current_mapping["size_kb"] <= 66000):
                        arenas.append(dict(current_mapping))
    except Exception as e:
        pass
    return arenas

def focus_window():
    run_cmd("umbriel msg window-focus-warp yomu", check=False)
    run_cmd("wlrctl toplevel focus app_id:yomu", check=False)
    time.sleep(0.5)

def main():
    backup_user_data()
    try:
        print("[*] Launching Yomu with LD_PRELOAD diagnostic helper...")
        proc = subprocess.Popen(["devenv", "shell", "--", "env", f"LD_PRELOAD={SO_PATH}", YOMU_BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        
        pid = None
        for _ in range(30):
            time.sleep(0.5)
            pid = get_yomu_pid()
            if pid:
                break
        if not pid:
            print("[-] Error: Failed to find yomu PID")
            return

        print(f"[+] Yomu running with PID: {pid}")
        time.sleep(4) # Let UI settle
        focus_window()

        m_base = get_proc_mem(pid)
        print(f"1. Baseline Idle Memory:")
        print(f"   RSS: {m_base.get('Rss', 0)/1024:.2f} MiB | PSS: {m_base.get('Pss', 0)/1024:.2f} MiB | Anon: {m_base.get('Anonymous', 0)/1024:.2f} MiB | Dirty: {m_base.get('Private_Dirty', 0)/1024:.2f} MiB")

        # Open book: reset to (0,0) and click Card 1 at (873, 583)
        print("[*] Opening book...")
        run_cmd("wlrctl pointer move -5000 -5000")
        time.sleep(0.2)
        run_cmd("wlrctl pointer move 873 583")
        time.sleep(0.2)
        run_cmd("wlrctl pointer click")
        time.sleep(2)

        # Check if book opened, if not try hero button (1023, 293)
        m_curr = get_proc_mem(pid)
        if m_curr.get('Rss', 0) < 250000:
            print("[*] Retrying open via Hero button (1023, 293)...")
            run_cmd("wlrctl pointer move -5000 -5000")
            time.sleep(0.2)
            run_cmd("wlrctl pointer move 1023 293")
            time.sleep(0.2)
            run_cmd("wlrctl pointer click")
            time.sleep(2)

        # Confirm reader is active
        m_curr = get_proc_mem(pid)
        print(f"   Current RSS after open: {m_curr.get('Rss', 0)/1024:.2f} MiB")

        # Navigate forward 15 pages
        print("[*] Turning pages forward...")
        turn_cmd = " ".join(["-k Right -s 300" for _ in range(15)])
        run_cmd(f"wtype {turn_cmd}")
        time.sleep(3)

        # Navigate backward 5 pages
        print("[*] Turning pages backward...")
        turn_back = " ".join(["-k Left -s 300" for _ in range(5)])
        run_cmd(f"wtype {turn_back}")
        time.sleep(5)

        m_peak = get_proc_mem(pid)
        arenas_peak = get_arena_mappings(pid)
        print(f"2. Peak Reading Memory:")
        print(f"   RSS: {m_peak.get('Rss', 0)/1024:.2f} MiB | PSS: {m_peak.get('Pss', 0)/1024:.2f} MiB | Anon: {m_peak.get('Anonymous', 0)/1024:.2f} MiB | Dirty: {m_peak.get('Private_Dirty', 0)/1024:.2f} MiB")
        print(f"   64 MiB Arena Mappings at Peak: {len(arenas_peak)} arenas, total anon in arenas: {sum(a['anon_kb'] for a in arenas_peak)/1024:.2f} MiB")

        # Close reader
        print("[*] Closing reader with Escape...")
        run_cmd("wtype -k Escape")
        time.sleep(10) # 10s wait for garbage texture drain and post-close settle

        m_post_close = get_proc_mem(pid)
        arenas_post_close = get_arena_mappings(pid)
        print(f"3. Post-Close Settled Memory (Normal glibc behavior):")
        print(f"   RSS: {m_post_close.get('Rss', 0)/1024:.2f} MiB | PSS: {m_post_close.get('Pss', 0)/1024:.2f} MiB | Anon: {m_post_close.get('Anonymous', 0)/1024:.2f} MiB | Dirty: {m_post_close.get('Private_Dirty', 0)/1024:.2f} MiB")
        print(f"   64 MiB Arena Mappings Post-Close: {len(arenas_post_close)} arenas, total anon in arenas: {sum(a['anon_kb'] for a in arenas_post_close)/1024:.2f} MiB")

        # Trigger diagnostic helper via SIGUSR1
        print("[*] Sending SIGUSR1 to trigger diagnostic mallinfo2, malloc_info, and malloc_trim(0)...")
        run_cmd(f"kill -USR1 {pid}")
        time.sleep(3)

        m_after_trim = get_proc_mem(pid)
        arenas_after_trim = get_arena_mappings(pid)
        print(f"4. Memory IMMEDIATELY AFTER malloc_trim(0):")
        print(f"   RSS: {m_after_trim.get('Rss', 0)/1024:.2f} MiB | PSS: {m_after_trim.get('Pss', 0)/1024:.2f} MiB | Anon: {m_after_trim.get('Anonymous', 0)/1024:.2f} MiB | Dirty: {m_after_trim.get('Private_Dirty', 0)/1024:.2f} MiB")
        print(f"   64 MiB Arena Mappings After Trim: {len(arenas_after_trim)} arenas, total anon in arenas: {sum(a['anon_kb'] for a in arenas_after_trim)/1024:.2f} MiB")

        diff_rss = (m_post_close.get('Rss', 0) - m_after_trim.get('Rss', 0)) / 1024.0
        diff_pss = (m_post_close.get('Pss', 0) - m_after_trim.get('Pss', 0)) / 1024.0
        diff_anon = (m_post_close.get('Anonymous', 0) - m_after_trim.get('Anonymous', 0)) / 1024.0
        diff_dirty = (m_post_close.get('Private_Dirty', 0) - m_after_trim.get('Private_Dirty', 0)) / 1024.0

        print(f"\n=======================================================")
        print(f"DIAGNOSTIC VERDICT ON malloc_trim(0):")
        print(f"  Reclaimed RSS:           {diff_rss:.2f} MiB")
        print(f"  Reclaimed PSS:           {diff_pss:.2f} MiB")
        print(f"  Reclaimed Anonymous:     {diff_anon:.2f} MiB")
        print(f"  Reclaimed Private Dirty: {diff_dirty:.2f} MiB")
        print(f"=======================================================\n")

        # Copy generated diagnostic artifacts
        for src, dst in [
            ("/tmp/yomu_glibc_mallinfo.txt", os.path.join(DIAG_DIR, "mallinfo2_log.txt")),
            ("/tmp/yomu_glibc_malloc_info.xml", os.path.join(DIAG_DIR, "malloc_info_pre_trim.xml")),
            ("/tmp/yomu_glibc_malloc_info_post_trim.xml", os.path.join(DIAG_DIR, "malloc_info_post_trim.xml")),
            ("/tmp/yomu_glibc_trim_result.txt", os.path.join(DIAG_DIR, "trim_result.txt")),
        ]:
            if os.path.exists(src):
                shutil.copy2(src, dst)
                print(f"[+] Saved {dst}")

        # Save structured results
        report = {
            "baseline": m_base,
            "peak": m_peak,
            "post_close": m_post_close,
            "after_trim": m_after_trim,
            "arenas_peak": arenas_peak,
            "arenas_post_close": arenas_post_close,
            "arenas_after_trim": arenas_after_trim,
            "reclaimed_rss_mib": diff_rss,
            "reclaimed_pss_mib": diff_pss,
            "reclaimed_anon_mib": diff_anon,
            "reclaimed_dirty_mib": diff_dirty,
        }
        with open(os.path.join(RAW_DATA_DIR, "glibc_diagnostic_experiment.json"), "w") as f:
            json.dump(report, f, indent=2)

    finally:
        print("[*] Quitting Yomu...")
        run_cmd("wtype -M ctrl -k q -m ctrl", check=False)
        time.sleep(1)
        pid = get_yomu_pid()
        if pid:
            run_cmd(f"kill -9 {pid}", check=False)
        restore_user_data()

if __name__ == "__main__":
    main()
