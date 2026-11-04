#!/usr/bin/env python3
"""
Stage 8F.1 — Jemalloc Benchmark Reconciliation & Extended Stability Investigation
Executes:
  1. Side-by-side reconciliation of Stage 8E binary vs Stage 8F binary under identical clean conditions.
  2. 15-cycle consecutive reader soak test measuring peak, post-close, smaps mappings, and extended decay.
  3. High-sample (30-turn) page navigation latency profiling (mean, median, p95, p99).
"""

import os
import sys
import time
import subprocess
import json
import shutil
import glob
import threading

STAGE_DIR = os.path.dirname(os.path.abspath(__file__))
RAW_DATA_DIR = os.path.join(STAGE_DIR, "raw_data")
CHARTS_DIR = os.path.join(STAGE_DIR, "charts")
STAGE_8E_HARNESS = os.path.join(os.path.dirname(STAGE_DIR), "stage_8e_allocator_investigation", "harness")
STAGE_8E_BIN = os.path.join(os.path.dirname(STAGE_DIR), "stage_8e_allocator_investigation", "bin", "yomu-jemalloc")
PROD_BIN = os.path.abspath("target/release/yomu")
BACKUP_DIR = "/tmp/yomu_backup_stage8f"
DATA_DIR = os.path.expanduser("~/.local/share/yomu")

sys.path.insert(0, STAGE_8E_HARNESS)
from monitor import monitor_loop

os.makedirs(RAW_DATA_DIR, exist_ok=True)
os.makedirs(CHARTS_DIR, exist_ok=True)

def restore_clean_user_data():
    for f in ["library.json", "progress.json"]:
        src = os.path.join(BACKUP_DIR, f)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(DATA_DIR, f))
    time.sleep(0.3)

def run_cmd(cmd, check=True):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, check=check)

def get_yomu_pid(bin_name="yomu"):
    res = subprocess.run(["pgrep", "-f", bin_name], capture_output=True, text=True)
    for p in res.stdout.strip().split():
        if p.isdigit() and int(p) != os.getpid():
            try:
                with open(f"/proc/{p}/cmdline", "rb") as f:
                    cmd = f.read().decode("utf-8", errors="ignore")
                    if bin_name in cmd and "devenv" not in cmd and "python" not in cmd:
                        return int(p)
            except Exception:
                pass
    return None

def launch_app(bin_path):
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
    run_cmd("wlrctl toplevel focus app_id:yomu", check=False)
    time.sleep(0.3)

def click_card1():
    focus_yomu()
    run_cmd("wlrctl pointer move -5000 -5000")
    time.sleep(0.2)
    run_cmd("wlrctl pointer move 1350 600")
    time.sleep(0.2)
    run_cmd("wlrctl pointer click")
    time.sleep(3.5)

def close_reader():
    run_cmd("wtype -k Escape")
    time.sleep(1)

def turn_forward(count=6, delay_ms=300):
    cmd = " ".join([f"-k Right -s {delay_ms}" for _ in range(count)])
    run_cmd(f"wtype {cmd}")

def turn_backward(count=3, delay_ms=300):
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

def analyze_smaps_mappings(pid):
    """Analyze /proc/<pid>/smaps to extract jemalloc chunk metrics."""
    res = {
        "anon_mappings_count": 0,
        "anon_mapped_total_kb": 0,
        "anon_dirty_total_kb": 0,
        "anon_large_chunks_count": 0, # >= 2 MB
        "anon_large_chunks_kb": 0,
    }
    try:
        with open(f"/proc/{pid}/smaps", "r") as f:
            cur_is_anon = False
            cur_size = 0
            for line in f:
                if "-" in line and ("/" not in line or "[anon" in line or line.strip().endswith("]")):
                    # Header of anonymous mapping
                    cur_is_anon = True
                    parts = line.split()[0].split("-")
                    start = int(parts[0], 16)
                    end = int(parts[1], 16)
                    cur_size = (end - start) // 1024
                    res["anon_mappings_count"] += 1
                    res["anon_mapped_total_kb"] += cur_size
                    if cur_size >= 2048:
                        res["anon_large_chunks_count"] += 1
                        res["anon_large_chunks_kb"] += cur_size
                elif "-" in line:
                    cur_is_anon = False
                elif cur_is_anon and line.startswith("Private_Dirty:"):
                    v = int(line.split(":")[1].strip().split()[0])
                    res["anon_dirty_total_kb"] += v
    except Exception:
        pass
    return res

def run_reconciliation():
    print("\n=======================================================")
    print("PART 1: SIDE-BY-SIDE RECONCILIATION (Stage 8E vs Stage 8F)")
    print("=======================================================")
    
    results = {}
    targets = [
        ("stage_8e_diagnostic", STAGE_8E_BIN),
        ("stage_8f_production", PROD_BIN),
    ]
    
    for label, bin_path in targets:
        restore_clean_user_data()
        print(f"\n[*] Evaluating {label} ({os.path.basename(bin_path)})...")
        proc, pid = launch_app(bin_path)
        out_base = os.path.join(RAW_DATA_DIR, f"reconciliation_{label}")
        current_label = ["baseline"]
        
        t = threading.Thread(
            target=monitor_loop,
            args=(pid, 120, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
        )
        t.start()
        
        try:
            time.sleep(5)
            m_base = get_proc_mem(pid)
            
            current_label[0] = "opening_book"
            click_card1()
            time.sleep(3)
            m_open = get_proc_mem(pid)
            
            current_label[0] = "reading_turn_forward"
            turn_forward(12, 300)
            time.sleep(3)
            
            current_label[0] = "reading_turn_backward"
            turn_backward(5, 300)
            time.sleep(3)
            
            current_label[0] = "peak_reading"
            time.sleep(4)
            m_peak = get_proc_mem(pid)
            
            current_label[0] = "closing_reader"
            close_reader()
            
            current_label[0] = "post_close_30s"
            time.sleep(30)
            m_30s = get_proc_mem(pid)
            
            current_label[0] = "post_close_60s"
            time.sleep(30)
            m_60s = get_proc_mem(pid)
            
            results[label] = {
                "baseline_rss_mib": m_base.get("Rss", 0) / 1024.0,
                "after_open_rss_mib": m_open.get("Rss", 0) / 1024.0,
                "peak_reading_rss_mib": m_peak.get("Rss", 0) / 1024.0,
                "post_close_30s_rss_mib": m_30s.get("Rss", 0) / 1024.0,
                "post_close_60s_rss_mib": m_60s.get("Rss", 0) / 1024.0,
                "peak_anon_mib": m_peak.get("Anonymous", 0) / 1024.0,
                "post_close_60s_anon_mib": m_60s.get("Anonymous", 0) / 1024.0,
            }
            
            print(f"[{label}] Results:")
            print(f"  Baseline:   {results[label]['baseline_rss_mib']:.1f} MiB")
            print(f"  After Open: {results[label]['after_open_rss_mib']:.1f} MiB")
            print(f"  Peak RSS:   {results[label]['peak_reading_rss_mib']:.1f} MiB")
            print(f"  Post-Close (+60s): {results[label]['post_close_60s_rss_mib']:.1f} MiB")
            t.join(timeout=5)
        finally:
            quit_yomu(proc)
            
    with open(os.path.join(RAW_DATA_DIR, "reconciliation_summary.json"), "w") as f:
        json.dump(results, f, indent=2)

def run_15_cycle_soak():
    print("\n=======================================================")
    print("PART 2: EXTENDED 15-CYCLE MEMORY SOAK TEST")
    print("=======================================================")
    
    restore_clean_user_data()
    proc, pid = launch_app(PROD_BIN)
    out_base = os.path.join(RAW_DATA_DIR, "prod_jemalloc_15_cycles_soak")
    current_label = ["baseline"]
    
    # Total duration: 10s baseline + 15 * 24s + 180s extended settle = ~550s
    total_time = 580
    t = threading.Thread(
        target=monitor_loop,
        args=(pid, total_time, 1.0, f"{out_base}.csv", f"{out_base}.json", lambda: current_label[0])
    )
    t.start()
    
    cycle_records = []
    
    try:
        time.sleep(6)
        m_start = get_proc_mem(pid)
        print(f"[Baseline] Startup RSS: {m_start.get('Rss', 0)/1024:.1f} MiB | Anon: {m_start.get('Anonymous', 0)/1024:.1f} MiB")
        
        for i in range(1, 16):
            current_label[0] = f"cycle_{i}_open"
            click_card1()
            time.sleep(2)
            
            current_label[0] = f"cycle_{i}_read"
            turn_forward(6, 250)
            time.sleep(2)
            
            m_peak = get_proc_mem(pid)
            smaps_peak = analyze_smaps_mappings(pid)
            
            current_label[0] = f"cycle_{i}_close"
            close_reader()
            time.sleep(10) # 10s post-close settle
            
            m_closed = get_proc_mem(pid)
            smaps_closed = analyze_smaps_mappings(pid)
            
            record = {
                "cycle": i,
                "peak_rss_mib": m_peak.get("Rss", 0) / 1024.0,
                "peak_pss_mib": m_peak.get("Pss", 0) / 1024.0,
                "peak_anon_mib": m_peak.get("Anonymous", 0) / 1024.0,
                "peak_dirty_mib": m_peak.get("Private_Dirty", 0) / 1024.0,
                "closed_rss_mib": m_closed.get("Rss", 0) / 1024.0,
                "closed_pss_mib": m_closed.get("Pss", 0) / 1024.0,
                "closed_anon_mib": m_closed.get("Anonymous", 0) / 1024.0,
                "closed_dirty_mib": m_closed.get("Private_Dirty", 0) / 1024.0,
                "anon_mapped_mb": smaps_closed["anon_mapped_total_kb"] / 1024.0,
                "large_chunks_count": smaps_closed["anon_large_chunks_count"],
            }
            cycle_records.append(record)
            
            print(f"Cycle {i:2d}: Peak RSS={record['peak_rss_mib']:5.1f}M | Closed RSS={record['closed_rss_mib']:5.1f}M | Anon={record['closed_anon_mib']:5.1f}M | Mapped={record['anon_mapped_mb']:5.1f}M")
            
        # Extended 180s post-soak settle
        current_label[0] = "post_soak_settle_180s"
        print("\n[*] Commencing 180s extended idle settle after Cycle 15...")
        for wait_step in [30, 60, 120, 180]:
            time.sleep(30 if wait_step == 30 else 30 if wait_step == 60 else 60)
            m_decay = get_proc_mem(pid)
            print(f"  +{wait_step}s Settle: RSS={m_decay.get('Rss', 0)/1024:.1f}M | Anon={m_decay.get('Anonymous', 0)/1024:.1f}M")
            
        t.join(timeout=10)
    finally:
        quit_yomu(proc)
        
    with open(os.path.join(RAW_DATA_DIR, "15_cycles_summary.json"), "w") as f:
        json.dump(cycle_records, f, indent=2)

def run_high_sample_latency():
    print("\n=======================================================")
    print("PART 3: HIGH-SAMPLE PAGE NAVIGATION LATENCY PROFILING (30 SAMPLES)")
    print("=======================================================")
    
    restore_clean_user_data()
    proc, pid = launch_app(PROD_BIN)
    
    try:
        time.sleep(4)
        click_card1()
        time.sleep(3)
        focus_yomu()
        
        samples = []
        # Warmup turns
        turn_forward(3, 200)
        time.sleep(1)
        
        for _ in range(30):
            t0 = time.perf_counter()
            run_cmd("wtype -k Right")
            time.sleep(0.15)
            dt = (time.perf_counter() - t0) * 1000.0
            samples.append(dt)
            
        sorted_samples = sorted(samples)
        mean_lat = sum(samples) / len(samples)
        median_lat = sorted_samples[len(samples) // 2]
        p95_lat = sorted_samples[int(len(samples) * 0.95)]
        p99_lat = sorted_samples[int(len(samples) * 0.99)]
        min_lat = sorted_samples[0]
        max_lat = sorted_samples[-1]
        
        print(f"Latency Statistics (30 turns):")
        print(f"  Mean:   {mean_lat:.1f} ms")
        print(f"  Median: {median_lat:.1f} ms")
        print(f"  Min:    {min_lat:.1f} ms")
        print(f"  Max:    {max_lat:.1f} ms")
        print(f"  P95:    {p95_lat:.1f} ms")
        print(f"  P99:    {p99_lat:.1f} ms")
        
        out_path = os.path.join(RAW_DATA_DIR, "prod_jemalloc_latency_30_samples.json")
        with open(out_path, "w") as f:
            json.dump({
                "mean_ms": mean_lat,
                "median_ms": median_lat,
                "min_ms": min_lat,
                "max_ms": max_lat,
                "p95_ms": p95_lat,
                "p99_ms": p99_lat,
                "samples_ms": samples
            }, f, indent=2)
    finally:
        quit_yomu(proc)

def main():
    restore_clean_user_data()
    try:
        run_reconciliation()
        run_15_cycle_soak()
        run_high_sample_latency()
    finally:
        restore_clean_user_data()
        print("\n[+] Stage 8F.1 Investigation completed successfully!")

if __name__ == "__main__":
    main()
