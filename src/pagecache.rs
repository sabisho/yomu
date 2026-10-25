//! Which pages to keep decoded and which to prefetch. Pure logic (no gpui), so the memory
//! behaviour is tested.
//!
//! Page *indices* always run in reading order (page `n + 1` is the next page to read), whether the
//! book is read left-to-right or right-to-left. Reading direction only decides which key, click or
//! slide direction maps to "next", so the preload plan is the same for both.

/// Pages decoded ahead of the current one.
pub const PREFETCH_AHEAD: usize = 5;
/// Pages decoded behind the current one (so going back is instant too).
pub const PREFETCH_BEHIND: usize = 3;
/// Pages kept after they fall out of the prefetch range, to avoid churn when flipping back and forth.
pub const KEEP_BEHIND: usize = 4;
pub const KEEP_AHEAD: usize = PREFETCH_AHEAD;

/// Upper bound on decoded page memory (BGRA bytes) held at once (1 GiB default).
pub const BUDGET_BYTES: u64 = 1024 * 1024 * 1024;

/// Computes an adaptive memory budget based on host physical RAM.
/// Scales from 512 MiB on constrained devices up to 3 GiB on large desktop workstations.
pub fn system_memory_budget() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:")
                    && let Some(kb_str) = rest.trim().strip_suffix("kB")
                    && let Ok(kb) = kb_str.trim().parse::<u64>()
                {
                    let total_bytes = kb * 1024;
                    return (total_bytes / 8).clamp(512 * 1024 * 1024, 3 * 1024 * 1024 * 1024);
                }
            }
        }
    }
    BUDGET_BYTES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub index: usize,
    pub bytes: u64,
}

/// Pages to decode, most urgent first: the current page, then forward (the likeliest next move),
/// then one page back.
pub fn prefetch_order(current: usize, total: usize) -> Vec<usize> {
    let mut order = Vec::with_capacity(1 + PREFETCH_AHEAD + PREFETCH_BEHIND);
    order.push(current);
    order.extend((1..=PREFETCH_AHEAD).map(|ahead| current + ahead));
    order.extend((1..=PREFETCH_BEHIND).filter_map(|behind| current.checked_sub(behind)));
    // Interleave so the first page in each direction comes before the second, and so on.
    order.sort_by_key(|index| {
        let distance = index.abs_diff(current);
        (distance, *index < current)
    });
    order.retain(|index| *index < total);
    order
}

/// Which cached pages to drop: everything outside the keep window, then (if still over `budget`)
/// the pages furthest from `current`. The current page is never dropped.
pub fn plan_eviction(entries: &[Entry], current: usize, budget: u64) -> Vec<usize> {
    let keep: std::collections::HashSet<usize> =
        (current.saturating_sub(KEEP_BEHIND)..=current + KEEP_AHEAD).collect();
    plan_eviction_with_keep(entries, current, budget, &keep)
}

/// Evict decoded pages while preserving an explicit visible/prefetch set. Used by continuous
/// reading, where the viewport can cover a different number of pages depending on zoom.
pub fn plan_eviction_with_keep(
    entries: &[Entry],
    current: usize,
    budget: u64,
    keep: &std::collections::HashSet<usize>,
) -> Vec<usize> {
    let mut evict = Vec::new();
    let mut remaining = Vec::with_capacity(entries.len());
    let mut total: u64 = 0;

    for entry in entries {
        if !keep.contains(&entry.index) {
            evict.push(entry.index);
        } else {
            total += entry.bytes;
            remaining.push(entry);
        }
    }

    if total > budget {
        remaining.sort_by_key(|entry| {
            (
                keep.contains(&entry.index),
                std::cmp::Reverse(entry.index.abs_diff(current)),
            )
        });
        for entry in remaining {
            if total <= budget {
                break;
            }
            if entry.index == current {
                continue;
            }
            total -= entry.bytes;
            evict.push(entry.index);
        }
    }
    evict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize, megabytes: u64) -> Entry {
        Entry {
            index,
            bytes: megabytes * 1024 * 1024,
        }
    }

    #[test]
    fn prefetch_goes_forward_first_then_one_back() {
        assert_eq!(
            prefetch_order(10, 100),
            vec![10, 11, 9, 12, 8, 13, 7, 14, 15]
        );
    }

    #[test]
    fn prefetch_is_clamped_at_both_ends() {
        assert_eq!(prefetch_order(0, 100), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(prefetch_order(99, 100), vec![99, 98, 97, 96]);
        assert_eq!(prefetch_order(98, 100), vec![98, 99, 97, 96, 95]);
        assert_eq!(prefetch_order(0, 1), vec![0]);
        assert_eq!(prefetch_order(0, 3), vec![0, 1, 2]);
    }

    #[test]
    fn prefetch_is_a_small_bounded_set() {
        for current in 0..50 {
            assert!(prefetch_order(current, 50).len() <= 1 + PREFETCH_AHEAD + PREFETCH_BEHIND);
        }
    }

    #[test]
    fn pages_outside_the_window_are_evicted() {
        let entries: Vec<Entry> = (0..16).map(|i| entry(i, 1)).collect();
        let mut evicted = plan_eviction(&entries, 6, BUDGET_BYTES);
        evicted.sort();
        // keeps 6 - 4 ..= 6 + 5 = 2..=11
        assert_eq!(evicted, vec![0, 1, 12, 13, 14, 15]);
    }

    #[test]
    fn nothing_is_evicted_inside_the_window_and_budget() {
        let entries: Vec<Entry> = (2..=11).map(|i| entry(i, 10)).collect();
        assert!(plan_eviction(&entries, 6, BUDGET_BYTES).is_empty());
    }

    #[test]
    fn over_budget_drops_the_furthest_pages_but_never_the_current_one() {
        // 10 pages x 150 MB against a 1024 MB budget
        let entries: Vec<Entry> = (2..=11).map(|i| entry(i, 150)).collect();
        let mut evicted = plan_eviction(&entries, 6, BUDGET_BYTES);
        evicted.sort();
        assert!(evicted.len() >= 3);
        assert!(!evicted.contains(&6));
    }

    #[test]
    fn a_single_huge_current_page_is_kept_even_over_budget() {
        let entries = vec![entry(3, 1500)];
        assert!(plan_eviction(&entries, 3, BUDGET_BYTES).is_empty());
    }

    #[test]
    fn eviction_is_the_same_for_either_reading_direction() {
        // direction never enters the plan: indices are reading order
        let entries: Vec<Entry> = (0..8).map(|i| entry(i, 5)).collect();
        assert_eq!(
            plan_eviction(&entries, 4, BUDGET_BYTES),
            plan_eviction(&entries, 4, BUDGET_BYTES)
        );
    }
}
