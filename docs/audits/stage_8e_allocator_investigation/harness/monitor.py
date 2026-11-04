#!/usr/bin/env python3
"""
Automated Memory Measurement Harness for Yomu
Stage 8D - Comprehensive Memory Retention & Leak Investigation
"""

import sys
import os
import time
import json
import csv
import subprocess
import glob
from datetime import datetime

def find_yomu_pid():
    """Find the running Yomu process PID."""
    try:
        out = subprocess.check_output(["pgrep", "-f", "target/(release|debug)/yomu"]).decode().strip()
        pids = [int(p) for p in out.split() if p.isdigit()]
        for p in pids:
            if p != os.getpid():
                return p
    except Exception:
        pass
    # Fallback to searching /proc
    for proc in glob.glob("/proc/[0-9]*"):
        try:
            pid = int(os.path.basename(proc))
            if pid == os.getpid():
                continue
            with open(os.path.join(proc, "cmdline"), "rb") as f:
                cmd = f.read().decode('utf-8', errors='ignore')
                if "target/release/yomu" in cmd or "target/debug/yomu" in cmd:
                    return pid
        except Exception:
            continue
    return None

def parse_proc_status(pid):
    """Parse /proc/<pid>/status for VmSize, VmRSS, Threads, etc."""
    res = {}
    try:
        with open(f"/proc/{pid}/status", "r") as f:
            for line in f:
                parts = line.split(":")
                if len(parts) == 2:
                    k = parts[0].strip()
                    v = parts[1].strip()
                    res[k] = v
    except Exception as e:
        res["error"] = str(e)
    return res

def parse_proc_smaps_rollup(pid):
    """Parse /proc/<pid>/smaps_rollup for RSS, PSS, Private/Shared, Anonymous."""
    res = {}
    try:
        with open(f"/proc/{pid}/smaps_rollup", "r") as f:
            for line in f:
                parts = line.split(":")
                if len(parts) == 2:
                    k = parts[0].strip()
                    # e.g. "Rss:                3184 kB"
                    v = parts[1].strip().split()[0]
                    if v.isdigit():
                        res[k] = int(v)
    except Exception as e:
        res["error"] = str(e)
    return res

def get_fd_count(pid):
    """Count open file descriptors in /proc/<pid>/fd."""
    try:
        return len(os.listdir(f"/proc/{pid}/fd"))
    except Exception:
        return 0

def get_cpu_times(pid):
    """Read utime and stime from /proc/<pid>/stat."""
    try:
        with open(f"/proc/{pid}/stat", "r") as f:
            content = f.read()
            # Comm might contain spaces/parentheses
            rparen = content.rfind(")")
            rest = content[rparen + 2:].split()
            utime = int(rest[11])
            stime = int(rest[12])
            return utime, stime
    except Exception:
        return 0, 0

def get_system_mem():
    """Read system memory from /proc/meminfo."""
    res = {}
    try:
        with open("/proc/meminfo", "r") as f:
            for line in f:
                parts = line.split(":")
                if len(parts) == 2:
                    k = parts[0].strip()
                    v = parts[1].strip().split()[0]
                    if v.isdigit():
                        res[k] = int(v)
    except Exception:
        pass
    return res

def get_gpu_mem(pid):
    """Query nvidia-smi for per-process GPU memory if available."""
    try:
        out = subprocess.check_output(
            ["nvidia-smi", "--query-compute-apps=pid,used_memory", "--format=csv,noheader,nounits"],
            stderr=subprocess.DEVNULL
        ).decode().strip()
        for line in out.splitlines():
            parts = line.split(",")
            if len(parts) >= 2 and parts[0].strip() == str(pid):
                return int(parts[1].strip())
    except Exception:
        pass
    return 0

def sample_process(pid, label=""):
    """Collect a single full telemetry sample."""
    smaps = parse_proc_smaps_rollup(pid)
    status = parse_proc_status(pid)
    fd_count = get_fd_count(pid)
    utime, stime = get_cpu_times(pid)
    sys_mem = get_system_mem()
    gpu_mem = get_gpu_mem(pid)
    
    threads = int(status.get("Threads", 0))
    vmsize_kb = 0
    vm_str = status.get("VmSize", "0 kB").split()[0]
    if vm_str.isdigit():
        vmsize_kb = int(vm_str)

    sample = {
        "timestamp_iso": datetime.now().isoformat(),
        "timestamp_epoch": time.time(),
        "pid": pid,
        "label": label,
        # smaps_rollup metrics (all in KiB)
        "rss_kib": smaps.get("Rss", 0),
        "pss_kib": smaps.get("Pss", 0),
        "pss_dirty_kib": smaps.get("Pss_Dirty", 0),
        "pss_anon_kib": smaps.get("Pss_Anon", 0),
        "pss_file_kib": smaps.get("Pss_File", 0),
        "pss_shmem_kib": smaps.get("Pss_Shmem", 0),
        "private_clean_kib": smaps.get("Private_Clean", 0),
        "private_dirty_kib": smaps.get("Private_Dirty", 0),
        "anonymous_kib": smaps.get("Anonymous", 0),
        "shared_clean_kib": smaps.get("Shared_Clean", 0),
        "shared_dirty_kib": smaps.get("Shared_Dirty", 0),
        "referenced_kib": smaps.get("Referenced", 0),
        # status metrics
        "vmsize_kib": vmsize_kb,
        "threads": threads,
        "fd_count": fd_count,
        "utime": utime,
        "stime": stime,
        # GPU
        "gpu_proc_mib": gpu_mem,
        # System
        "sys_mem_available_kib": sys_mem.get("MemAvailable", 0),
        "sys_mem_free_kib": sys_mem.get("MemFree", 0),
    }
    return sample

def monitor_loop(pid, duration_sec, interval_sec, csv_path, json_path, label_getter=None):
    """Sample PID periodically for duration_sec and append to files."""
    start_time = time.time()
    fieldnames = [
        "timestamp_iso", "timestamp_epoch", "elapsed_sec", "pid", "label",
        "rss_kib", "pss_kib", "private_dirty_kib", "private_clean_kib", "anonymous_kib",
        "shared_dirty_kib", "shared_clean_kib", "vmsize_kib", "threads", "fd_count",
        "utime", "stime", "gpu_proc_mib", "sys_mem_available_kib"
    ]
    
    file_exists = os.path.exists(csv_path)
    csv_file = open(csv_path, "a", newline="")
    writer = csv.DictWriter(csv_file, fieldnames=fieldnames)
    if not file_exists:
        writer.writeheader()
    
    json_file = open(json_path, "a")

    try:
        while time.time() - start_time < duration_sec:
            if not os.path.exists(f"/proc/{pid}"):
                print(f"Process {pid} exited.")
                break
            label = label_getter() if label_getter else ""
            sample = sample_process(pid, label)
            sample["elapsed_sec"] = round(time.time() - start_time, 2)
            
            # Row for CSV
            csv_row = {k: sample[k] for k in fieldnames if k in sample}
            writer.writerow(csv_row)
            csv_file.flush()
            
            # JSONL
            json_file.write(json.dumps(sample) + "\n")
            json_file.flush()
            
            time.sleep(interval_sec)
    finally:
        csv_file.close()
        json_file.close()

if __name__ == "__main__":
    if len(sys.argv) < 5:
        print("Usage: monitor.py <pid> <duration_sec> <interval_sec> <output_base_path> [label]")
        sys.exit(1)
    
    pid = int(sys.argv[1])
    dur = float(sys.argv[2])
    interval = float(sys.argv[3])
    out_base = sys.argv[4]
    lbl = sys.argv[5] if len(sys.argv) > 5 else "sample"
    
    csv_out = f"{out_base}.csv"
    json_out = f"{out_base}.json"
    
    print(f"Monitoring PID {pid} for {dur}s at {interval}s interval -> {csv_out}")
    monitor_loop(pid, dur, interval, csv_out, json_out, lambda: lbl)
