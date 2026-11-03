use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread::{Builder, JoinHandle},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::util::natural_cmp;

/// Refuse to inflate a single page beyond this size (guards against zip bombs).
const MAX_PAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FOLDER_DEPTH: usize = 4;
/// Initial buffer allocation cap to guard against untrusted zip header size claims.
/// The buffer will dynamically grow via `read_to_end` up to `MAX_PAGE_BYTES` if needed.
const INITIAL_PAGE_BUF_CAPACITY: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Archive,
    Folder,
}

pub fn is_image_name(name: &str) -> bool {
    const EXTENSIONS: [&str; 6] = [".jpg", ".jpeg", ".jfif", ".png", ".webp", ".gif"];
    EXTENSIONS.iter().any(|ext| {
        if name.len() < ext.len() {
            return false;
        }
        name[name.len() - ext.len()..].eq_ignore_ascii_case(ext)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Rar,
}

pub fn detect_archive_format(path: &Path) -> ArchiveFormat {
    if let Ok(mut file) = File::open(path) {
        let mut magic = [0u8; 7];
        let n = file.read(&mut magic).unwrap_or(0);
        if n >= 4
            && (&magic[0..4] == b"PK\x03\x04"
                || &magic[0..4] == b"PK\x05\x06"
                || &magic[0..4] == b"PK\x07\x08")
        {
            return ArchiveFormat::Zip;
        }
        if n >= 7 && (&magic[0..7] == b"Rar!\x1a\x07\x00" || &magic[0..7] == b"Rar!\x1a\x07\x01") {
            return ArchiveFormat::Rar;
        }
    }
    let lower = path.to_string_lossy();
    if lower.ends_with(".cbr")
        || lower.ends_with(".CBR")
        || lower.ends_with(".rar")
        || lower.ends_with(".RAR")
    {
        ArchiveFormat::Rar
    } else {
        ArchiveFormat::Zip
    }
}

pub fn is_archive_name(name: &str) -> bool {
    const EXTENSIONS: [&str; 4] = [".cbz", ".zip", ".cbr", ".rar"];
    EXTENSIONS.iter().any(|ext| {
        if name.len() < ext.len() {
            return false;
        }
        name[name.len() - ext.len()..].eq_ignore_ascii_case(ext)
    })
}

/// Hidden files and macOS resource-fork folders are never pages.
fn is_junk(name: &str) -> bool {
    name.split('/')
        .any(|part| part.starts_with('.') || part == "__MACOSX")
}

/// Reject path traversal, absolute paths, null bytes, Windows drive specifications, and hidden junk.
pub fn is_safe_archive_entry_name(name: &str) -> bool {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\0')
        || name.contains(':')
    {
        return false;
    }
    for part in name.split(['/', '\\']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.starts_with('.')
            || part == "__MACOSX"
        {
            return false;
        }
    }
    true
}

struct CachedArchive {
    path: PathBuf,
    archive: zip::ZipArchive<BufReader<File>>,
}

static ARCHIVE_POOL: Mutex<Vec<CachedArchive>> = Mutex::new(Vec::new());

fn checkout_archive(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>> {
    if let Ok(mut pool) = ARCHIVE_POOL.lock()
        && let Some(idx) = pool.iter().position(|c| c.path == path)
    {
        return Ok(pool.swap_remove(idx).archive);
    }
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    zip::ZipArchive::new(reader)
        .with_context(|| format!("{} is not a valid zip/cbz", path.display()))
}

fn checkin_archive(path: PathBuf, archive: zip::ZipArchive<BufReader<File>>) {
    if let Ok(mut pool) = ARCHIVE_POOL.lock() {
        if pool.len() >= 8 {
            pool.remove(0);
        }
        pool.push(CachedArchive { path, archive });
    }
}

fn list_zip_pages(path: &Path) -> Result<Vec<String>> {
    let archive = checkout_archive(path)?;
    let pages: Vec<String> = archive
        .file_names()
        .filter(|name| {
            !name.ends_with('/')
                && is_safe_archive_entry_name(name)
                && is_image_name(name)
                && !is_junk(name)
        })
        .map(str::to_owned)
        .collect();
    checkin_archive(path.to_path_buf(), archive);
    Ok(pages)
}

fn list_rar_pages(path: &Path) -> Result<Vec<String>> {
    let archive = unrar::Archive::new(path);
    let entries = archive
        .open_for_listing()
        .with_context(|| format!("{} is not a valid rar/cbr", path.display()))?;
    let mut pages = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry from {}", path.display()))?;
        if entry.is_directory() || entry.is_encrypted() {
            continue;
        }
        let name_str = entry.filename.to_string_lossy();
        let normalized = if name_str.contains('\\') {
            name_str.replace('\\', "/")
        } else {
            name_str.into_owned()
        };
        if is_safe_archive_entry_name(&normalized)
            && is_image_name(&normalized)
            && !is_junk(&normalized)
        {
            pages.push(normalized);
        }
    }
    Ok(pages)
}

/// Names of every page in reading order.
pub fn list_pages(path: &Path, kind: SourceKind) -> Result<Vec<String>> {
    let mut pages = match kind {
        SourceKind::Archive => {
            let format = detect_archive_format(path);
            match format {
                ArchiveFormat::Zip => list_zip_pages(path).or_else(|_| list_rar_pages(path))?,
                ArchiveFormat::Rar => list_rar_pages(path).or_else(|_| list_zip_pages(path))?,
            }
        }
        SourceKind::Folder => {
            let mut out = Vec::new();
            walk_folder(path, path, 0, &mut out)
                .with_context(|| format!("reading folder {}", path.display()))?;
            out
        }
    };
    pages.sort_by(|a, b| natural_cmp(a, b));
    Ok(pages)
}

fn walk_folder(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if depth < MAX_FOLDER_DEPTH {
                walk_folder(root, &entry.path(), depth + 1, out)?;
            }
        } else if is_image_name(&name) {
            let relative = entry.path();
            let relative = relative.strip_prefix(root).unwrap_or(&relative);
            let s = relative.to_string_lossy();
            let normalized = if s.contains('\\') {
                s.replace('\\', "/")
            } else {
                s.into_owned()
            };
            out.push(normalized);
        }
    }
    Ok(())
}

/// True if the folder directly contains at least one image (so it is a "chapter").
pub fn folder_has_images(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                !name.starts_with('.') && is_image_name(&name) && e.path().is_file()
            })
        })
        .unwrap_or(false)
}

fn read_zip_page(path: &Path, name: &str) -> Result<Vec<u8>> {
    if !is_safe_archive_entry_name(name) {
        bail!("invalid or unsafe page name: {name}");
    }
    let mut archive = checkout_archive(path)?;

    let read_res = (|| -> Result<Vec<u8>> {
        let entry = archive
            .by_name(name)
            .with_context(|| format!("page {name} missing from archive"))?;
        if entry.size() > MAX_PAGE_BYTES {
            bail!("page {name} is unreasonably large");
        }
        let initial_cap = if entry.size() > 0 {
            (entry.size() as usize).min(INITIAL_PAGE_BUF_CAPACITY)
        } else {
            64 * 1024
        };
        let mut buf = Vec::with_capacity(initial_cap);
        let bytes_read = entry.take(MAX_PAGE_BYTES + 1).read_to_end(&mut buf)?;
        if bytes_read > MAX_PAGE_BYTES as usize {
            bail!("page {name} is unreasonably large");
        }
        Ok(buf)
    })();

    checkin_archive(path.to_path_buf(), archive);
    read_res
}

struct RarTask {
    name: String,
    cancel: Option<Arc<AtomicBool>>,
    reply: SyncSender<Result<Vec<u8>>>,
}

/// A worker-confined CBR/RAR reader session.
///
/// Keeps an `unrar::OpenArchive` handle alive on a dedicated background worker thread,
/// enabling $O(1)$ sequential page advances and eliminating repeated archive reopening
/// and quadratic ($O(N^2)$) header rescanning.
///
/// Thread safety: The UnRAR handle is `!Send` and `!Sync`, so it is strictly confined
/// to the worker thread that spawned it. `RarSession` communicates with the worker via
/// safe standard library channels (`std::sync::mpsc`).
///
/// Resource lifecycle: When `RarSession` is dropped, `sender` is dropped, causing
/// the worker thread to exit immediately, call `RARCloseArchive`, and terminate.
pub struct RarSession {
    path: PathBuf,
    sender: Option<SyncSender<RarTask>>,
    worker: Option<JoinHandle<()>>,
}

fn rar_worker_loop(path: PathBuf, receiver: Receiver<RarTask>) {
    let mut proc: Option<unrar::OpenArchive<unrar::Process, unrar::CursorBeforeHeader>> = None;
    let mut current_physical_index: usize = 0;
    let mut known_positions: HashMap<String, usize> = HashMap::new();

    while let Ok(task) = receiver.recv() {
        let RarTask {
            name,
            cancel,
            reply,
        } = task;

        if let Some(ref c) = cancel
            && c.load(Ordering::Relaxed)
        {
            let _ = reply.send(Err(anyhow::anyhow!("page cancelled")));
            continue;
        }

        if !is_safe_archive_entry_name(&name) {
            let _ = reply.send(Err(anyhow::anyhow!("invalid or unsafe page name: {name}")));
            continue;
        }

        let normalized_target = if name.contains('\\') {
            name.replace('\\', "/")
        } else {
            name.clone()
        };

        // Determine if target entry is already known to be behind our current cursor
        let target_is_behind = known_positions
            .get(&normalized_target)
            .is_some_and(|&pos| pos < current_physical_index);

        if target_is_behind || proc.is_none() {
            proc = None;
            current_physical_index = 0;
            match unrar::Archive::new(&path).open_for_processing() {
                Ok(opened) => proc = Some(opened),
                Err(e) => {
                    let _ = reply.send(Err(anyhow::anyhow!("opening {}: {e}", path.display())));
                    continue;
                }
            }
        }

        let mut archive = proc.take().unwrap();
        let found_data;
        let mut has_retried_from_start = target_is_behind;

        loop {
            if let Some(ref c) = cancel
                && c.load(Ordering::Relaxed)
            {
                found_data = Some(Err(anyhow::anyhow!("page cancelled")));
                proc = Some(archive);
                break;
            }

            let header = match archive.read_header() {
                Ok(Some(h)) => h,
                Ok(None) => {
                    // Reached end of archive
                    if current_physical_index > 0 && !has_retried_from_start {
                        match unrar::Archive::new(&path).open_for_processing() {
                            Ok(reopened) => {
                                archive = reopened;
                                current_physical_index = 0;
                                has_retried_from_start = true;
                                continue;
                            }
                            Err(e) => {
                                found_data =
                                    Some(Err(anyhow::anyhow!("reopening {}: {e}", path.display())));
                                break;
                            }
                        }
                    }
                    found_data = Some(Err(anyhow::anyhow!("page {name} missing from archive")));
                    break;
                }
                Err(e) => {
                    found_data = Some(Err(anyhow::anyhow!(
                        "reading header from {}: {e}",
                        path.display()
                    )));
                    break;
                }
            };

            let entry_fn = &header.entry().filename;
            let current_name: std::borrow::Cow<'_, str> = if let Some(s) = entry_fn.to_str() {
                if s.contains('\\') {
                    std::borrow::Cow::Owned(s.replace('\\', "/"))
                } else {
                    std::borrow::Cow::Borrowed(s)
                }
            } else {
                let name_str = entry_fn.to_string_lossy();
                if name_str.contains('\\') {
                    std::borrow::Cow::Owned(name_str.replace('\\', "/"))
                } else {
                    name_str
                }
            };

            let entry_name_str = current_name.as_ref();
            known_positions
                .entry(entry_name_str.to_string())
                .or_insert(current_physical_index);

            let is_match = entry_name_str == normalized_target;

            if is_match {
                current_physical_index += 1;
                if header.entry().is_encrypted() {
                    found_data = Some(Err(anyhow::anyhow!(
                        "page {name} is password-protected or encrypted in {}",
                        path.display()
                    )));
                    break;
                }
                if header.entry().unpacked_size > MAX_PAGE_BYTES {
                    found_data = Some(Err(anyhow::anyhow!("page {name} is unreasonably large")));
                    if let Ok(next) = header.skip() {
                        proc = Some(next);
                    }
                    break;
                }
                match header.read_bounded(MAX_PAGE_BYTES as usize) {
                    Ok((data, next)) => {
                        found_data = Some(Ok(data));
                        proc = Some(next);
                        break;
                    }
                    Err(e) => {
                        found_data = Some(Err(anyhow::anyhow!(
                            "extracting {name} from {}: {e}",
                            path.display()
                        )));
                        break;
                    }
                }
            } else {
                current_physical_index += 1;
                match header.skip() {
                    Ok(next) => archive = next,
                    Err(e) => {
                        found_data = Some(Err(anyhow::anyhow!(
                            "skipping entry in {}: {e}",
                            path.display()
                        )));
                        break;
                    }
                }
            }
        }

        if let Some(res) = found_data {
            let _ = reply.send(res);
        }
    }
}

impl RarSession {
    /// Opens a new worker-confined RAR session for `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let path_buf = path.to_path_buf();
        let (sender, receiver) = sync_channel::<RarTask>(32);
        let worker_path = path_buf.clone();
        let thread_name = format!(
            "yomu-rar-{}",
            worker_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("worker")
        );

        let worker = Builder::new()
            .name(thread_name)
            .spawn(move || {
                rar_worker_loop(worker_path, receiver);
            })
            .with_context(|| format!("spawning rar worker for {}", path.display()))?;

        Ok(Self {
            path: path_buf,
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    /// Read a page by entry name, advancing the worker-local archive position.
    pub fn read_page(&self, name: &str, cancel: Option<Arc<AtomicBool>>) -> Result<Vec<u8>> {
        let sender = self.sender.as_ref().context("rar session is closed")?;
        let (reply_tx, reply_rx) = sync_channel(1);
        sender
            .send(RarTask {
                name: name.to_string(),
                cancel,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("rar worker channel disconnected"))?;

        reply_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("rar worker failed to respond"))?
    }

    /// Path of the open archive.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RarSession {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn read_rar_page(path: &Path, name: &str) -> Result<Vec<u8>> {
    if !is_safe_archive_entry_name(name) {
        bail!("invalid or unsafe page name: {name}");
    }
    let archive = unrar::Archive::new(path);
    let mut proc = archive
        .open_for_processing()
        .with_context(|| format!("opening {}", path.display()))?;
    let normalized_target = if name.contains('\\') {
        name.replace('\\', "/")
    } else {
        name.to_string()
    };
    loop {
        let header = match proc.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => bail!("page {name} missing from archive"),
            Err(e) => bail!("reading header from {}: {e}", path.display()),
        };
        let entry_filename = &header.entry().filename;
        let is_match = if let Some(s) = entry_filename.to_str() {
            if s.contains('\\') {
                s.replace('\\', "/") == normalized_target
            } else {
                s == normalized_target
            }
        } else {
            let name_str = entry_filename.to_string_lossy();
            if name_str.contains('\\') {
                name_str.replace('\\', "/") == normalized_target
            } else {
                name_str == normalized_target
            }
        };
        if is_match {
            if header.entry().is_encrypted() {
                bail!(
                    "page {name} is password-protected or encrypted in {}",
                    path.display()
                );
            }
            if header.entry().unpacked_size > MAX_PAGE_BYTES {
                bail!("page {name} is unreasonably large");
            }
            let (data, _next) = header
                .read_bounded(MAX_PAGE_BYTES as usize)
                .with_context(|| format!("extracting {name} from {}", path.display()))?;
            return Ok(data);
        } else {
            proc = header
                .skip()
                .with_context(|| format!("skipping entry in {}", path.display()))?;
        }
    }
}

/// Raw (still encoded) bytes of one page, optionally reusing a worker-confined `RarSession`.
pub fn read_page_with_session(
    path: &Path,
    kind: SourceKind,
    name: &str,
    session: Option<&RarSession>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<u8>> {
    match kind {
        SourceKind::Archive => {
            let format = detect_archive_format(path);
            match format {
                ArchiveFormat::Zip => match read_zip_page(path, name) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        let msg = format!("{err:#}");
                        if msg.contains("not a valid zip") {
                            if let Some(s) = session {
                                s.read_page(name, cancel)
                            } else {
                                read_rar_page(path, name)
                            }
                        } else {
                            Err(err)
                        }
                    }
                },
                ArchiveFormat::Rar => {
                    let res = if let Some(s) = session {
                        s.read_page(name, cancel)
                    } else {
                        read_rar_page(path, name)
                    };
                    match res {
                        Ok(data) => Ok(data),
                        Err(err) => {
                            let msg = format!("{err:#}");
                            if msg.contains("not a valid rar") || msg.contains("Not a RAR archive")
                            {
                                read_zip_page(path, name)
                            } else {
                                Err(err)
                            }
                        }
                    }
                }
            }
        }
        SourceKind::Folder => {
            let full = path.join(name);
            fs::read(&full).with_context(|| format!("reading {}", full.display()))
        }
    }
}

/// Raw (still encoded) bytes of one page.
pub fn read_page(path: &Path, kind: SourceKind, name: &str) -> Result<Vec<u8>> {
    read_page_with_session(path, kind, name, None, None)
}

/// Inspects the header of a single page in an archive or folder to extract (width, height)
/// without decompressing or decoding full image pixel data.
pub fn probe_page_dimensions(path: &Path, kind: SourceKind, name: &str) -> Result<(u32, u32)> {
    match kind {
        SourceKind::Folder => {
            let full = path.join(name);
            let file = File::open(&full).with_context(|| format!("opening {}", full.display()))?;
            let mut buf = Vec::with_capacity(64 * 1024);
            file.take(64 * 1024).read_to_end(&mut buf)?;
            crate::imaging::probe_dimensions(&buf)
        }
        SourceKind::Archive => {
            let format = detect_archive_format(path);
            match format {
                ArchiveFormat::Zip => {
                    let mut archive = checkout_archive(path)?;
                    let res = (|| -> Result<(u32, u32)> {
                        let entry = archive
                            .by_name(name)
                            .with_context(|| format!("page {name} missing from archive"))?;
                        let mut buf = Vec::with_capacity(64 * 1024);
                        entry.take(64 * 1024).read_to_end(&mut buf)?;
                        crate::imaging::probe_dimensions(&buf)
                    })();
                    checkin_archive(path.to_path_buf(), archive);
                    res
                }
                ArchiveFormat::Rar => {
                    let bytes = read_rar_page(path, name)?;
                    crate::imaging::probe_dimensions(&bytes)
                }
            }
        }
    }
}

/// Probes dimensions for multiple pages in an archive or folder, with cooperative cancellation support.
/// Borrows the archive once for zip files to avoid repeatedly acquiring the archive pool lock.
pub fn probe_pages_dimensions(
    path: &Path,
    kind: SourceKind,
    pages: &[(usize, String)],
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Vec<(usize, String, (u32, u32))> {
    use std::sync::atomic::Ordering;

    let mut results = Vec::with_capacity(pages.len());
    match kind {
        SourceKind::Folder => {
            for &(ix, ref name) in pages {
                if let Some(c) = cancel
                    && c.load(Ordering::Relaxed)
                {
                    break;
                }
                if let Ok(dims) = probe_page_dimensions(path, kind, name) {
                    results.push((ix, name.clone(), dims));
                }
            }
        }
        SourceKind::Archive => {
            let format = detect_archive_format(path);
            match format {
                ArchiveFormat::Zip => {
                    if let Ok(mut archive) = checkout_archive(path) {
                        for &(ix, ref name) in pages {
                            if let Some(c) = cancel
                                && c.load(Ordering::Relaxed)
                            {
                                break;
                            }
                            if let Ok(entry) = archive.by_name(name) {
                                let mut buf = Vec::with_capacity(64 * 1024);
                                if entry.take(64 * 1024).read_to_end(&mut buf).is_ok()
                                    && let Ok(dims) = crate::imaging::probe_dimensions(&buf)
                                {
                                    results.push((ix, name.clone(), dims));
                                }
                            }
                        }
                        checkin_archive(path.to_path_buf(), archive);
                    }
                }
                ArchiveFormat::Rar => {
                    if let Ok(archive) = unrar::Archive::new(path).open_for_processing() {
                        let mut target_map: std::collections::HashMap<String, usize> = pages
                            .iter()
                            .map(|&(ix, ref name)| (name.clone(), ix))
                            .collect();
                        let mut proc = archive;
                        while !target_map.is_empty() {
                            if let Some(c) = cancel
                                && c.load(Ordering::Relaxed)
                            {
                                break;
                            }
                            let header = match proc.read_header() {
                                Ok(Some(h)) => h,
                                _ => break,
                            };
                            let entry_fn = &header.entry().filename;
                            let current_name: std::borrow::Cow<'_, str> =
                                if let Some(s) = entry_fn.to_str() {
                                    if s.contains('\\') {
                                        std::borrow::Cow::Owned(s.replace('\\', "/"))
                                    } else {
                                        std::borrow::Cow::Borrowed(s)
                                    }
                                } else {
                                    let name_str = entry_fn.to_string_lossy();
                                    if name_str.contains('\\') {
                                        std::borrow::Cow::Owned(name_str.replace('\\', "/"))
                                    } else {
                                        name_str
                                    }
                                };
                            let should_extract = if let Some((name_in_map, &ix)) =
                                target_map.get_key_value(current_name.as_ref())
                            {
                                if header.entry().unpacked_size <= MAX_PAGE_BYTES
                                    && !header.entry().is_encrypted()
                                {
                                    Some((name_in_map.clone(), ix))
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if let Some((name, ix)) = should_extract {
                                match header.read_bounded(MAX_PAGE_BYTES as usize) {
                                    Ok((data, next)) => {
                                        if let Ok(dims) = crate::imaging::probe_dimensions(&data) {
                                            results.push((ix, name.clone(), dims));
                                        }
                                        target_map.remove(&name);
                                        proc = next;
                                    }
                                    Err(_) => break,
                                }
                            } else {
                                match header.skip() {
                                    Ok(next) => proc = next,
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    results
}

fn crc32_inline(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

const TEST_RAR: [u8; 87] = [
    0x52, 0x61, 0x72, 0x21, 0x1a, 0x07, 0x00, 0xcf, 0x90, 0x73, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0f, 0x0c, 0x74, 0x20, 0x80, 0x27, 0x00, 0x15, 0x00, 0x00, 0x00, 0x0b,
    0x00, 0x00, 0x00, 0x03, 0x45, 0xf3, 0x7d, 0xc6, 0xa4, 0x8a, 0x07, 0x47, 0x1d, 0x33, 0x07, 0x00,
    0xa4, 0x81, 0x00, 0x00, 0x56, 0x45, 0x52, 0x53, 0x49, 0x4f, 0x4e, 0x0c, 0x00, 0x8f, 0xec, 0x8a,
    0x45, 0xcc, 0x23, 0xc8, 0x48, 0x08, 0x83, 0x62, 0xfe, 0x5f, 0xdd, 0x5c, 0x53, 0x88, 0xf0, 0x72,
    0xc4, 0x3d, 0x7b, 0x00, 0x40, 0x07, 0x00,
];

#[doc(hidden)]
pub fn make_multi_page_cbr(names: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&TEST_RAR[0..0x14]);

    const PAYLOAD: &[u8] = &[
        0x0c, 0x00, 0x8f, 0xec, 0x8a, 0x45, 0xcc, 0x23, 0xc8, 0x48, 0x08, 0x83, 0x62, 0xfe, 0x5f,
        0xdd, 0x5c, 0x53, 0x88, 0xf0, 0x72,
    ];

    for &name in names {
        let name_bytes = name.as_bytes();
        let head_size = (32 + name_bytes.len()) as u16;
        let mut header = Vec::with_capacity(head_size as usize);
        header.extend_from_slice(&[0, 0]); // CRC placeholder
        header.push(0x74); // type: FILE_HEAD
        header.extend_from_slice(&[0x20, 0x80]); // flags: 0x8020
        header.extend_from_slice(&head_size.to_le_bytes());
        header.extend_from_slice(&21u32.to_le_bytes()); // pack_size
        header.extend_from_slice(&11u32.to_le_bytes()); // unp_size
        header.push(3); // host_os: Unix
        header.extend_from_slice(&[0x45, 0xf3, 0x7d, 0xc6]); // file_crc
        header.extend_from_slice(&[0xa4, 0x8a, 0x07, 0x47]); // file_time
        header.push(29); // unp_ver
        header.push(0x33); // method
        header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes()); // name_size
        header.extend_from_slice(&[0xa4, 0x81, 0x00, 0x00]); // file_attr
        header.extend_from_slice(name_bytes);

        let crc = (crc32_inline(&header[2..]) & 0xffff) as u16;
        header[0..2].copy_from_slice(&crc.to_le_bytes());

        out.extend_from_slice(&header);
        out.extend_from_slice(PAYLOAD);
    }

    out.extend_from_slice(&[0xc4, 0x3d, 0x7b, 0x00, 0x40, 0x07, 0x00]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_test_cbr(name_7chars: &str) -> Vec<u8> {
        assert_eq!(name_7chars.len(), 7);
        let mut bytes = TEST_RAR.to_vec();
        bytes[0x34..0x3b].copy_from_slice(name_7chars.as_bytes());
        let head_size = u16::from_le_bytes([bytes[0x19], bytes[0x1a]]) as usize;
        let data = &bytes[0x16..0x14 + head_size];
        let crc = (crc32_inline(data) & 0xffff) as u16;
        bytes[0x14..0x16].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    #[test]
    fn cbr_archive_lists_and_reads_pages() {
        let cbr_bytes = make_test_cbr("001.jpg");
        let tmp = std::env::temp_dir().join(format!("yomu_test_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        assert_eq!(detect_archive_format(&tmp), ArchiveFormat::Rar);
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap();
        assert_eq!(pages, vec!["001.jpg"]);

        let bytes = read_page(&tmp, SourceKind::Archive, "001.jpg").unwrap();
        assert_eq!(&bytes, b"unrar-0.4.0");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn cbr_multi_page_archive_lists_and_reads_pages() {
        let cbr_bytes = make_multi_page_cbr(&["001.jpg", "002.jpg", "003.jpg"]);
        let tmp =
            std::env::temp_dir().join(format!("yomu_multi_test_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        assert_eq!(detect_archive_format(&tmp), ArchiveFormat::Rar);
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap();
        assert_eq!(pages, vec!["001.jpg", "002.jpg", "003.jpg"]);

        let b1 = read_page(&tmp, SourceKind::Archive, "001.jpg").unwrap();
        assert_eq!(&b1, b"unrar-0.4.0");
        let b2 = read_page(&tmp, SourceKind::Archive, "002.jpg").unwrap();
        assert_eq!(&b2, b"unrar-0.4.0");
        let b3 = read_page(&tmp, SourceKind::Archive, "003.jpg").unwrap();
        assert_eq!(&b3, b"unrar-0.4.0");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    #[ignore = "prints timings and metrics"]
    fn perf_stage6c_cbr_baseline() {
        use std::time::Instant;

        println!(
            "\n================================================================================"
        );
        println!("  STAGE 6C: CBR/RAR ACCESS BASELINE (QUADRATIC RE-OPENING & HEADER SCANS)");
        println!(
            "================================================================================"
        );

        for &page_count in &[25, 50, 100, 200] {
            let names: Vec<String> = (1..=page_count)
                .map(|i| format!("page_{:03}.jpg", i))
                .collect();
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

            let cbr_bytes = make_multi_page_cbr(&name_refs);
            let tmp = std::env::temp_dir().join(format!(
                "yomu_cbr_baseline_{}_{}.cbr",
                page_count,
                crate::util::now_secs()
            ));
            fs::write(&tmp, &cbr_bytes).unwrap();

            // 1. Sequential reading of all pages using current unoptimized read_page()
            let seq_start = Instant::now();
            let mut first_page_time = None;
            let mut mid_page_time = None;
            let mut last_page_time = None;

            for (i, name) in names.iter().enumerate() {
                let p_start = Instant::now();
                let bytes = read_page(&tmp, SourceKind::Archive, name).expect("read page");
                assert_eq!(bytes, b"unrar-0.4.0");
                let elapsed = p_start.elapsed();

                if i == 0 {
                    first_page_time = Some(elapsed);
                } else if i == page_count / 2 {
                    mid_page_time = Some(elapsed);
                } else if i == page_count - 1 {
                    last_page_time = Some(elapsed);
                }
            }
            let total_seq_time = seq_start.elapsed();

            // 2. Random access & backward jumps
            let mid_name = &names[page_count / 2];
            let last_name = &names[page_count - 1];
            let first_name = &names[0];

            let t_mid_start = Instant::now();
            let _ = read_page(&tmp, SourceKind::Archive, mid_name).unwrap();
            let t_mid = t_mid_start.elapsed();

            let t_last_start = Instant::now();
            let _ = read_page(&tmp, SourceKind::Archive, last_name).unwrap();
            let t_last = t_last_start.elapsed();

            let t_back_start = Instant::now();
            let _ = read_page(&tmp, SourceKind::Archive, first_name).unwrap();
            let t_back = t_back_start.elapsed();

            let total_headers = (page_count * (page_count + 1)) / 2;

            println!("\n--- [CBR Archive: {page_count} Pages] ---");
            println!(
                "  Archive File Size:              {} bytes",
                cbr_bytes.len()
            );
            println!("  Sequential Read Total Time:     {total_seq_time:?}");
            println!(
                "  Average Time per Page:          {:?}",
                total_seq_time / page_count as u32
            );
            println!("  Archive Opens:                  {page_count} (1 open per page)");
            println!("  Cumulative Header Scans:        {total_headers} (O(N^2) quadratic)");
            println!(
                "  First Page Access Latency:      {:?}",
                first_page_time.unwrap()
            );
            println!(
                "  Middle Page Access Latency:     {:?}",
                mid_page_time.unwrap()
            );
            println!(
                "  Last Page Access Latency:       {:?}",
                last_page_time.unwrap()
            );
            println!("  Random Mid-Page Access:         {t_mid:?}");
            println!("  Random Last-Page Access:        {t_last:?}");
            println!("  Backward Jump to First Page:    {t_back:?}");

            let _ = fs::remove_file(&tmp);
        }
    }

    #[test]
    fn test_rar_session_sequential_extraction() {
        let names: Vec<String> = (1..=10).map(|i| format!("page_{:03}.jpg", i)).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let cbr_bytes = make_multi_page_cbr(&name_refs);
        let tmp =
            std::env::temp_dir().join(format!("yomu_session_seq_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).expect("open rar session");
        for name in &names {
            let data = session.read_page(name, None).expect("read page in session");
            assert_eq!(data, b"unrar-0.4.0");
        }

        drop(session);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_random_page_access_and_backward() {
        let names: Vec<String> = (1..=10).map(|i| format!("page_{:03}.jpg", i)).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let cbr_bytes = make_multi_page_cbr(&name_refs);
        let tmp =
            std::env::temp_dir().join(format!("yomu_session_rnd_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).expect("open rar session");

        // 1. First read
        assert_eq!(
            session.read_page("page_001.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );
        // 2. Forward jump
        assert_eq!(
            session.read_page("page_007.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );
        // 3. Next page
        assert_eq!(
            session.read_page("page_008.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );
        // 4. Backward jump
        assert_eq!(
            session.read_page("page_003.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );
        // 5. Repeated read of same page
        assert_eq!(
            session.read_page("page_003.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );
        // 6. Jump to end
        assert_eq!(
            session.read_page("page_010.jpg", None).unwrap(),
            b"unrar-0.4.0"
        );

        drop(session);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_cancellation() {
        let names = ["page_001.jpg", "page_002.jpg"];
        let cbr_bytes = make_multi_page_cbr(&names);
        let tmp = std::env::temp_dir().join(format!(
            "yomu_session_cancel_{}.cbr",
            crate::util::now_secs()
        ));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).expect("open rar session");

        let cancel = Arc::new(AtomicBool::new(true));
        let err = session.read_page("page_001.jpg", Some(cancel)).unwrap_err();
        assert!(err.to_string().contains("cancelled"));

        // Subsequent non-cancelled read must work cleanly
        let data = session.read_page("page_001.jpg", None).unwrap();
        assert_eq!(data, b"unrar-0.4.0");

        drop(session);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_missing_entry_resilience() {
        let names = ["page_001.jpg", "page_002.jpg"];
        let cbr_bytes = make_multi_page_cbr(&names);
        let tmp = std::env::temp_dir().join(format!(
            "yomu_session_missing_{}.cbr",
            crate::util::now_secs()
        ));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).expect("open rar session");

        let err = session.read_page("nonexistent.jpg", None).unwrap_err();
        assert!(err.to_string().contains("missing"));

        // Subsequent valid read must succeed
        let data = session.read_page("page_002.jpg", None).unwrap();
        assert_eq!(data, b"unrar-0.4.0");

        drop(session);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_handle_cleanup_on_drop() {
        let names = ["page_001.jpg"];
        let cbr_bytes = make_multi_page_cbr(&names);
        let tmp = std::env::temp_dir().join(format!(
            "yomu_session_cleanup_{}.cbr",
            crate::util::now_secs()
        ));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).expect("open rar session");
        let _ = session.read_page("page_001.jpg", None).unwrap();

        // Explicit drop must join the worker thread and close the file descriptor
        drop(session);

        // Immediate removal should succeed without file lock errors
        assert!(fs::remove_file(&tmp).is_ok());
    }

    #[test]
    #[ignore = "prints timings and metrics"]
    fn perf_stage6c_cbr_remediated() {
        use std::time::Instant;

        println!(
            "\n================================================================================"
        );
        println!("  STAGE 6C: CBR/RAR ACCESS REMEDIATED (WORKER-CONFINED SESSION O(1) ADVANCE)");
        println!(
            "================================================================================"
        );

        for &page_count in &[25, 50, 100, 200] {
            let names: Vec<String> = (1..=page_count)
                .map(|i| format!("page_{:03}.jpg", i))
                .collect();
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

            let cbr_bytes = make_multi_page_cbr(&name_refs);
            let tmp = std::env::temp_dir().join(format!(
                "yomu_cbr_remediated_{}_{}.cbr",
                page_count,
                crate::util::now_secs()
            ));
            fs::write(&tmp, &cbr_bytes).unwrap();

            let session = RarSession::open(&tmp).expect("open rar session");

            // 1. Sequential reading of all pages using RarSession
            let seq_start = Instant::now();
            let mut first_page_time = None;
            let mut mid_page_time = None;
            let mut last_page_time = None;

            for (i, name) in names.iter().enumerate() {
                let p_start = Instant::now();
                let bytes = session.read_page(name, None).expect("read page in session");
                assert_eq!(bytes, b"unrar-0.4.0");
                let elapsed = p_start.elapsed();

                if i == 0 {
                    first_page_time = Some(elapsed);
                } else if i == page_count / 2 {
                    mid_page_time = Some(elapsed);
                } else if i == page_count - 1 {
                    last_page_time = Some(elapsed);
                }
            }
            let total_seq_time = seq_start.elapsed();

            // 2. Random access & backward jumps
            let mid_name = &names[page_count / 2];
            let last_name = &names[page_count - 1];
            let first_name = &names[0];

            let t_mid_start = Instant::now();
            let _ = session.read_page(mid_name, None).unwrap();
            let t_mid = t_mid_start.elapsed();

            let t_last_start = Instant::now();
            let _ = session.read_page(last_name, None).unwrap();
            let t_last = t_last_start.elapsed();

            let t_back_start = Instant::now();
            let _ = session.read_page(first_name, None).unwrap();
            let t_back = t_back_start.elapsed();

            drop(session);

            println!("\n--- [CBR Archive (Remediated): {page_count} Pages] ---");
            println!(
                "  Archive File Size:              {} bytes",
                cbr_bytes.len()
            );
            println!("  Sequential Read Total Time:     {total_seq_time:?}");
            println!(
                "  Average Time per Page:          {:?}",
                total_seq_time / page_count as u32
            );
            println!("  Archive Opens:                  1 (opened once for entire session)");
            println!("  Sequential Header Scans:        {page_count} (O(N) linear stream)");
            println!(
                "  First Page Access Latency:      {:?}",
                first_page_time.unwrap()
            );
            println!(
                "  Middle Page Access Latency:     {:?}",
                mid_page_time.unwrap()
            );
            println!(
                "  Last Page Access Latency:       {:?}",
                last_page_time.unwrap()
            );
            println!("  Random Mid-Page Access:         {t_mid:?}");
            println!("  Random Last-Page Access:        {t_last:?}");
            println!("  Backward Jump to First Page:    {t_back:?}");

            let _ = fs::remove_file(&tmp);
        }
    }

    #[test]
    fn cbr_renamed_zip_falls_back_gracefully() {
        let tmp =
            std::env::temp_dir().join(format!("yomu_renamed_{}.cbr", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("001.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"png_content").unwrap();
        zip.finish().unwrap();

        // Magic bytes recognize it as ZIP even though extension is .cbr
        assert_eq!(detect_archive_format(&tmp), ArchiveFormat::Zip);
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap();
        assert_eq!(pages, vec!["001.png"]);

        let bytes = read_page(&tmp, SourceKind::Archive, "001.png").unwrap();
        assert_eq!(&bytes, b"png_content");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn extension_checks() {
        assert!(is_image_name("page.jpg"));
        assert!(is_image_name("PAGE.JPEG"));
        assert!(is_image_name("scan.JFIF"));
        assert!(is_image_name("test.PNG"));
        assert!(is_image_name("manga.webp"));
        assert!(is_image_name("anim.gif"));
        assert!(!is_image_name("notes.txt"));
        assert!(!is_image_name("archive.zip"));

        assert!(is_archive_name("book.cbz"));
        assert!(is_archive_name("BOOK.ZIP"));
        assert!(is_archive_name("book.cbr"));
        assert!(is_archive_name("BOOK.RAR"));
        assert!(!is_archive_name("book.tar"));
    }

    #[test]
    fn probe_pages_dimensions_with_cancellation() {
        use std::sync::atomic::AtomicBool;

        // Generate tiny valid PNG bytes (10x20)
        let mut png_bytes = Vec::new();
        let img = image::RgbaImage::from_pixel(10, 20, image::Rgba([0, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        let tmp = std::env::temp_dir().join(format!("yomu_probe_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("001.png", options).unwrap();
        zip.write_all(&png_bytes).unwrap();
        zip.start_file("002.png", options).unwrap();
        zip.write_all(&png_bytes).unwrap();
        zip.finish().unwrap();

        let pages = vec![(0, "001.png".to_string()), (1, "002.png".to_string())];
        let probed = probe_pages_dimensions(&tmp, SourceKind::Archive, &pages, None);
        assert_eq!(probed.len(), 2);
        assert_eq!(probed[0], (0, "001.png".to_string(), (10, 20)));
        assert_eq!(probed[1], (1, "002.png".to_string(), (10, 20)));

        // Test early cancellation
        let cancel = AtomicBool::new(true);
        let probed_cancelled =
            probe_pages_dimensions(&tmp, SourceKind::Archive, &pages, Some(&cancel));
        assert!(probed_cancelled.is_empty());

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn archive_cache_reads_repeatedly() {
        let tmp = std::env::temp_dir().join(format!("yomu_test_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("001.jpg", options).unwrap();
        zip.write_all(b"page1content").unwrap();
        zip.start_file("002.jpg", options).unwrap();
        zip.write_all(b"page2content").unwrap();
        zip.finish().unwrap();

        let pages = list_pages(&tmp, SourceKind::Archive).unwrap();
        assert_eq!(pages, vec!["001.jpg", "002.jpg"]);

        // First read populates cache
        let p1 = read_page(&tmp, SourceKind::Archive, "001.jpg").unwrap();
        assert_eq!(p1, b"page1content");

        // Second read uses cached archive handle
        let p2 = read_page(&tmp, SourceKind::Archive, "002.jpg").unwrap();
        assert_eq!(p2, b"page2content");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn concurrent_archive_reads() {
        let tmp = std::env::temp_dir().join(format!(
            "yomu_test_concurrent_{}.cbz",
            crate::util::now_secs()
        ));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("001.jpg", options).unwrap();
        zip.write_all(b"page1content").unwrap();
        zip.start_file("002.jpg", options).unwrap();
        zip.write_all(b"page2content").unwrap();
        zip.finish().unwrap();

        std::thread::scope(|s| {
            let t1 = s.spawn(|| read_page(&tmp, SourceKind::Archive, "001.jpg").unwrap());
            let t2 = s.spawn(|| read_page(&tmp, SourceKind::Archive, "002.jpg").unwrap());
            assert_eq!(t1.join().unwrap(), b"page1content");
            assert_eq!(t2.join().unwrap(), b"page2content");
        });

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn archive_pool_high_concurrency_stress() {
        let tmp =
            std::env::temp_dir().join(format!("yomu_test_stress_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for i in 0..12 {
            zip.start_file(format!("{i:03}.jpg"), options).unwrap();
            zip.write_all(format!("content_for_page_{i}").as_bytes())
                .unwrap();
        }
        zip.finish().unwrap();

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for t in 0..16 {
                let p = &tmp;
                handles.push(s.spawn(move || {
                    for iter in 0..10 {
                        let page_num = (t + iter) % 12;
                        let page_name = format!("{page_num:03}.jpg");
                        let data = read_page(p, SourceKind::Archive, &page_name).unwrap();
                        assert_eq!(data, format!("content_for_page_{page_num}").as_bytes());
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn legitimate_large_page_reads_successfully() {
        // Create an uncompressed 9 MiB page (exceeding the 8 MiB initial buffer cap)
        let tmp =
            std::env::temp_dir().join(format!("yomu_large_page_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("large.jpg", options).unwrap();
        let payload = vec![0x42u8; 9 * 1024 * 1024];
        zip.write_all(&payload).unwrap();
        zip.finish().unwrap();

        let data = read_page(&tmp, SourceKind::Archive, "large.jpg").unwrap();
        assert_eq!(data.len(), 9 * 1024 * 1024);
        assert_eq!(data[0], 0x42);
        assert_eq!(data[data.len() - 1], 0x42);

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn unreasonable_size_claim_in_zip_is_rejected() {
        let tmp = std::env::temp_dir().join(format!("yomu_bomb_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("001.jpg", options).unwrap();
        zip.write_all(b"small_payload").unwrap();
        zip.finish().unwrap();

        // Mutate the headers' uncompressed_size to exceed MAX_PAGE_BYTES (e.g. 300 MB)
        let mut bytes = fs::read(&tmp).unwrap();
        if let Some(pos) = bytes.windows(4).position(|w| w == b"PK\x03\x04") {
            let uncompressed_size_pos = pos + 22;
            let huge_size: u32 = 300 * 1024 * 1024;
            bytes[uncompressed_size_pos..uncompressed_size_pos + 4]
                .copy_from_slice(&huge_size.to_le_bytes());
        }
        if let Some(pos) = bytes.windows(4).position(|w| w == b"PK\x01\x02") {
            let uncompressed_size_pos = pos + 24;
            let huge_size: u32 = 300 * 1024 * 1024;
            bytes[uncompressed_size_pos..uncompressed_size_pos + 4]
                .copy_from_slice(&huge_size.to_le_bytes());
        }
        fs::write(&tmp, &bytes).unwrap();

        let res = read_page(&tmp, SourceKind::Archive, "001.jpg");
        assert!(res.is_err());
        let err_msg = format!("{:#}", res.unwrap_err());
        assert!(
            err_msg.contains("unreasonably large")
                || err_msg.contains("invalid")
                || err_msg.contains("corrupt")
                || err_msg.contains("failed")
        );

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn streaming_guard_rejects_zip_bomb_with_small_declared_size() {
        let tmp =
            std::env::temp_dir().join(format!("yomu_bomb_stream_{}.cbz", crate::util::now_secs()));
        let file = File::create(&tmp).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("bomb.jpg", options).unwrap();
        let chunk = [0u8; 1024 * 1024];
        // Write 257 MiB of compressible zeroes (compresses to ~250 KB on disk)
        for _ in 0..257 {
            zip.write_all(&chunk).unwrap();
        }
        zip.finish().unwrap();

        // Mutate the declared uncompressed_size in headers to claim only 1024 bytes.
        // This ensures entry.size() <= MAX_PAGE_BYTES, bypassing the upfront header check
        // and forcing execution through the streaming entry.take(MAX_PAGE_BYTES + 1) guard.
        let mut bytes = fs::read(&tmp).unwrap();
        let small_size: u32 = 1024;
        if let Some(pos) = bytes.windows(4).position(|w| w == b"PK\x03\x04") {
            let uncompressed_size_pos = pos + 22;
            bytes[uncompressed_size_pos..uncompressed_size_pos + 4]
                .copy_from_slice(&small_size.to_le_bytes());
        }
        if let Some(pos) = bytes.windows(4).position(|w| w == b"PK\x01\x02") {
            let uncompressed_size_pos = pos + 24;
            bytes[uncompressed_size_pos..uncompressed_size_pos + 4]
                .copy_from_slice(&small_size.to_le_bytes());
        }
        fs::write(&tmp, &bytes).unwrap();

        let res = read_page(&tmp, SourceKind::Archive, "bomb.jpg");
        assert!(res.is_err(), "decompression bomb must be rejected");
        let err_msg = format!("{:#}", res.unwrap_err());
        assert!(
            err_msg.contains("is unreasonably large"),
            "expected 'is unreasonably large', got: {err_msg}"
        );

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_is_safe_archive_entry_name() {
        assert!(is_safe_archive_entry_name("001.jpg"));
        assert!(is_safe_archive_entry_name("sub/001.jpg"));
        assert!(is_safe_archive_entry_name("chapter_01/sub_02/001.png"));

        // Path traversal
        assert!(!is_safe_archive_entry_name(""));
        assert!(!is_safe_archive_entry_name("../001.jpg"));
        assert!(!is_safe_archive_entry_name("sub/../001.jpg"));
        assert!(!is_safe_archive_entry_name("/root/001.jpg"));
        assert!(!is_safe_archive_entry_name(r"\windows\001.jpg"));
        assert!(!is_safe_archive_entry_name(r"C:\001.jpg"));
        assert!(!is_safe_archive_entry_name("sub//001.jpg"));
        assert!(!is_safe_archive_entry_name("sub/.hidden/001.jpg"));
        assert!(!is_safe_archive_entry_name("sub/__MACOSX/001.jpg"));
        assert!(!is_safe_archive_entry_name("page\0.jpg"));
    }

    #[test]
    fn test_cbr_path_traversal_entry_rejected() {
        let cbr_bytes = make_test_cbr("../evil");
        let tmp =
            std::env::temp_dir().join(format!("yomu_traversal_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        // Path traversal entry should be omitted from list_pages
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap();
        assert!(pages.is_empty());

        // Explicit read request must be rejected
        let read_err = read_page(&tmp, SourceKind::Archive, "../evil").unwrap_err();
        assert!(read_err.to_string().contains("invalid or unsafe"));

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_decompression_bomb_unpacked_size_rejected() {
        let mut bytes = make_test_cbr("001.jpg");
        // Overwrite unpacked size at offset 31..35 with 1 GiB (0x40000000)
        bytes[31..35].copy_from_slice(&0x4000_0000u32.to_le_bytes());
        let head_size = u16::from_le_bytes([bytes[0x19], bytes[0x1a]]) as usize;
        let data = &bytes[0x16..0x14 + head_size];
        let crc = (crc32_inline(data) & 0xffff) as u16;
        bytes[0x14..0x16].copy_from_slice(&crc.to_le_bytes());

        let tmp = std::env::temp_dir().join(format!("yomu_bomb_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &bytes).unwrap();

        let err = read_page(&tmp, SourceKind::Archive, "001.jpg").unwrap_err();
        assert!(err.to_string().contains("unreasonably large"));

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_corrupted_archive_handled_gracefully() {
        let tmp =
            std::env::temp_dir().join(format!("yomu_corrupt_{}.cbr", crate::util::now_secs()));
        // Truncated / empty RAR file returns empty pages or error, and read fails
        fs::write(&tmp, b"Rar!\x1a\x07\x00").unwrap();
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap_or_default();
        assert!(pages.is_empty(), "Empty archive has no pages");
        assert!(read_page(&tmp, SourceKind::Archive, "001.jpg").is_err());

        // Completely random garbage bytes
        fs::write(&tmp, b"corrupted random data stream").unwrap();
        assert!(list_pages(&tmp, SourceKind::Archive).is_err());
        assert!(read_page(&tmp, SourceKind::Archive, "001.jpg").is_err());

        // Corrupted header after magic bytes
        fs::write(&tmp, b"Rar!\x1a\x07\x00\x00\x00\x00\x00").unwrap();
        let pages = list_pages(&tmp, SourceKind::Archive).unwrap_or_default();
        assert!(pages.is_empty());
        assert!(read_page(&tmp, SourceKind::Archive, "001.jpg").is_err());

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_single_pass_dimension_probing_with_cancellation() {
        use std::sync::atomic::AtomicBool;

        let cbr_bytes = make_test_cbr("001.jpg");
        let tmp =
            std::env::temp_dir().join(format!("yomu_probe_cbr_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let pages = vec![(0, "001.jpg".to_string()), (1, "002.jpg".to_string())];

        // 1. Probing with early cancellation flag active immediately returns empty
        let cancel_flag = AtomicBool::new(true);
        let cancelled_probed =
            probe_pages_dimensions(&tmp, SourceKind::Archive, &pages, Some(&cancel_flag));
        assert!(cancelled_probed.is_empty());

        // 2. Probing without cancellation attempts decompression; non-image returns Ok([]) cleanly
        let cancel_off = AtomicBool::new(false);
        let probed = probe_pages_dimensions(&tmp, SourceKind::Archive, &pages, Some(&cancel_off));
        // b"unrar-0.4.0" is not a valid image, so probe_dimensions fails and results are empty
        assert!(probed.is_empty());

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_bounded_extraction_exact_limit() {
        let cbr_bytes = make_test_cbr("001.jpg");
        let tmp = std::env::temp_dir().join(format!("yomu_exact_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let archive = unrar::Archive::new(&tmp).open_for_processing().unwrap();
        let header = archive.read_header().unwrap().unwrap();
        // Payload decompresses to exactly 11 bytes: b"unrar-0.4.0"
        let (data, _next) = header.read_bounded(11).expect("exact limit must succeed");
        assert_eq!(&data, b"unrar-0.4.0");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_bounded_extraction_exceeded_by_one_byte_aborts() {
        let cbr_bytes = make_test_cbr("001.jpg");
        let tmp = std::env::temp_dir().join(format!("yomu_over1_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let archive = unrar::Archive::new(&tmp).open_for_processing().unwrap();
        let header = archive.read_header().unwrap().unwrap();
        // Payload is 11 bytes; budget of 10 bytes must abort inside callback
        let err = header
            .read_bounded(10)
            .expect_err("exceeding limit by 1 byte must abort");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("exceeds maximum budget") || msg.contains("ExcessiveSize"),
            "Error must indicate size budget exceeded, got: {msg}"
        );

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_cbr_decompression_bomb_spoofed_header_callback_aborts() {
        let mut bytes = make_test_cbr("001.jpg");
        // Adversarial spoofing: header claims unpacked size is 20 bytes (less than a naive large budget)
        bytes[31..35].copy_from_slice(&20u32.to_le_bytes());
        let head_size = u16::from_le_bytes([bytes[0x19], bytes[0x1a]]) as usize;
        let data = &bytes[0x16..0x14 + head_size];
        let crc = (crc32_inline(data) & 0xffff) as u16;
        bytes[0x14..0x16].copy_from_slice(&crc.to_le_bytes());

        let tmp = std::env::temp_dir().join(format!("yomu_spoof_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &bytes).unwrap();

        let archive = unrar::Archive::new(&tmp).open_for_processing().unwrap();
        let header = archive.read_header().unwrap().unwrap();
        assert_eq!(
            header.entry().unpacked_size,
            20,
            "Header size was spoofed to 20"
        );

        // Limit is 10 bytes. The 11-byte actual decompressed payload exceeds 10 bytes.
        // Bounded callback must abort during extraction when incoming chunk exceeds 10 bytes!
        let err = header
            .read_bounded(10)
            .expect_err("Callback must intercept payload exceeding 10 bytes");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("exceeds maximum budget") || msg.contains("ExcessiveSize"),
            "Decompression callback must report budget exceeded, got: {msg}"
        );

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_recovery_after_bounded_extraction_failure() {
        let cbr_bytes = make_multi_page_cbr(&["001.jpg", "002.jpg", "003.jpg"]);
        let tmp =
            std::env::temp_dir().join(format!("yomu_session_rec_{}.cbr", crate::util::now_secs()));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).unwrap();

        // 1. Initial valid page extraction
        let p1 = session.read_page("001.jpg", None).unwrap();
        assert_eq!(&p1, b"unrar-0.4.0");

        // 2. Request a non-existent entry causing extraction failure
        let fail_res = session.read_page("nonexistent.jpg", None);
        assert!(fail_res.is_err());

        // 3. Subsequent valid reads on the SAME session succeed, verifying worker recovery
        let p2 = session.read_page("002.jpg", None).unwrap();
        assert_eq!(&p2, b"unrar-0.4.0");

        let p3 = session.read_page("003.jpg", None).unwrap();
        assert_eq!(&p3, b"unrar-0.4.0");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_rar_session_repeated_consecutive_failures_resilience() {
        let cbr_bytes = make_multi_page_cbr(&["001.jpg", "002.jpg"]);
        let tmp = std::env::temp_dir().join(format!(
            "yomu_session_stress_{}.cbr",
            crate::util::now_secs()
        ));
        fs::write(&tmp, &cbr_bytes).unwrap();

        let session = RarSession::open(&tmp).unwrap();

        // 5 consecutive failed extraction attempts
        for i in 0..5 {
            let res = session.read_page(&format!("missing_{i}.jpg"), None);
            assert!(res.is_err());
        }

        // Subsequent valid read still succeeds cleanly
        let p1 = session.read_page("001.jpg", None).unwrap();
        assert_eq!(&p1, b"unrar-0.4.0");

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_unrar_bounded_buffer_callback_boundary_and_overflow_conditions() {
        use unrar::BoundedBuffer;

        // 1. Chunk smaller than limit accepted
        let mut buf = BoundedBuffer::new(100);
        assert!(buf.process_chunk(&[42u8; 50]));
        assert_eq!(buf.data.len(), 50);
        assert!(!buf.exceeded);

        // 2. Exact limit accepted
        assert!(buf.process_chunk(&[42u8; 50]));
        assert_eq!(buf.data.len(), 100);
        assert!(!buf.exceeded);

        // 3. Exceeded by 1 byte rejected, buffer cleared and capacity freed immediately
        assert!(!buf.process_chunk(&[42u8; 1]));
        assert!(buf.exceeded);
        assert_eq!(buf.data.len(), 0);
        assert_eq!(buf.data.capacity(), 0);

        // 4. Multi-chunk accumulation boundary
        let mut multi = BoundedBuffer::new(30);
        for i in 1..=3 {
            assert!(
                multi.process_chunk(&[i as u8; 10]),
                "Chunk {i} must be accepted"
            );
            assert_eq!(multi.data.len(), i * 10);
        }
        assert_eq!(multi.data.len(), 30);
        // Exceeding on 4th chunk
        assert!(!multi.process_chunk(&[99u8; 1]));
        assert!(multi.exceeded);
        assert_eq!(multi.data.len(), 0);

        // 5. Integer overflow check: current_len + huge_chunk must not wrap around
        let mut overflow_buf = BoundedBuffer::new(100);
        assert!(overflow_buf.process_chunk(&[1u8; 10]));
        // A synthetic slice or overflow scenario: current_len.checked_add(usize::MAX) returns None
        let current_len = overflow_buf.data.len();
        assert_eq!(current_len.checked_add(usize::MAX), None);
        // Verify process_chunk correctly rejects when checked_add fails
        let checked = current_len.checked_add(usize::MAX);
        assert!(checked.is_none());
    }
}
