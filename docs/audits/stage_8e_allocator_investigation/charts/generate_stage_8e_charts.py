#!/usr/bin/env python3
"""
Chart generator for Stage 8E Memory Allocator Comparison
Generates publication-quality charts comparing glibc, jemalloc, and mimalloc.
"""

import os
import csv
import json
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker

RAW_DIR = "docs/audits/stage_8e_allocator_investigation/raw_data"
CHARTS_DIR = "docs/audits/stage_8e_allocator_investigation/charts"
os.makedirs(CHARTS_DIR, exist_ok=True)

# Set Gruvbox / clean dark style
plt.style.use('dark_background')
plt.rcParams['font.sans-serif'] = 'DejaVu Sans'
plt.rcParams['font.family'] = 'sans-serif'
plt.rcParams['figure.autolayout'] = True

COLORS = {
    "glibc": "#fb4934",     # Red
    "jemalloc": "#b8bb26",  # Green
    "mimalloc": "#fabd2f",  # Yellow/Gold
}

def load_csv_timeline(filepath):
    times, rss, anon, pss = [], [], [], []
    with open(filepath, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            t = float(row['elapsed_sec'])
            r = float(row['rss_kib']) / 1024.0
            a = float(row['anonymous_kib']) / 1024.0
            p = float(row['pss_kib']) / 1024.0
            times.append(t)
            rss.append(r)
            anon.append(a)
            pss.append(p)
    return times, rss, anon, pss

def plot_timeline_test_b_d():
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(12, 9), sharex=True)

    for var in ["glibc", "jemalloc", "mimalloc"]:
        csv_file = os.path.join(RAW_DIR, f"{var}_test_b_d_session.csv")
        if os.path.exists(csv_file):
            t, r, a, p = load_csv_timeline(csv_file)
            ax1.plot(t, r, label=f"{var.upper()} RSS", color=COLORS[var], linewidth=2.2)
            ax2.plot(t, a, label=f"{var.upper()} Anonymous", color=COLORS[var], linewidth=2.2)

    # Annotations
    ax1.axvline(15, color='#83a598', linestyle='--', alpha=0.6, label='Book Opened')
    ax1.axvline(35, color='#fe8019', linestyle='--', alpha=0.6, label='Reader Closed')
    ax2.axvline(15, color='#83a598', linestyle='--', alpha=0.6)
    ax2.axvline(35, color='#fe8019', linestyle='--', alpha=0.6)

    ax1.set_title("Test B & D — Memory Timeline: Open, Read, Close & 180s Post-Close Settle", fontsize=14, fontweight='bold', pad=12)
    ax1.set_ylabel("Resident Set Size (MiB)", fontsize=12)
    ax1.grid(True, linestyle=':', alpha=0.4)
    ax1.legend(loc='upper right', framealpha=0.8)

    ax2.set_xlabel("Elapsed Time (seconds)", fontsize=12)
    ax2.set_ylabel("Anonymous Memory (MiB)", fontsize=12)
    ax2.grid(True, linestyle=':', alpha=0.4)
    ax2.legend(loc='upper right', framealpha=0.8)

    out_path = os.path.join(CHARTS_DIR, "timeline_test_b_d.png")
    plt.savefig(out_path, dpi=200)
    plt.close()
    print(f"[+] Saved {out_path}")

def plot_cycles_test_c():
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(13, 6))

    cycles = [1, 2, 3, 4]
    # Data recorded from test runs
    peak_data = {
        "glibc": [614.9, 657.5, 635.8, 647.7],
        "jemalloc": [359.7, 353.3, 331.2, 339.0],
        "mimalloc": [392.2, 595.9, 620.3, 649.9],
    }
    closed_data = {
        "glibc": [614.9, 657.5, 635.8, 580.2],
        "jemalloc": [357.0, 345.3, 320.5, 329.8],
        "mimalloc": [377.8, 410.1, 438.9, 464.6],
    }

    for var in ["glibc", "jemalloc", "mimalloc"]:
        ax1.plot(cycles, peak_data[var], marker='o', linewidth=2.4, markersize=8, label=var.upper(), color=COLORS[var])
        ax2.plot(cycles, closed_data[var], marker='s', linewidth=2.4, markersize=8, label=var.upper(), color=COLORS[var])

    ax1.set_title("Test C: Peak Reading RSS across Repeated Cycles", fontsize=12, fontweight='bold', pad=10)
    ax1.set_xlabel("Reading Cycle", fontsize=11)
    ax1.set_ylabel("Peak RSS (MiB)", fontsize=11)
    ax1.set_xticks(cycles)
    ax1.grid(True, linestyle=':', alpha=0.4)
    ax1.legend(loc='best', framealpha=0.8)

    ax2.set_title("Test C: Settled Post-Close RSS across Repeated Cycles", fontsize=12, fontweight='bold', pad=10)
    ax2.set_xlabel("Reading Cycle", fontsize=11)
    ax2.set_ylabel("Closed Settled RSS (MiB)", fontsize=11)
    ax2.set_xticks(cycles)
    ax2.grid(True, linestyle=':', alpha=0.4)
    ax2.legend(loc='best', framealpha=0.8)

    out_path = os.path.join(CHARTS_DIR, "cycles_test_c.png")
    plt.savefig(out_path, dpi=200)
    plt.close()
    print(f"[+] Saved {out_path}")

def plot_comparative_summary():
    fig, ((ax1, ax2), (ax3, ax4)) = plt.subplots(2, 2, figsize=(14, 10))

    variants = ["GLIBC", "JEMALLOC", "MIMALLOC"]
    colors = [COLORS["glibc"], COLORS["jemalloc"], COLORS["mimalloc"]]

    # 1. Peak Reading RSS
    peak_rss = [640.4, 359.8, 387.7]
    peak_anon = [511.5, 229.9, 258.6]
    x = range(len(variants))
    width = 0.35
    ax1.bar([p - width/2 for p in x], peak_rss, width, label='Peak RSS', color='#83a598')
    ax1.bar([p + width/2 for p in x], peak_anon, width, label='Peak Anonymous', color='#d3869b')
    ax1.set_title("Peak Reading Memory Consumption", fontsize=12, fontweight='bold')
    ax1.set_ylabel("Memory (MiB)", fontsize=11)
    ax1.set_xticks(x)
    ax1.set_xticklabels(variants, fontweight='bold')
    ax1.grid(True, linestyle=':', alpha=0.4)
    ax1.legend(loc='upper right')

    # 2. Retained Post-Close Memory (Test B, 180s)
    post_rss = [640.5, 344.5, 393.8]
    post_anon = [511.5, 214.6, 264.7]
    ax2.bar([p - width/2 for p in x], post_rss, width, label='Retained RSS (180s)', color='#fb4934')
    ax2.bar([p + width/2 for p in x], post_anon, width, label='Retained Anonymous', color='#fabd2f')
    ax2.set_title("Post-Close Retained Memory (180s Settled)", fontsize=12, fontweight='bold')
    ax2.set_ylabel("Memory (MiB)", fontsize=11)
    ax2.set_xticks(x)
    ax2.set_xticklabels(variants, fontweight='bold')
    ax2.grid(True, linestyle=':', alpha=0.4)
    ax2.legend(loc='upper right')

    # 3. Navigation Latency (Test G)
    lat_avg = [164.7, 162.7, 173.7]
    lat_p95 = [172.4, 175.1, 180.2]
    ax3.bar([p - width/2 for p in x], lat_avg, width, label='Average Latency', color='#8ec07c')
    ax3.bar([p + width/2 for p in x], lat_p95, width, label='P95 Latency', color='#b8bb26')
    ax3.set_title("Page Turn & Navigation Latency", fontsize=12, fontweight='bold')
    ax3.set_ylabel("Latency (milliseconds)", fontsize=11)
    ax3.set_xticks(x)
    ax3.set_xticklabels(variants, fontweight='bold')
    ax3.set_ylim(0, 220)
    ax3.grid(True, linestyle=':', alpha=0.4)
    ax3.legend(loc='upper right')

    # 4. Net Memory Retained Above Baseline (Post-Close vs Baseline)
    base_rss = [176.6, 190.8, 219.5]
    net_retained = [post_rss[i] - base_rss[i] for i in range(3)]
    bars = ax4.bar(variants, net_retained, color=colors, width=0.5)
    for bar in bars:
        yval = bar.get_height()
        ax4.text(bar.get_x() + bar.get_width()/2.0, yval + 10, f"+{yval:.1f} MiB", ha='center', va='bottom', fontweight='bold', fontsize=11)
    ax4.set_title("Net Memory Growth Over Baseline (Post-Close)", fontsize=12, fontweight='bold')
    ax4.set_ylabel("Net Retained RSS (MiB)", fontsize=11)
    ax4.set_ylim(0, 520)
    ax4.grid(True, linestyle=':', alpha=0.4)

    out_path = os.path.join(CHARTS_DIR, "comparative_summary.png")
    plt.savefig(out_path, dpi=200)
    plt.close()
    print(f"[+] Saved {out_path}")

def main():
    plot_timeline_test_b_d()
    plot_cycles_test_c()
    plot_comparative_summary()
    print("[+] All Stage 8E charts generated successfully!")

if __name__ == "__main__":
    main()
