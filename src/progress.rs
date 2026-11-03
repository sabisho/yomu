#![forbid(unsafe_code)]

//! Reading progress persistence and sidecar management.
//!
//! Separates high-frequency reading progress updates (current page, opened state, last read timestamp)
//! from static library collection metadata, eliminating full-library serialization and SSD write
//! amplification on page turns.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::AtomicU64;

use crate::library::{DataDirs, Library};

/// Current schema version for `progress.json`.
pub const PROGRESS_VERSION: u32 = 1;

/// Compact reading progress record for a single book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookProgress {
    /// Zero-based index of the page the reader was last on.
    #[serde(default)]
    pub current_page: usize,
    /// Whether the book has ever been opened.
    #[serde(default)]
    pub opened: bool,
    /// Unix timestamp in seconds when the book was last read.
    #[serde(default)]
    pub last_read: Option<u64>,
}

/// Bounded collection of reading progress records, stored in `progress.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: BTreeMap<String, BookProgress>,
    #[serde(skip)]
    pub mutation_seq: u64,
}

impl Default for ProgressStore {
    fn default() -> Self {
        Self {
            version: PROGRESS_VERSION,
            entries: BTreeMap::new(),
            mutation_seq: 0,
        }
    }
}

impl ProgressStore {
    /// Create a new, empty progress store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only access to progress records.
    pub fn entries(&self) -> &BTreeMap<String, BookProgress> {
        &self.entries
    }

    /// Number of books with recorded progress.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether any progress records are stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Monotonically increasing mutation sequence for dirty tracking.
    pub fn mutation_seq(&self) -> u64 {
        self.mutation_seq
    }

    /// Look up the reading progress for a book by its identifier.
    pub fn get(&self, book_id: &str) -> Option<&BookProgress> {
        self.entries.get(book_id)
    }

    /// Update reading progress for a book.
    ///
    /// Returns `true` if the recorded progress changed (dirty), or `false`
    /// if the existing record was already identical (no-op).
    pub fn set(
        &mut self,
        book_id: &str,
        current_page: usize,
        opened: bool,
        last_read: Option<u64>,
    ) -> bool {
        let new_progress = BookProgress {
            current_page,
            opened,
            last_read,
        };

        if self.entries.get(book_id) == Some(&new_progress) {
            return false;
        }

        self.entries.insert(book_id.to_string(), new_progress);
        self.mutation_seq = self.mutation_seq.wrapping_add(1);
        true
    }

    /// Remove progress for a book (e.g. when removed from the library).
    pub fn remove(&mut self, book_id: &str) -> bool {
        if self.entries.remove(book_id).is_some() {
            self.mutation_seq = self.mutation_seq.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// Reset all progress entries (e.g. clear all data).
    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.mutation_seq = self.mutation_seq.wrapping_add(1);
        }
    }

    /// Populate progress records from a `Library` snapshot (used for migration from `library.json`).
    pub fn harvest_from_library(&mut self, library: &Library) -> usize {
        let mut count = 0;
        for book in library.books() {
            if book.opened || book.current_page > 0 || book.last_read.is_some() {
                self.entries.insert(
                    book.id.clone(),
                    BookProgress {
                        current_page: book.current_page,
                        opened: book.opened,
                        last_read: book.last_read,
                    },
                );
                count += 1;
            }
        }
        if count > 0 {
            self.mutation_seq = self.mutation_seq.wrapping_add(1);
        }
        count
    }

    /// Overlay all tracked progress records onto the books in `library`.
    pub fn apply_to_library(&self, library: &mut Library) {
        for (id, progress) in &self.entries {
            library.update_progress(
                id,
                progress.current_page,
                progress.opened,
                progress.last_read,
            );
        }
    }

    /// Load `progress.json` from disk.
    ///
    /// If the file does not exist, returns an empty `ProgressStore`.
    /// If corrupted, renames the corrupted file to `.corrupt` and returns an error.
    pub fn load(dirs: &DataDirs) -> Result<Self> {
        let file = dirs.progress_file();

        // Clean up leftover temporary files from prior crashes or ungraceful shutdowns
        if let Some(parent) = file.parent()
            && let Ok(entries) = fs::read_dir(parent)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str())
                    && (name.starts_with("progress.tmp.") || name == "progress.json.tmp")
                {
                    let _ = fs::remove_file(p);
                }
            }
        }

        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", file.display())),
        };

        match serde_json::from_str::<ProgressStore>(&text) {
            Ok(store) => Ok(store),
            Err(err) => {
                // Back up corrupted progress file instead of silently clobbering it
                let backup = file.with_extension("json.corrupt");
                let _ = fs::rename(&file, &backup);
                Err(err).with_context(|| {
                    format!(
                        "progress file was corrupted and moved to {}; starting with fresh progress",
                        backup.display()
                    )
                })
            }
        }
    }

    /// Write `progress.json` atomically using temp file + fsync + rename + parent dir sync.
    pub fn save(&self, dirs: &DataDirs) -> Result<()> {
        use std::io::Write as _;

        let file = dirs.progress_file();
        let parent = file
            .parent()
            .context("progress file destination has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;

        // 1. Serialize in-memory before touching disk
        let json = serde_json::to_vec_pretty(self)?;

        // 2. Unique temporary file in the same directory (same filesystem mount)
        static TMP_SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = TMP_SAVE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = parent.join(format!(
            "progress.tmp.{}.{}.{}",
            std::process::id(),
            nanos,
            counter
        ));

        // 3. Write and sync temporary file contents
        let write_and_sync = || -> Result<()> {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .with_context(|| format!("opening temporary progress file {}", tmp.display()))?;

            f.write_all(&json)
                .with_context(|| format!("writing to {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("synchronizing {}", tmp.display()))?;
            Ok(())
        };

        if let Err(err) = write_and_sync() {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }

        // 4. Atomically replace destination with bounded retry for transient locks (antivirus/indexing)
        if let Err(err) = crate::library::atomic_replace(&tmp, &file) {
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| format!("replacing {}", file.display()));
        }

        // 5. Synchronize parent directory where supported (Unix/Linux/macOS)
        #[cfg(unix)]
        {
            if let Ok(dir_file) = fs::File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_store_lifecycle_and_dirty_checking() {
        let mut store = ProgressStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
        assert_eq!(store.mutation_seq(), 0);

        // Setting a new book progress returns true (modified)
        assert!(store.set("book_1", 5, true, Some(100)));
        assert_eq!(store.len(), 1);
        assert_eq!(store.mutation_seq(), 1);
        assert_eq!(
            store.get("book_1"),
            Some(&BookProgress {
                current_page: 5,
                opened: true,
                last_read: Some(100),
            })
        );

        // Setting the exact same progress returns false (unchanged, zero dirty mutation)
        assert!(!store.set("book_1", 5, true, Some(100)));
        assert_eq!(store.mutation_seq(), 1);

        // Updating progress returns true
        assert!(store.set("book_1", 6, true, Some(105)));
        assert_eq!(store.mutation_seq(), 2);

        // Removing book returns true
        assert!(store.remove("book_1"));
        assert_eq!(store.len(), 0);
        assert_eq!(store.mutation_seq(), 3);
        assert!(!store.remove("book_1"));
    }

    #[test]
    fn test_first_time_progress_creation() {
        let tmp = std::env::temp_dir().join(format!("yomu-progress-test-1-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let progress_file = dirs.progress_file();
        assert!(!progress_file.exists());

        let mut store = ProgressStore::new();
        store.set("manga_alpha", 12, true, Some(1700000000));
        store.save(&dirs).expect("first-time save must succeed");
        assert!(progress_file.exists());

        let loaded = ProgressStore::load(&dirs).expect("load should parse progress.json");
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.get("manga_alpha"),
            Some(&BookProgress {
                current_page: 12,
                opened: true,
                last_read: Some(1700000000),
            })
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_replacing_existing_progress_file() {
        let tmp = std::env::temp_dir().join(format!("yomu-progress-test-2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let progress_file = dirs.progress_file();

        let mut store = ProgressStore::new();
        store.set("manga_beta", 1, true, Some(100));
        store.save(&dirs).unwrap();

        // Mutate and replace
        store.set("manga_beta", 25, true, Some(200));
        store.save(&dirs).expect("replace save must succeed");

        let loaded = ProgressStore::load(&dirs).unwrap();
        assert_eq!(loaded.get("manga_beta").unwrap().current_page, 25);

        // Ensure no temporary files remained
        let parent = progress_file.parent().unwrap();
        let tmp_files: Vec<_> = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("progress.tmp."))
            .collect();
        assert!(
            tmp_files.is_empty(),
            "temporary progress files must be cleaned up"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_progress_cleanup_stale_temporary_files_on_load() {
        let tmp = std::env::temp_dir().join(format!("yomu-progress-test-3-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let progress_file = dirs.progress_file();
        let parent = progress_file.parent().unwrap();
        fs::create_dir_all(parent).unwrap();

        // Simulate leftover temp files from prior interrupted processes
        let stale1 = parent.join("progress.tmp.1234.5678.9");
        let stale2 = parent.join("progress.json.tmp");
        fs::write(&stale1, b"abandoned write 1").unwrap();
        fs::write(&stale2, b"abandoned write 2").unwrap();
        assert!(stale1.exists());
        assert!(stale2.exists());

        // Save a valid progress store
        let mut store = ProgressStore::new();
        store.set("book_gamma", 42, true, None);
        store.save(&dirs).unwrap();

        // Loading should clean up both stale temp files
        let loaded = ProgressStore::load(&dirs).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(
            !stale1.exists(),
            "stale temp file 1 must be cleaned up on load"
        );
        assert!(
            !stale2.exists(),
            "stale temp file 2 must be cleaned up on load"
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
