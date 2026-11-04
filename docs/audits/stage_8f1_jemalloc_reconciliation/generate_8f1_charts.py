#!/usr/bin/env python3
"""
Generate publication-quality charts for Stage 8F.1: Jemalloc Reconciliation & Stability Verification.
"""

import os
import csv
import json
import matplotlib.pyplot as plt

STAGE_DIR = os.path.dirname(os.path.abspath(__file__))
RAW_DIR = os.path.join(STAGE_DIR, "raw_data")
CHARTS_DIR = os.path.join(STAGE_DIR, "charts")

os.makedirs(CHARTS_DIR, exist_ok=True)

plt.style.use('seaborn-v0_8-whitegrid' if 'seaborn-v0_8-whitegrid' in plt.style.available else 'default')
plt.rcParams['font.sans-serif'] = 'DejaVu Sans'
plt.rcParams['font.size'] = 10

def load_csv(filepath):
    times, rss, anon, pss = [], [], [], []
    with open(filepath, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            times.append(float(row['elapsed_sec']))
            rss.append(float(row.get('rss_kib', row.get('rss_kb', 0))) / 1024.0)
            anon.append(float(row.get('anonymous_kib', row.get('anon_kb', 0))) / 1024.0)
            pss.append(float(row.get('pss_kib', row.get('pss_kb', 0))) / 1024.0)
    return times, rss, anon, pss

def plot_reconciliation():
    f_8e = os.path.join(RAW_DIR, "reconciliation_stage_8e_diagnostic.csv")
    f_8f = os.path.join(RAW_DIR, "reconciliation_stage_8f_production.csv")

    fig, ax = plt.subplots(figsize=(10, 5.5), dpi=150)
    t_8e, rss_8e, anon_8e, _ = load_csv(f_8e)
    t_8f, rss_8f, anon_8f, _ = load_csv(f_8f)

    ax.plot(t_8e, rss_8e, label='Stage 8E Diagnostic Binary (RSS)', color='#458588', linewidth=2.2)
    ax.plot(t_8e, anon_8e, label='Stage 8E Diagnostic Binary (Anonymous)', color='#458588', linestyle='--', alpha=0.7)

    ax.plot(t_8f, rss_8f, label='Stage 8F Production Binary (RSS)', color='#b16286', linewidth=2.2)
    ax.plot(t_8f, anon_8f, label='Stage 8F Production Binary (Anonymous)', color='#b16286', linestyle='--', alpha=0.7)

    ax.set_title('Stage 8F.1: Side-by-Side Binary Reconciliation (Identical Conditions)', fontsize=12, fontweight='bold', pad=12)
    ax.set_xlabel('Elapsed Time (seconds)', fontsize=11)
    ax.set_ylabel('Memory (MiB)', fontsize=11)
    ax.set_ylim(0, 550)
    ax.legend(loc='center right', frameon=True)
    plt.tight_layout()
    out = os.path.join(CHARTS_DIR, "reconciliation_8e_vs_8f.png")
    fig.savefig(out)
    plt.close()
    print(f"[+] Saved {out}")

def plot_15_cycles_plateau():
    summary_path = os.path.join(RAW_DIR, "15_cycles_summary.json")
    with open(summary_path) as f:
        records = json.load(f)

    cycles = [r["cycle"] for r in records]
    peaks = [r["peak_rss_mib"] for r in records]
    closed = [r["closed_rss_mib"] for r in records]
    anon = [r["closed_anon_mib"] for r in records]

    fig, ax = plt.subplots(figsize=(11, 5.5), dpi=150)
    ax.plot(cycles, peaks, 'o-', label='Peak Reading RSS (MiB)', color='#d65d0e', linewidth=2.0, markersize=6)
    ax.plot(cycles, closed, 's-', label='Post-Close Settled RSS (MiB)', color='#458588', linewidth=2.2, markersize=6)
    ax.plot(cycles, anon, '^--', label='Post-Close Anonymous (MiB)', color='#689d6a', linewidth=1.8, markersize=5)

    ax.axhline(419.0, color='#928374', linestyle=':', label='15-Cycle Mean Post-Close RSS (419.0 MiB)')

    ax.set_title('Stage 8F.1: Extended 15-Cycle Reading Soak Test (Memory Plateau)', fontsize=12, fontweight='bold', pad=12)
    ax.set_xlabel('Cycle Number', fontsize=11)
    ax.set_ylabel('Memory (MiB)', fontsize=11)
    ax.set_xticks(cycles)
    ax.set_ylim(200, 550)
    ax.legend(loc='lower right', frameon=True)
    plt.tight_layout()
    out = os.path.join(CHARTS_DIR, "soak_15_cycles_plateau.png")
    fig.savefig(out)
    plt.close()
    print(f"[+] Saved {out}")

def plot_15_cycles_timeline():
    f_timeline = os.path.join(RAW_DIR, "prod_jemalloc_15_cycles_soak.csv")
    times, rss, anon, pss = load_csv(f_timeline)

    fig, ax = plt.subplots(figsize=(12, 5.5), dpi=150)
    ax.plot(times, rss, label='Resident Set Size (RSS)', color='#458588', linewidth=1.8)
    ax.plot(times, anon, label='Anonymous Memory', color='#689d6a', linewidth=1.5, linestyle='--')
    ax.plot(times, pss, label='Proportional Set Size (PSS)', color='#d65d0e', linewidth=1.2, alpha=0.7)

    ax.set_title('Stage 8F.1: Continuous 15-Cycle Soak Telemetry (580 Seconds)', fontsize=12, fontweight='bold', pad=12)
    ax.set_xlabel('Elapsed Time (seconds)', fontsize=11)
    ax.set_ylabel('Memory (MiB)', fontsize=11)
    ax.set_ylim(0, 550)
    ax.legend(loc='lower right', frameon=True)
    plt.tight_layout()
    out = os.path.join(CHARTS_DIR, "soak_15_cycles_timeline.png")
    fig.savefig(out)
    plt.close()
    print(f"[+] Saved {out}")

def plot_latency_distribution():
    f_lat = os.path.join(RAW_DIR, "prod_jemalloc_latency_30_samples.json")
    with open(f_lat) as f:
        data = json.load(f)

    samples = data["samples_ms"]
    fig, ax = plt.subplots(figsize=(8, 5), dpi=150)
    ax.hist(samples, bins=12, color='#458588', edgecolor='white', alpha=0.85)
    ax.axvline(data["mean_ms"], color='#cc241d', linestyle='-', linewidth=2, label=f'Mean: {data["mean_ms"]:.1f} ms')
    ax.axvline(data["median_ms"], color='#d65d0e', linestyle='--', linewidth=2, label=f'Median: {data["median_ms"]:.1f} ms')
    ax.axvline(data["p95_ms"], color='#b16286', linestyle=':', linewidth=2, label=f'P95: {data["p95_ms"]:.1f} ms')

    ax.set_title('Stage 8F.1: Page Navigation Latency Distribution (30 Samples)', fontsize=12, fontweight='bold', pad=12)
    ax.set_xlabel('Latency (ms)', fontsize=11)
    ax.set_ylabel('Sample Count', fontsize=11)
    ax.legend(loc='upper right', frameon=True)
    plt.tight_layout()
    out = os.path.join(CHARTS_DIR, "latency_distribution.png")
    fig.savefig(out)
    plt.close()
    print(f"[+] Saved {out}")

if __name__ == "__main__":
    plot_reconciliation()
    plot_15_cycles_plateau()
    plot_15_cycles_timeline()
    plot_latency_distribution()
