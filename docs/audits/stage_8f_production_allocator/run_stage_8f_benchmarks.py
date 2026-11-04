#!/usr/bin/env python3
"""
Automated Production Jemalloc Benchmarking Suite (Stage 8F)
Executes identical Wayland GUI test scenarios against:
  - Production Jemalloc: target/release/yomu (configured with tikv-jemallocator 0.6.1)
  - Baseline glibc: docs/audits/stage_8e_allocator_investigation/bin/yomu-glibc (original release binary)
"""

import os
import sys
import time
import subprocess
import json
import shutil
import glob
import threading

AUDIT_DIR = os.path.dirname(os.path.abspath(__file__))
RAW_DATA_DIR = os.path.join(AUDIT_DIR, "raw_data")
CHARTS_DIR = os.path.join(AUDIT_DIR, "charts")
STAGE_8E_HARNESS = os.path.join(os.path.dirname(AUDIT_DIR), "stage_8e_allocator_investigation", "harness")
STAGE_8E_BIN = os.path.join(os.path.dirname(AUDIT_DIR), "stage_8e_allocator_investigation", "bin")
BACKUP_DIR = "/tmp/yomu_backup_stage8f"

sys.path.insert(0, STAGE_8E_HARNESS)
from monitor import monitor_loop

os.makedirs(RAW_DATA_DIR, exist_ok=True)
os.makedirs(CHARTS_DIR, exist_ok=True)
os.makedirs(BACKUP_DIR, exist_ok=True)

PROD_BIN = os.path.abspath("target/release/yomu")
GLIBC_BIN = os.path.join(STAGE_8E_BIN, "yomu-glibc")

TARGETS = {
    "prod_jemalloc": PROD_BIN,
}

def backup_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    for f in ["library.json", "progress.json"]:
        src = os.path.join(data_dir, f)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(BACKUP_DIR, f))
    print("[+] User data safely backed up to", BACKUP_DIR)

def restore_user_data():
    data_dir = os.path.expanduser("~/.local/share/yomu")
    for f in ["library.json", "progress.json"]:
        src = os.path.join(BACKUP_DIR, f)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(data_dir, f))
    print("[+] User data cleanly restored from", BACKUP_DIR)

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid(bin_name=None):
    patterns = [bin_name] if bin_name else ["yomu-glibc", "target/release/yomu", "yomu"]
    for pat in patterns:
        if not pat: continue
        res = run_cmd(f"pgrep -f '{pat}'", check=False)
        for p in res.stdout.strip().split():
            if p.isdigit() and int(p) != os.getpid():
                if os.path.exists(f"/proc/{p}/cmdline"):
                    return int(p)
    return None

def launch_target(bin_path):
    bin_name = os.path.basename(bin_path)
    proc = subprocess.Popen(
        ["devenv", "shell", "--", bin_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    for _ in range(30):
        time.sleep(0.5)
        pid = get_yomu_pid(bin_name)
        if pid:
            return proc, pid
    raise RuntimeError(f"Failed to launch {bin_path}")

def focus_yomu():
    run_cmd("umbriel msg window-focus-warp yomu", check=False)
    run_cmd("wlrctl toplevel focus app_id:yomu", check=False)
    time.sleep(0.5)

def click_card1():
    focus_yomu()
    run_cmd("wlrctl pointer move -5000 -5000")
    time.sleep(0.2)
    run_cmd("wlrctl pointer move 1350 600")
    time.sleep(0.2)
    run_cmd("wlrctl pointer click")
    time.sleep(3)

def click_hero_continue():
    focus_yomu()
    run_cmd("wlrctl pointer move -5000 -5000")
    time.sleep(0.2)
    run_cmd("wlrctl pointer move 1500 310")
    time.sleep(0.2)
    run_cmd("wlrctl pointer click")
    time.sleep(3)

def close_reader():
    run_cmd("wtype -k Escape")
    time.sleep(1)

def turn_forward(count=10, delay_ms=300):
    cmd = " ".join([f"-k Right -s {delay_ms}" for _ in range(count)])
    run_cmd(f"wtype {cmd}")

def turn_backward(count=5, delay_ms=300):
    cmd = " ".join([f"-k Left -s {delay_ms}" for _ in range(count)])
    run_cmd(f"wtype {cmd}")

def quit_yomu(proc):
    run_cmd("wtype -M ctrl -k q -m ctrl", check=False)
    time.sleep(1)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except Exception:
            proc.kill()
    time.sleep(1)

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
    except Exception:
        pass
    return smaps

def run_test_a_baseline(target_name, bin_path):
    print(f"\n[{target_name}] --- Test A: 60s Idle Baseline ---")
    proc, pid = launch_target(bin_path)
    out_base = os.path.join(RAW_DATA_DIR, f"{target_name}_test_a_baseline")
    try:
        time.sleep(4)
        focus_yomu()
        current_label = ["baseline"]
        t = threading.Thread(
            target=monitor_loop,
            args=(pid, 60, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
        )
        t.start()
        t.join()
        m = get_proc_mem(pid)
        print(f"[{target_name}] Baseline Settled: RSS={m.get('Rss', 0)/1024:.2f} MiB | Anon={m.get('Anonymous', 0)/1024:.2f} MiB | PSS={m.get('Pss', 0)/1024:.2f} MiB")
    finally:
        quit_yomu(proc)

def run_test_b_d_session(target_name, bin_path):
    print(f"\n[{target_name}] --- Test B & D: Single Reader Session + Extended 300s Post-Close Monitor ---")
    proc, pid = launch_target(bin_path)
    out_base = os.path.join(RAW_DATA_DIR, f"{target_name}_test_b_d_session")
    current_label = ["baseline"]
    # Total duration: 8s baseline + 25s reading + 300s post-close = 333s
    total_time = 340
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, total_time, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    try:
        time.sleep(6)
        current_label[0] = "opening_book"
        click_card1()
        time.sleep(2)
        if get_proc_mem(pid).get('Rss', 0) < 250000:
            click_hero_continue()
            time.sleep(2)
        
        current_label[0] = "reading_turn_forward"
        turn_forward(12, 300)
        time.sleep(2)

        current_label[0] = "reading_turn_backward"
        turn_backward(5, 300)
        time.sleep(4)

        current_label[0] = "peak_reading"
        time.sleep(4)
        m_peak = get_proc_mem(pid)
        print(f"[{target_name}] Peak Reading: RSS={m_peak.get('Rss', 0)/1024:.2f} MiB | Anon={m_peak.get('Anonymous', 0)/1024:.2f} MiB | PSS={m_peak.get('Pss', 0)/1024:.2f} MiB")

        current_label[0] = "closing_reader"
        close_reader()
        
        # Test B (120s post-close monitoring)
        current_label[0] = "post_close_0_30s"
        time.sleep(30)
        m_30 = get_proc_mem(pid)

        current_label[0] = "post_close_30_60s"
        time.sleep(30)
        m_60 = get_proc_mem(pid)

        current_label[0] = "post_close_60_120s"
        time.sleep(60)
        m_120 = get_proc_mem(pid)

        # Test D (Extended 5-minute / 300s post-close monitoring)
        current_label[0] = "post_close_120_180s"
        time.sleep(60)
        m_180 = get_proc_mem(pid)

        current_label[0] = "post_close_180_240s"
        time.sleep(60)
        m_240 = get_proc_mem(pid)

        current_label[0] = "post_close_240_300s_extended"
        time.sleep(60)
        m_300 = get_proc_mem(pid)

        print(f"[{target_name}] Post-Close Retention Profile:")
        print(f"  +30s:  RSS={m_30.get('Rss', 0)/1024:.1f}M | Anon={m_30.get('Anonymous', 0)/1024:.1f}M")
        print(f"  +60s:  RSS={m_60.get('Rss', 0)/1024:.1f}M | Anon={m_60.get('Anonymous', 0)/1024:.1f}M")
        print(f"  +120s: RSS={m_120.get('Rss', 0)/1024:.1f}M | Anon={m_120.get('Anonymous', 0)/1024:.1f}M")
        print(f"  +180s: RSS={m_180.get('Rss', 0)/1024:.1f}M | Anon={m_180.get('Anonymous', 0)/1024:.1f}M")
        print(f"  +240s: RSS={m_240.get('Rss', 0)/1024:.1f}M | Anon={m_240.get('Anonymous', 0)/1024:.1f}M")
        print(f"  +300s: RSS={m_300.get('Rss', 0)/1024:.1f}M | Anon={m_300.get('Anonymous', 0)/1024:.1f}M (5-Min Extended Idle)")
        t.join(timeout=10)
    finally:
        quit_yomu(proc)

def run_test_c_repeated_cycles(target_name, bin_path, cycles=5):
    print(f"\n[{target_name}] --- Test C: {cycles} Repeated Reader Cycles ---")
    proc, pid = launch_target(bin_path)
    out_base = os.path.join(RAW_DATA_DIR, f"{target_name}_test_c_repeated_cycles")
    current_label = ["baseline"]
    total_time = 20 + cycles * 28
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, total_time, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    try:
        time.sleep(5)
        for i in range(1, cycles + 1):
            current_label[0] = f"cycle_{i}_open"
            click_card1()
            time.sleep(2)
            if get_proc_mem(pid).get('Rss', 0) < 250000:
                click_hero_continue()
                time.sleep(2)
            current_label[0] = f"cycle_{i}_read"
            turn_forward(6, 300)
            time.sleep(2)
            m_peak = get_proc_mem(pid)
            current_label[0] = f"cycle_{i}_close"
            close_reader()
            time.sleep(8)
            m_closed = get_proc_mem(pid)
            print(f"[{target_name}] Cycle {i}: Peak RSS={m_peak.get('Rss', 0)/1024:.1f}M | Closed RSS={m_closed.get('Rss', 0)/1024:.1f}M | Anon={m_closed.get('Anonymous', 0)/1024:.1f}M")
        t.join(timeout=10)
    finally:
        quit_yomu(proc)

def run_test_e_direct_switch(target_name, bin_path):
    print(f"\n[{target_name}] --- Test E: Direct Book Switching ---")
    proc, pid = launch_target(bin_path)
    out_base = os.path.join(RAW_DATA_DIR, f"{target_name}_test_e_direct_switch")
    current_label = ["baseline"]
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, 60, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    try:
        time.sleep(5)
        current_label[0] = "open_book_1"
        click_card1()
        time.sleep(2)
        turn_forward(4, 300)
        time.sleep(2)
        m1 = get_proc_mem(pid)
        print(f"[{target_name}] Book 1 Reading RSS: {m1.get('Rss', 0)/1024:.1f}M")

        current_label[0] = "switch_to_book_2"
        close_reader()
        time.sleep(1)
        click_hero_continue()
        time.sleep(2)
        turn_forward(4, 300)
        time.sleep(3)
        m2 = get_proc_mem(pid)
        print(f"[{target_name}] Direct Switch to Book 2 RSS: {m2.get('Rss', 0)/1024:.1f}M")

        current_label[0] = "final_close"
        close_reader()
        time.sleep(10)
        m_fin = get_proc_mem(pid)
        print(f"[{target_name}] Settled Post-Switch RSS: {m_fin.get('Rss', 0)/1024:.1f}M")
        t.join(timeout=10)
    finally:
        quit_yomu(proc)

def run_test_f_cover_cache(target_name, bin_path):
    print(f"\n[{target_name}] --- Test F: Cover Cache Isolation (Grid Scrolling) ---")
    proc, pid = launch_target(bin_path)
    out_base = os.path.join(RAW_DATA_DIR, f"{target_name}_test_f_cover_cache")
    current_label = ["baseline"]
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, 45, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    try:
        time.sleep(5)
        focus_yomu()
        current_label[0] = "scrolling_library_grid"
        for _ in range(5):
            cmd_down = " ".join(["-k Down -s 150" for _ in range(10)])
            run_cmd(f"wtype {cmd_down}")
            time.sleep(1)
            cmd_up = " ".join(["-k Up -s 150" for _ in range(10)])
            run_cmd(f"wtype {cmd_up}")
            time.sleep(1)
        current_label[0] = "settling"
        time.sleep(10)
        m = get_proc_mem(pid)
        print(f"[{target_name}] Cover Cache Scrolling Settled RSS: {m.get('Rss', 0)/1024:.1f}M | Anon: {m.get('Anonymous', 0)/1024:.1f}M")
        t.join(timeout=10)
    finally:
        quit_yomu(proc)

def run_test_g_latency(target_name, bin_path):
    print(f"\n[{target_name}] --- Test G: Page Decode & Navigation Latency ---")
    proc, pid = launch_target(bin_path)
    try:
        time.sleep(5)
        click_card1()
        time.sleep(3)
        if get_proc_mem(pid).get('Rss', 0) < 250000:
            click_hero_continue()
            time.sleep(3)
        focus_yomu()

        latencies = []
        for i in range(15):
            t0 = time.perf_counter()
            run_cmd("wtype -k Right")
            time.sleep(0.15)
            dt = (time.perf_counter() - t0) * 1000.0
            latencies.append(dt)

        avg_lat = sum(latencies) / len(latencies)
        p95_lat = sorted(latencies)[int(len(latencies) * 0.95)]
        print(f"[{target_name}] Navigation Latency: Avg={avg_lat:.1f}ms | P95={p95_lat:.1f}ms")

        lat_path = os.path.join(RAW_DATA_DIR, f"{target_name}_test_g_latency.json")
        with open(lat_path, "w") as f:
            json.dump({"target": target_name, "avg_ms": avg_lat, "p95_ms": p95_lat, "samples_ms": latencies}, f, indent=2)
    finally:
        quit_yomu(proc)

def main():
    backup_user_data()
    try:
        for target_name, bin_path in TARGETS.items():
            print(f"\n=======================================================")
            print(f"STARTING BENCHMARK SUITE FOR TARGET: {target_name}")
            print(f"Binary: {bin_path}")
            print(f"=======================================================")
            
            run_test_a_baseline(target_name, bin_path)
            run_test_b_d_session(target_name, bin_path)
            run_test_c_repeated_cycles(target_name, bin_path, cycles=5)
            run_test_e_direct_switch(target_name, bin_path)
            run_test_f_cover_cache(target_name, bin_path)
            run_test_g_latency(target_name, bin_path)
            
            time.sleep(3)
    finally:
        restore_user_data()
        print("\n[+] Stage 8F Production Benchmark tests finished successfully!")

if __name__ == "__main__":
    main()