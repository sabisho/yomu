#!/usr/bin/env python3
"""
Generate publication-quality comparative charts for Stage 8F: Production Jemalloc Integration.
"""

import os
import csv
import json
import matplotlib.pyplot as plt

AUDIT_DIR = os.path.dirname(os.path.abspath(__file__))
RAW_8F = os.path.join(AUDIT_DIR, "raw_data")
RAW_8E = os.path.join(os.path.dirname(AUDIT_DIR), "stage_8e_allocator_investigation", "raw_data")
CHARTS_DIR = os.path.join(AUDIT_DIR, "charts")

os.makedirs(CHARTS_DIR, exist_ok=True)

plt.style.use('seaborn-v0_8-whitegrid' if 'seaborn-v0_8-whitegrid' in plt.style.available else 'default')
plt.rcParams['font.sans-serif'] = 'DejaVu Sans'
plt.rcParams['font.size'] = 10

def load_csv_timeline(filepath):
    times, rss, anon, pss = [], [], [], []
    with open(filepath, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            times.append(float(row['elapsed_sec']))
            rss.append(float(row.get('rss_kib', row.get('rss_kb', 0))) / 1024.0)
            anon.append(float(row.get('anonymous_kib', row.get('anon_kb', 0))) / 1024.0)
            pss.append(float(row.get('pss_kib', row.get('pss_kb', 0))) / 1024.0)
    return times, rss, anon, pss

def plot_timeline_comparison():
    glibc_csv = os.path.join(RAW_8E, "glibc_test_b_d_session.csv")
    prod_csv = os.path.join(RAW_8F, "prod_jemalloc_test_b_d_session.csv")

    fig, ax = plt.subplots(figsize=(11, 6), dpi=150)

    if os.path.exists(glibc_csv):
        t_g, rss_g, anon_g, _ = load_csv_timeline(glibc_csv)
        ax.plot(t_g, rss_g, label='glibc Baseline (RSS)', color='#d65d0e', linewidth=2.2)
        ax.plot(t_g, anon_g, label='glibc Baseline (Anonymous)', color='#d65d0e', linestyle='--', alpha=0.7)

    if os.path.exists(prod_csv):
        t_j, rss_j, anon_j, _ = load_csv_timeline(prod_csv)
        ax.plot(t_j, rss_j, label='Production jemalloc (RSS)', color='#458588', linewidth=2.4)
        ax.plot(t_j, anon_j, label='Production jemalloc (Anonymous)', color='#458588', linestyle='--', alpha=0.7)

    ax.set_title('Stage 8F: Reading Lifecycle & Post-Close Retention (glibc vs Production jemalloc)', fontsize=13, fontweight='bold', pad=12)
    ax.set_xlabel('Elapsed Time (seconds)', fontsize=11)
    ax.set_ylabel('Memory (MiB)', fontsize=11)
    ax.axvspan(6, 25, color='#fabd2f', alpha=0.15, label='Active Reading Window')
    ax.axvline(25, color='#cc241d', linestyle=':', label='Reader Closed')
    ax.legend(loc='center right', frameon=True)
    ax.set_ylim(0, 750)
    plt.tight_layout()
    out_path = os.path.join(CHARTS_DIR, "timeline_prod_vs_glibc.png")
    fig.savefig(out_path)
    plt.close()
    print(f"[+] Saved {out_path}")

def plot_cycles_comparison():
    glibc_csv = os.path.join(RAW_8E, "glibc_test_c_repeated_cycles.csv")
    prod_csv = os.path.join(RAW_8F, "prod_jemalloc_test_c_repeated_cycles.csv")

    fig, ax = plt.subplots(figsize=(11, 5.5), dpi=150)

    if os.path.exists(glibc_csv):
        t_g, rss_g, _, _ = load_csv_timeline(glibc_csv)
        ax.plot(t_g, rss_g, label='glibc Baseline (4 Cycles)', color='#d65d0e', linewidth=2.2)

    if os.path.exists(prod_csv):
        t_j, rss_j, _, _ = load_csv_timeline(prod_csv)
        ax.plot(t_j, rss_j, label='Production jemalloc (5 Cycles)', color='#458588', linewidth=2.4)

    ax.set_title('Stage 8F: Repeated Reading Cycles Memory Stability', fontsize=13, fontweight='bold', pad=12)
    ax.set_xlabel('Elapsed Time (seconds)', fontsize=11)
    ax.set_ylabel('RSS Memory (MiB)', fontsize=11)
    ax.legend(loc='lower right', frameon=True)
    ax.set_ylim(100, 750)
    plt.tight_layout()
    out_path = os.path.join(CHARTS_DIR, "cycles_prod_vs_glibc.png")
    fig.savefig(out_path)
    plt.close()
    print(f"[+] Saved {out_path}")

def plot_summary_barchart():
    fig, ((ax1, ax2), (ax3, ax4)) = plt.subplots(2, 2, figsize=(11, 8.5), dpi=150)

    # 1. Peak Reading RSS
    cats = ['glibc Baseline', 'Production jemalloc']
    colors = ['#d65d0e', '#458588']
    peaks = [640.4, 484.9]
    ax1.bar(cats, peaks, color=colors, width=0.5)
    ax1.set_title('Peak Reading RSS (MiB)', fontweight='bold')
    ax1.set_ylabel('MiB')
    ax1.set_ylim(0, 750)
    for i, v in enumerate(peaks):
        ax1.text(i, v + 15, f"{v:.1f}M", ha='center', fontweight='bold')

    # 2. Post-Close Retention RSS
    post_close = [640.5, 473.5]
    ax2.bar(cats, post_close, color=colors, width=0.5)
    ax2.set_title('Post-Close Settled RSS (MiB)', fontweight='bold')
    ax2.set_ylabel('MiB')
    ax2.set_ylim(0, 750)
    for i, v in enumerate(post_close):
        ax2.text(i, v + 15, f"{v:.1f}M", ha='center', fontweight='bold')

    # 3. Direct Switch Settled RSS
    switch_res = [559.0, 423.5]
    ax3.bar(cats, switch_res, color=colors, width=0.5)
    ax3.set_title('Direct Book Switch RSS (MiB)', fontweight='bold')
    ax3.set_ylabel('MiB')
    ax3.set_ylim(0, 650)
    for i, v in enumerate(switch_res):
        ax3.text(i, v + 15, f"{v:.1f}M", ha='center', fontweight='bold')

    # 4. Navigation Latency
    lats = [164.7, 164.6]
    ax4.bar(cats, lats, color=colors, width=0.5)
    ax4.set_title('Avg Navigation Latency (ms)', fontweight='bold')
    ax4.set_ylabel('Milliseconds')
    ax4.set_ylim(0, 220)
    for i, v in enumerate(lats):
        ax4.text(i, v + 5, f"{v:.1f} ms", ha='center', fontweight='bold')

    plt.suptitle('Stage 8F Production jemalloc Integration: Comparative Summary', fontsize=14, fontweight='bold', y=0.98)
    plt.tight_layout()
    out_path = os.path.join(CHARTS_DIR, "stage_8f_comparative_summary.png")
    fig.savefig(out_path)
    plt.close()
    print(f"[+] Saved {out_path}")

if __name__ == '__main__':
    plot_timeline_comparison()
    plot_cycles_comparison()
    plot_summary_barchart()
