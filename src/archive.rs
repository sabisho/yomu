use std::{
    fs::{self, File},
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    sync::Mutex,
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
                .read()
                .with_context(|| format!("extracting {name} from {}", path.display()))?;
            if data.len() as u64 > MAX_PAGE_BYTES {
                bail!("page {name} decompressed size exceeds maximum budget");
            }
            return Ok(data);
        } else {
            proc = header
                .skip()
                .with_context(|| format!("skipping entry in {}", path.display()))?;
        }
    }
}

/// Raw (still encoded) bytes of one page.
pub fn read_page(path: &Path, kind: SourceKind, name: &str) -> Result<Vec<u8>> {
    match kind {
        SourceKind::Archive => {
            let format = detect_archive_format(path);
            match format {
                ArchiveFormat::Zip => match read_zip_page(path, name) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        let msg = format!("{err:#}");
                        if msg.contains("not a valid zip") {
                            read_rar_page(path, name)
                        } else {
                            Err(err)
                        }
                    }
                },
                ArchiveFormat::Rar => match read_rar_page(path, name) {
                    Ok(data) => Ok(data),
                    Err(err) => {
                        let msg = format!("{err:#}");
                        if msg.contains("not a valid rar") || msg.contains("Not a RAR archive") {
                            read_zip_page(path, name)
                        } else {
                            Err(err)
                        }
                    }
                },
            }
        }
        SourceKind::Folder => {
            let full = path.join(name);
            fs::read(&full).with_context(|| format!("reading {}", full.display()))
        }
    }
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
                                match header.read() {
                                    Ok((data, next)) => {
                                        if data.len() as u64 <= MAX_PAGE_BYTES
                                            && let Ok(dims) =
                                                crate::imaging::probe_dimensions(&data)
                                        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
        0x52, 0x61, 0x72, 0x21, 0x1a, 0x07, 0x00, 0xcf, 0x90, 0x73, 0x00, 0x00, 0x0d, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x0c, 0x74, 0x20, 0x80, 0x27, 0x00, 0x15, 0x00, 0x00,
        0x00, 0x0b, 0x00, 0x00, 0x00, 0x03, 0x45, 0xf3, 0x7d, 0xc6, 0xa4, 0x8a, 0x07, 0x47, 0x1d,
        0x33, 0x07, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x56, 0x45, 0x52, 0x53, 0x49, 0x4f, 0x4e, 0x0c,
        0x00, 0x8f, 0xec, 0x8a, 0x45, 0xcc, 0x23, 0xc8, 0x48, 0x08, 0x83, 0x62, 0xfe, 0x5f, 0xdd,
        0x5c, 0x53, 0x88, 0xf0, 0x72, 0xc4, 0x3d, 0x7b, 0x00, 0x40, 0x07, 0x00,
    ];

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
}
