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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Archive,
    Folder,
}

pub fn is_image_name(name: &str) -> bool {
    const EXTENSIONS: [&str; 5] = [".jpg", ".jpeg", ".png", ".webp", ".gif"];
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
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let archive = zip::ZipArchive::new(reader)
        .with_context(|| format!("{} is not a valid zip/cbz", path.display()))?;
    Ok(archive
        .file_names()
        .filter(|name| !name.ends_with('/') && is_image_name(name) && !is_junk(name))
        .map(str::to_owned)
        .collect::<Vec<_>>())
}

fn list_rar_pages(path: &Path) -> Result<Vec<String>> {
    let archive = unrar::Archive::new(path);
    let entries = archive
        .open_for_listing()
        .with_context(|| format!("{} is not a valid rar/cbr", path.display()))?;
    let mut pages = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry from {}", path.display()))?;
        if entry.is_directory() {
            continue;
        }
        let name_str = entry.filename.to_string_lossy();
        let normalized = if name_str.contains('\\') {
            name_str.replace('\\', "/")
        } else {
            name_str.into_owned()
        };
        if is_image_name(&normalized) && !is_junk(&normalized) {
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
    let mut archive = checkout_archive(path)?;

    let read_res = (|| -> Result<Vec<u8>> {
        let entry = archive
            .by_name(name)
            .with_context(|| format!("page {name} missing from archive"))?;
        if entry.size() > MAX_PAGE_BYTES {
            bail!("page {name} is unreasonably large");
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.take(MAX_PAGE_BYTES).read_to_end(&mut buf)?;
        Ok(buf)
    })();

    checkin_archive(path.to_path_buf(), archive);
    read_res
}

fn read_rar_page(path: &Path, name: &str) -> Result<Vec<u8>> {
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
        let name_str = header.entry().filename.to_string_lossy();
        let current_name = if name_str.contains('\\') {
            name_str.replace('\\', "/")
        } else {
            name_str.into_owned()
        };
        if current_name == normalized_target {
            if header.entry().unpacked_size as u64 > MAX_PAGE_BYTES {
                bail!("page {name} is unreasonably large");
            }
            let (data, _next) = header
                .read()
                .with_context(|| format!("extracting {name} from {}", path.display()))?;
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
                ArchiveFormat::Zip => {
                    read_zip_page(path, name).or_else(|_| read_rar_page(path, name))
                }
                ArchiveFormat::Rar => {
                    read_rar_page(path, name).or_else(|_| read_zip_page(path, name))
                }
            }
        }
        SourceKind::Folder => {
            let full = path.join(name);
            fs::read(&full).with_context(|| format!("reading {}", full.display()))
        }
    }
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
}
