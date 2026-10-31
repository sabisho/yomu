//! Desktop environment integration, application icons, and XDG desktop entry installation.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

/// Embedded application logo (PNG format).
pub const LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png");

/// Embedded hicolor application icons across standard desktop resolutions.
pub const HICOLOR_ICON_SIZES: &[(&str, &[u8])] = &[
    ("16x16", include_bytes!("../../assets/icons/16x16.png")),
    ("24x24", include_bytes!("../../assets/icons/24x24.png")),
    ("32x32", include_bytes!("../../assets/icons/32x32.png")),
    ("48x48", include_bytes!("../../assets/icons/48x48.png")),
    ("64x64", include_bytes!("../../assets/icons/64x64.png")),
    ("128x128", include_bytes!("../../assets/icons/128x128.png")),
    ("256x256", include_bytes!("../../assets/icons/256x256.png")),
    ("512x512", include_bytes!("../../assets/icons/512x512.png")),
];

/// Returns the embedded XDG `.desktop` entry content.
pub fn desktop_entry_content() -> &'static str {
    include_str!("../../packaging/yomu.desktop")
}

/// Decodes and scales the embedded logo for window icon display.
pub fn app_icon() -> Option<Arc<image::RgbaImage>> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    let resized = image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Lanczos3);
    Some(Arc::new(resized))
}

/// Installs desktop entry and application icons to the specified data directory (typically `$XDG_DATA_HOME`).
///
/// Returns `Ok(true)` if files were written or updated, or `Ok(false)` if all items were already present.
/// If `force` is false, existing user-customized `.desktop` files or icons are preserved.
#[cfg(target_os = "linux")]
pub fn install_desktop_integration_to(data_dir: &Path, force: bool) -> anyhow::Result<bool> {
    let mut updated = false;
    let pixmap_icon = data_dir.join("pixmaps/yomu.png");
    let pixmap_needs_write = force || !pixmap_icon.exists();
    if pixmap_needs_write {
        if let Some(parent) = pixmap_icon.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&pixmap_icon, LOGO_PNG)?;
        updated = true;
    }

    for (sz, bytes) in HICOLOR_ICON_SIZES {
        let path = data_dir.join(format!("icons/hicolor/{sz}/apps/yomu.png"));
        let icon_needs_write = force || !path.exists();
        if icon_needs_write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, bytes)?;
            updated = true;
        }
    }

    let desktop_file = data_dir.join("applications/yomu.desktop");
    let desktop_content = desktop_entry_content();
    let desktop_needs_write = force || !desktop_file.exists();
    if desktop_needs_write {
        if let Some(parent) = desktop_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&desktop_file, desktop_content)?;
        updated = true;
    }

    Ok(updated)
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_integration_to(_data_dir: &Path, _force: bool) -> anyhow::Result<bool> {
    anyhow::bail!("Desktop integration installation is only supported on Linux.");
}

/// Installs XDG desktop entry and icons to `$XDG_DATA_HOME` on Linux systems.
pub fn install_desktop_integration(force: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine XDG_DATA_HOME directory"))?;

        let updated = install_desktop_integration_to(&data_dir, force)?;
        if updated {
            println!(
                "Installed yomu desktop integration in {}",
                data_dir.display()
            );
            let hicolor_dir = data_dir.join("icons/hicolor");
            match std::process::Command::new("gtk-update-icon-cache")
                .arg("-f")
                .arg("-t")
                .arg(&hicolor_dir)
                .output()
            {
                Ok(output) if output.status.success() => {
                    println!("Updated icon cache: {}", hicolor_dir.display());
                }
                Ok(output) => {
                    eprintln!(
                        "gtk-update-icon-cache notice (exit {}): {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "gtk-update-icon-cache not available ({err}); skipping cache update."
                    );
                }
            }
        } else {
            println!(
                "Desktop integration already installed in {} (use --force to overwrite)",
                data_dir.display()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = force;
        anyhow::bail!("Desktop integration installation is only supported on Linux.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_entry_content() {
        let content = desktop_entry_content();
        assert!(content.contains("[Desktop Entry]"));
        assert!(content.contains("Name=yomu"));
        assert!(content.contains("Exec=yomu"));
        assert!(content.contains("Icon=yomu"));
        assert!(content.contains("Categories=Graphics;Viewer;"));
    }

    #[test]
    fn test_hicolor_icon_sizes_presence() {
        assert_eq!(HICOLOR_ICON_SIZES.len(), 8);
        for (size, bytes) in HICOLOR_ICON_SIZES {
            assert!(!size.is_empty());
            assert!(!bytes.is_empty(), "icon {size} must not be empty");
        }
    }

    #[test]
    fn test_app_icon_loads_and_resizes() {
        let icon = app_icon();
        assert!(icon.is_some());
        let img = icon.unwrap();
        assert_eq!(img.width(), 128);
        assert_eq!(img.height(), 128);
    }
}
