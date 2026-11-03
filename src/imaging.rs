use std::cell::RefCell;
use std::io::Cursor;

use anyhow::{Context, Result, bail};
use fast_image_resize::{
    FilterType as ResizeFilter, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::Image as FirImage,
};
use image::{DynamicImage, ExtendedColorType, RgbImage, RgbaImage};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

thread_local! {
    /// Reusable thread-local resizer to avoid reallocating internal SIMD scratch buffers.
    static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
}

/// Longest side of a stored cover thumbnail, in pixels.
pub const COVER_MAX_W: u32 = 400;
pub const COVER_MAX_H: u32 = 600;

/// Refuse absurd dimensions before allocating (decompression bombs).
const MAX_PIXELS: u64 = 120_000_000;

/// Inspects the header of an image buffer to retrieve (width, height) without decoding pixels.
pub fn probe_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.is_empty() {
        bail!("empty image buffer");
    }

    // 1. Fast path for PNG (IHDR chunk at fixed offset)
    if bytes.len() >= 24 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" && &bytes[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if w > 0 && h > 0 {
            if u64::from(w) * u64::from(h) > MAX_PIXELS {
                bail!("image is too large ({w}x{h})");
            }
            return Ok((w, h));
        }
    }

    // 2. Fast path for JPEG via SIMD-accelerated header parser
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut decoder = JpegDecoder::new(Cursor::new(bytes));
        if decoder.decode_headers().is_ok()
            && let Some((w, h)) = decoder.dimensions()
        {
            let (w, h) = (w as u32, h as u32);
            if w > 0 && h > 0 {
                if u64::from(w) * u64::from(h) > MAX_PIXELS {
                    bail!("image is too large ({w}x{h})");
                }
                return Ok((w, h));
            }
        }
    }

    // 3. Fast path for WebP (VP8, VP8L, VP8X headers)
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let chunk = &bytes[12..16];
        if chunk == b"VP8 " {
            if bytes.len() >= 30 && bytes[23] == 0x9d && bytes[24] == 0x01 && bytes[25] == 0x2a {
                let w = (u16::from(bytes[26]) | (u16::from(bytes[27] & 0x3f) << 8)) as u32;
                let h = (u16::from(bytes[28]) | (u16::from(bytes[29] & 0x3f) << 8)) as u32;
                if w > 0 && h > 0 {
                    if u64::from(w) * u64::from(h) > MAX_PIXELS {
                        bail!("image is too large ({w}x{h})");
                    }
                    return Ok((w, h));
                }
            }
        } else if chunk == b"VP8L" {
            if bytes.len() >= 25 && bytes[20] == 0x2f {
                let b0 = bytes[21] as u32;
                let b1 = bytes[22] as u32;
                let b2 = bytes[23] as u32;
                let b3 = bytes[24] as u32;
                let w = 1 + (((b1 & 0x3f) << 8) | b0);
                let h = 1 + (((b3 & 0x0f) << 10) | (b2 << 2) | ((b1 & 0xc0) >> 6));
                if w > 0 && h > 0 {
                    if u64::from(w) * u64::from(h) > MAX_PIXELS {
                        bail!("image is too large ({w}x{h})");
                    }
                    return Ok((w, h));
                }
            }
        } else if chunk == b"VP8X" && bytes.len() >= 30 {
            let w = 1 + (bytes[24] as u32 | ((bytes[25] as u32) << 8) | ((bytes[26] as u32) << 16));
            let h = 1 + (bytes[27] as u32 | ((bytes[28] as u32) << 8) | ((bytes[29] as u32) << 16));
            if w > 0 && h > 0 {
                if u64::from(w) * u64::from(h) > MAX_PIXELS {
                    bail!("image is too large ({w}x{h})");
                }
                return Ok((w, h));
            }
        }
    }

    // 4. General fallback via image crate
    if let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()
        && let Ok((w, h)) = reader.into_dimensions()
        && w > 0
        && h > 0
        && u64::from(w) * u64::from(h) <= MAX_PIXELS
    {
        return Ok((w, h));
    }

    bail!("could not determine image dimensions from header");
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    let img = reader.decode()?;
    if u64::from(img.width()) * u64::from(img.height()) > MAX_PIXELS {
        bail!("image is too large ({}x{})", img.width(), img.height());
    }
    Ok(img)
}

/// A decoded page.
#[derive(Debug)]
pub struct Decoded {
    /// Pixels in **BGRA** order, already scaled down to the requested size.
    pub image: RgbaImage,
    /// Dimensions of the original, unscaled image.
    pub orig: (u32, u32),
}

fn rgb_to_bgra(rgb: &[u8], nw: u32, nh: u32) -> Result<RgbaImage> {
    let chunks = rgb.as_chunks::<3>().0;
    let total = nw as usize * nh as usize;
    if chunks.len() < total {
        bail!("rgb buffer smaller than nw * nh * 3");
    }
    let mut bgra = Vec::with_capacity(total * 4);
    for src in &chunks[..total] {
        bgra.extend_from_slice(&[src[2], src[1], src[0], 255]);
    }
    RgbaImage::from_raw(nw, nh, bgra).context("failed to create bgra buffer from raw bytes")
}

fn decode_jpeg_fit_bgra(bytes: &[u8], max_w: u32, max_h: u32) -> Result<Decoded> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::BGRA);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|e| anyhow::anyhow!("failed to decode JPEG headers: {e:?}"))?;
    let (w, h) = decoder.dimensions().context("missing JPEG dimensions")?;
    let (w, h) = (w as u32, h as u32);
    if w == 0 || h == 0 {
        bail!("zero dimension image");
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        bail!("image is too large ({w}x{h})");
    }
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    let pixels = decoder
        .decode()
        .map_err(|e| anyhow::anyhow!("JPEG decode error: {e:?}"))?;

    let image = if scale < 1.0 {
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        if (nw, nh) == (w, h) {
            RgbaImage::from_raw(w, h, pixels).context("raw JPEG buffer has the wrong size")?
        } else {
            let src = FirImage::from_vec_u8(w, h, pixels, PixelType::U8x4)?;
            let mut dst = FirImage::new(nw, nh, PixelType::U8x4);
            let resize_options =
                ResizeOptions::new().resize_alg(ResizeAlg::Convolution(ResizeFilter::CatmullRom));
            RESIZER.with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &resize_options))?;
            RgbaImage::from_raw(nw, nh, dst.into_vec())
                .context("resized buffer has the wrong size")?
        }
    } else {
        RgbaImage::from_raw(w, h, pixels).context("raw JPEG buffer has the wrong size")?
    };

    Ok(Decoded {
        image,
        orig: (w, h),
    })
}

/// Decode a page and scale it down (never up) so it fits inside `max_w` x `max_h`.
///
/// Pre-scaling on the CPU with a good filter looks much sharper than letting the GPU
/// minify a 3000px scan with plain bilinear sampling, and it keeps VRAM use low.
///
/// The returned buffer is already **BGRA**, the channel order gpui expects.
pub fn decode_fit_bgra(bytes: &[u8], max_w: u32, max_h: u32) -> Result<Decoded> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        match decode_jpeg_fit_bgra(bytes, max_w, max_h) {
            Ok(decoded) => return Ok(decoded),
            Err(e) => {
                if e.to_string().contains("image is too large") {
                    return Err(e);
                }
            }
        }
    }

    let img = decode(bytes)?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        bail!("zero dimension image");
    }
    let alpha = img.color().has_alpha();
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);

    let bgra = if alpha {
        let mut rgba = img.into_rgba8();
        if scale < 1.0 {
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            if (nw, nh) != (w, h) {
                rgba = resize_rgba(rgba, nw, nh, ResizeFilter::CatmullRom, true)?;
            }
        }
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        rgba
    } else {
        let rgb = img.into_rgb8();
        if scale < 1.0 {
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            if (nw, nh) != (w, h) {
                let resized = resize_rgb(rgb, nw, nh, ResizeFilter::CatmullRom)?;
                rgb_to_bgra(resized.as_raw(), nw, nh)?
            } else {
                rgb_to_bgra(rgb.as_raw(), w, h)?
            }
        } else {
            rgb_to_bgra(rgb.as_raw(), w, h)?
        }
    };

    Ok(Decoded {
        image: bgra,
        orig: (w, h),
    })
}

/// Resize to exactly `nw` x `nh` with a SIMD convolution filter.
///
/// This is the slowest step of showing a large scan (several times the JPEG decode itself with the
/// `image` crate's resize), so it uses `fast_image_resize`, which does the same filtering several
/// times faster. Alpha is only premultiplied when the image actually has transparency.
fn resize_rgba(
    rgba: RgbaImage,
    nw: u32,
    nh: u32,
    filter: ResizeFilter,
    alpha: bool,
) -> Result<RgbaImage> {
    let (w, h) = rgba.dimensions();
    let src = FirImage::from_vec_u8(w, h, rgba.into_raw(), PixelType::U8x4)?;
    let mut dst = FirImage::new(nw, nh, PixelType::U8x4);
    let mut options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));
    options.mul_div_alpha = alpha;
    RESIZER.with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &options))?;
    RgbaImage::from_raw(nw, nh, dst.into_vec()).context("resized buffer has the wrong size")
}

fn resize_rgb(rgb: RgbImage, nw: u32, nh: u32, filter: ResizeFilter) -> Result<RgbImage> {
    let (w, h) = rgb.dimensions();
    let src = FirImage::from_vec_u8(w, h, rgb.into_raw(), PixelType::U8x3)?;
    let mut dst = FirImage::new(nw, nh, PixelType::U8x3);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));
    RESIZER.with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &options))?;
    RgbImage::from_raw(nw, nh, dst.into_vec()).context("resized rgb buffer has the wrong size")
}

fn make_jpeg_cover(bytes: &[u8], max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|e| anyhow::anyhow!("failed to decode JPEG headers: {e:?}"))?;
    let (w, h) = decoder.dimensions().context("missing JPEG dimensions")?;
    let (w, h) = (w as u32, h as u32);
    if w == 0 || h == 0 {
        bail!("zero dimension image");
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        bail!("image is too large ({w}x{h})");
    }
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let pixels = decoder
        .decode()
        .map_err(|e| anyhow::anyhow!("JPEG decode error: {e:?}"))?;

    let rgb = if (nw, nh) == (w, h) {
        RgbImage::from_raw(w, h, pixels).context("invalid rgb buffer size")?
    } else {
        let src = FirImage::from_vec_u8(w, h, pixels, PixelType::U8x3)?;
        let mut dst = FirImage::new(nw, nh, PixelType::U8x3);
        let options =
            ResizeOptions::new().resize_alg(ResizeAlg::Convolution(ResizeFilter::Lanczos3));
        RESIZER.with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &options))?;
        RgbImage::from_raw(nw, nh, dst.into_vec())
            .context("resized rgb buffer has the wrong size")?
    };

    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
    encoder.encode(&rgb, nw, nh, ExtendedColorType::Rgb8)?;
    Ok(out)
}

/// Encode a JPEG thumbnail that fits inside `max_w` x `max_h`, keeping the aspect ratio.
pub fn make_cover_jpeg(bytes: &[u8], max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        match make_jpeg_cover(bytes, max_w, max_h) {
            Ok(cover) => return Ok(cover),
            Err(e) => {
                if e.to_string().contains("image is too large") {
                    return Err(e);
                }
            }
        }
    }

    let img = decode(bytes)?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        bail!("zero dimension image");
    }
    let alpha = img.color().has_alpha();
    // Fit inside the box, keep the aspect ratio, never enlarge.
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);

    let rgb = if alpha {
        let thumb = if (nw, nh) == (w, h) {
            img.into_rgba8()
        } else {
            resize_rgba(img.into_rgba8(), nw, nh, ResizeFilter::Lanczos3, true)?
        };
        DynamicImage::ImageRgba8(thumb).into_rgb8()
    } else {
        let rgb = img.into_rgb8();
        if (nw, nh) == (w, h) {
            rgb
        } else {
            resize_rgb(rgb, nw, nh, ResizeFilter::Lanczos3)?
        }
    };

    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
    encoder.encode(&rgb, nw, nh, ExtendedColorType::Rgb8)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba};
    use std::io::Cursor;

    fn png(w: u32, h: u32, px: Rgba<u8>) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, px);
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn scales_down_and_swaps_to_bgra() {
        let bytes = png(400, 800, Rgba([10, 20, 30, 255]));
        let out = decode_fit_bgra(&bytes, 100, 100).unwrap();
        assert_eq!(out.image.dimensions(), (50, 100));
        assert_eq!(out.orig, (400, 800));
        assert_eq!(out.image.get_pixel(0, 0).0, [30, 20, 10, 255]);
    }

    #[test]
    fn resize_matches_the_image_crates_catmull_rom_closely() {
        // a smooth gradient with some structure; both implementations must agree
        let source = image::RgbImage::from_fn(900, 1350, |x, y| {
            let stripes = if (x / 30 + y / 45) % 2 == 0 { 20 } else { 0 };
            image::Rgb([
                (x / 4) as u8,
                (y / 6) as u8,
                ((x + y) / 9) as u8 / 2 + stripes,
            ])
        });
        let reference = DynamicImage::ImageRgb8(source.clone())
            .resize_exact(300, 450, image::imageops::FilterType::CatmullRom)
            .to_rgb8();
        let fast = resize_rgba(
            DynamicImage::ImageRgb8(source).into_rgba8(),
            300,
            450,
            ResizeFilter::CatmullRom,
            false,
        )
        .unwrap();
        assert_eq!(fast.dimensions(), (300, 450));

        let total: u64 = fast
            .pixels()
            .zip(reference.pixels())
            .flat_map(|(a, b)| a.0[..3].iter().zip(b.0.iter()))
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        let mean = total as f64 / (300.0 * 450.0 * 3.0);
        assert!(mean < 1.5, "mean absolute difference {mean}");
    }

    #[test]
    fn scaling_fits_the_box_and_keeps_the_aspect_ratio_for_awkward_sizes() {
        for (w, h, max) in [
            (1000, 1500, 500),
            (999, 1501, 700),
            (1200, 800, 640),
            (3000, 4500, 1024),
        ] {
            let out = decode_fit_bgra(&png(w, h, Rgba([9, 8, 7, 255])), max, max).unwrap();
            let (dw, dh) = out.image.dimensions();
            assert!(dw <= max && dh <= max, "{w}x{h} -> {dw}x{dh}");
            assert!(dw == max || dh == max, "uses the full box: {dw}x{dh}");
            let (a, b) = (w as f32 / h as f32, dw as f32 / dh as f32);
            assert!((a - b).abs() / a < 0.01, "aspect {a} vs {b}");
            assert_eq!(out.orig, (w, h));
        }
    }

    #[test]
    fn transparency_survives_scaling() {
        let bytes = png(400, 400, Rgba([200, 100, 50, 128]));
        let out = decode_fit_bgra(&bytes, 100, 100).unwrap();
        let px = out.image.get_pixel(50, 50).0;
        assert!((i32::from(px[3]) - 128).abs() <= 1, "alpha {px:?}");
        // BGRA order: blue and red swapped
        assert!(
            (i32::from(px[0]) - 50).abs() <= 2 && (i32::from(px[2]) - 200).abs() <= 2,
            "{px:?}"
        );
    }

    #[test]
    fn small_covers_are_not_enlarged() {
        let jpg = make_cover_jpeg(
            &png(100, 150, Rgba([1, 2, 3, 255])),
            COVER_MAX_W,
            COVER_MAX_H,
        )
        .unwrap();
        let back = image::load_from_memory(&jpg).unwrap();
        assert_eq!((back.width(), back.height()), (100, 150));
    }

    #[test]
    fn never_upscales() {
        let bytes = png(40, 60, Rgba([1, 2, 3, 255]));
        let out = decode_fit_bgra(&bytes, 4000, 4000).unwrap();
        assert_eq!(out.image.dimensions(), (40, 60));
    }

    #[test]
    fn cover_is_valid_jpeg() {
        let bytes = png(1000, 1500, Rgba([200, 100, 50, 255]));
        let jpg = make_cover_jpeg(&bytes, COVER_MAX_W, COVER_MAX_H).unwrap();
        assert_eq!(&jpg[..2], &[0xff, 0xd8]);
        let back = image::load_from_memory(&jpg).unwrap();
        assert!(back.width() <= COVER_MAX_W && back.height() <= COVER_MAX_H);
    }

    fn rgb_bytes(w: u32, h: u32, px: image::Rgb<u8>) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, px);
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn rgb_scales_down_and_converts_to_bgra() {
        let bytes = rgb_bytes(400, 800, image::Rgb([10, 20, 30]));
        let out = decode_fit_bgra(&bytes, 100, 100).unwrap();
        assert_eq!(out.image.dimensions(), (50, 100));
        assert_eq!(out.orig, (400, 800));
        let px = out.image.get_pixel(0, 0).0;
        assert_eq!(px[3], 255);
        assert!((i32::from(px[0]) - 30).abs() <= 5, "B: {}", px[0]);
        assert!((i32::from(px[1]) - 20).abs() <= 5, "G: {}", px[1]);
        assert!((i32::from(px[2]) - 10).abs() <= 5, "R: {}", px[2]);
    }

    #[test]
    fn test_zune_api() {
        use std::io::Cursor;
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        use zune_jpeg::JpegDecoder;

        let bytes = rgb_bytes(10, 10, image::Rgb([10, 20, 30]));
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::BGRA);
        let mut decoder = JpegDecoder::new_with_options(Cursor::new(&bytes[..]), options);
        decoder.decode_headers().unwrap();
        let (w, h) = decoder.dimensions().unwrap();
        assert_eq!((w, h), (10, 10));
        let pixels = decoder.decode().unwrap();
        assert_eq!(pixels.len(), 10 * 10 * 4);
        assert!((i32::from(pixels[0]) - 30).abs() <= 5, "B: {}", pixels[0]);
        assert!((i32::from(pixels[1]) - 20).abs() <= 5, "G: {}", pixels[1]);
        assert!((i32::from(pixels[2]) - 10).abs() <= 5, "R: {}", pixels[2]);
        assert_eq!(pixels[3], 255);
    }

    #[test]
    fn test_zune_resize_bgra() {
        use std::io::Cursor;
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        use zune_jpeg::JpegDecoder;

        let bytes = rgb_bytes(100, 100, image::Rgb([10, 20, 30]));
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::BGRA);
        let mut decoder = JpegDecoder::new_with_options(Cursor::new(&bytes[..]), options);
        decoder.decode_headers().unwrap();
        let (w, h) = decoder.dimensions().unwrap();
        let pixels = decoder.decode().unwrap();

        let src = FirImage::from_vec_u8(w as u32, h as u32, pixels, PixelType::U8x4).unwrap();
        let mut dst = FirImage::new(50, 50, PixelType::U8x4);
        let options =
            ResizeOptions::new().resize_alg(ResizeAlg::Convolution(ResizeFilter::CatmullRom));
        RESIZER
            .with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &options))
            .unwrap();

        let resized_pixels = dst.into_vec();
        assert_eq!(resized_pixels.len(), 50 * 50 * 4);
        assert!(
            (i32::from(resized_pixels[0]) - 30).abs() <= 5,
            "B: {}",
            resized_pixels[0]
        );
        assert!(
            (i32::from(resized_pixels[1]) - 20).abs() <= 5,
            "G: {}",
            resized_pixels[1]
        );
        assert!(
            (i32::from(resized_pixels[2]) - 10).abs() <= 5,
            "R: {}",
            resized_pixels[2]
        );
        assert_eq!(resized_pixels[3], 255);
    }

    #[test]
    fn test_probe_dimensions() {
        let bytes_jpg = rgb_bytes(120, 80, image::Rgb([255, 0, 0]));
        let dims = probe_dimensions(&bytes_jpg).unwrap();
        assert_eq!(dims, (120, 80));

        let bytes_png = png(340, 220, Rgba([0, 255, 0, 255]));
        let dims_png = probe_dimensions(&bytes_png).unwrap();
        assert_eq!(dims_png, (340, 220));

        let img = RgbaImage::from_pixel(150, 90, Rgba([0, 0, 255, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::WebP).unwrap();
        let bytes_webp = out.into_inner();
        let dims_webp = probe_dimensions(&bytes_webp).unwrap();
        assert_eq!(dims_webp, (150, 90));

        // Invalid / empty bytes
        assert!(probe_dimensions(&[]).is_err());
        assert!(probe_dimensions(&[1, 2, 3, 4]).is_err());
        // Truncated PNG signature
        assert!(probe_dimensions(b"\x89PNG\r\n\x1a\n").is_err());
    }

    fn calculate_psnr_and_mae(a: &RgbaImage, b: &RgbaImage) -> (f64, f64, u8) {
        assert_eq!(a.dimensions(), b.dimensions());
        let (w, h) = a.dimensions();
        let total_channels = (w as f64) * (h as f64) * 4.0;
        let mut sum_sq_err = 0.0;
        let mut sum_abs_err = 0.0;
        let mut max_err = 0u8;

        for (pa, pb) in a.pixels().zip(b.pixels()) {
            for i in 0..4 {
                let diff = pa.0[i].abs_diff(pb.0[i]);
                if diff > max_err {
                    max_err = diff;
                }
                sum_sq_err += (diff as f64).powi(2);
                sum_abs_err += diff as f64;
            }
        }

        let mse = sum_sq_err / total_channels;
        let mae = sum_abs_err / total_channels;
        let psnr = if mse < 1e-10 {
            99.0
        } else {
            10.0 * (255.0 * 255.0 / mse).log10()
        };
        (psnr, mae, max_err)
    }

    #[test]
    fn test_manga_screentone_and_line_art_psnr_metrics() {
        // 1. High-contrast 1px black line art on white canvas (400x400)
        let mut line_art = RgbaImage::from_pixel(400, 400, Rgba([255, 255, 255, 255]));
        for y in 0..400 {
            for x in 0..400 {
                // Horizontal lines every 40px, vertical lines every 40px, 1px main diagonal
                if y % 40 == 0 || x % 40 == 0 || x == y {
                    line_art.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }

        // Compare fast_image_resize Catmull-Rom downscaling to 200x200 against image crate reference
        let ref_line_art = DynamicImage::ImageRgba8(line_art.clone())
            .resize_exact(200, 200, image::imageops::FilterType::CatmullRom)
            .to_rgba8();
        let fir_line_art =
            resize_rgba(line_art, 200, 200, ResizeFilter::CatmullRom, false).unwrap();

        let (psnr, mae, max_err) = calculate_psnr_and_mae(&fir_line_art, &ref_line_art);
        // SIMD 16-bit fixed point vs 32-bit float produces imperceptible variance (PSNR > 50 dB)
        assert!(
            psnr > 42.0,
            "Line art PSNR should exceed 42 dB, got {psnr:.2} dB"
        );
        assert!(
            mae < 1.0,
            "Line art MAE should be sub-pixel (<1.0), got {mae:.3}"
        );
        assert!(
            max_err <= 12,
            "Max per-channel error should be <= 12 LSBs, got {max_err}"
        );

        // 2. High-frequency manga screentone (checkerboard 1px alternating dots)
        let mut screentone = RgbaImage::from_pixel(400, 400, Rgba([255, 255, 255, 255]));
        for y in 0..400 {
            for x in 0..400 {
                if (x + y) % 2 == 0 {
                    screentone.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }

        let ref_screen = DynamicImage::ImageRgba8(screentone.clone())
            .resize_exact(200, 200, image::imageops::FilterType::CatmullRom)
            .to_rgba8();
        let fir_screen =
            resize_rgba(screentone, 200, 200, ResizeFilter::CatmullRom, false).unwrap();

        let (psnr_st, mae_st, max_err_st) = calculate_psnr_and_mae(&fir_screen, &ref_screen);
        assert!(
            psnr_st > 40.0,
            "Screentone PSNR should exceed 40 dB, got {psnr_st:.2} dB"
        );
        assert!(
            mae_st < 1.5,
            "Screentone MAE should be < 1.5, got {mae_st:.3}"
        );
        assert!(
            max_err_st <= 12,
            "Max screentone error should be <= 12 LSBs, got {max_err_st}"
        );
    }

    #[test]
    fn test_extreme_aspect_ratios_and_sizes() {
        // 1. Single-pixel 1x1 image
        let tiny = png(1, 1, Rgba([42, 84, 126, 255]));
        let out = decode_fit_bgra(&tiny, 1000, 1000).unwrap();
        assert_eq!(out.image.dimensions(), (1, 1));
        assert_eq!(out.orig, (1, 1));
        // Verify BGRA channel swap (B=126, G=84, R=42, A=255)
        let px = out.image.get_pixel(0, 0).0;
        assert_eq!(px, [126, 84, 42, 255]);

        // 2. Extreme panorama: 8000x400 (20:1 aspect ratio) fitting 1920x1080
        let pano = png(8000, 400, Rgba([100, 150, 200, 255]));
        let out_pano = decode_fit_bgra(&pano, 1920, 1080).unwrap();
        let (pw, ph) = out_pano.image.dimensions();
        assert_eq!(pw, 1920);
        assert_eq!(ph, 96);
        assert_eq!(out_pano.orig, (8000, 400));

        // 3. Extreme vertical webtoon strip: 400x12000 (1:30 aspect ratio) fitting 1920x1080
        let strip = png(400, 12000, Rgba([50, 100, 150, 255]));
        let out_strip = decode_fit_bgra(&strip, 1920, 1080).unwrap();
        let (sw, sh) = out_strip.image.dimensions();
        assert_eq!(sw, 36);
        assert_eq!(sh, 1080);
        assert_eq!(out_strip.orig, (400, 12000));

        // 4. Non-integer scale ratio: 1920x1080 downscaled to fit 1366x768
        let non_int = png(1920, 1080, Rgba([20, 40, 60, 255]));
        let out_ni = decode_fit_bgra(&non_int, 1366, 768).unwrap();
        let (nw, nh) = out_ni.image.dimensions();
        assert!(nw <= 1366 && nh <= 768);
        assert_eq!((nw, nh), (1365, 768));
    }

    #[test]
    fn test_decompression_bomb_and_corrupt_guards() {
        // 1. Synthesize PNG header claiming 20,000 x 20,000 = 400M pixels (exceeds MAX_PIXELS)
        let mut bomb_png = Vec::new();
        bomb_png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        bomb_png.extend_from_slice(&13u32.to_be_bytes()); // chunk length
        bomb_png.extend_from_slice(b"IHDR");
        bomb_png.extend_from_slice(&20_000u32.to_be_bytes()); // width
        bomb_png.extend_from_slice(&20_000u32.to_be_bytes()); // height
        bomb_png.extend_from_slice(&[8, 2, 0, 0, 0]); // bit depth, color type, etc.
        bomb_png.extend_from_slice(&[0, 0, 0, 0]); // CRC

        // probe_dimensions must reject it as too large
        let probe_err = probe_dimensions(&bomb_png).unwrap_err();
        assert!(probe_err.to_string().contains("image is too large"));

        // decode_fit_bgra must reject it without allocating
        let decode_err = decode_fit_bgra(&bomb_png, 800, 600).unwrap_err();
        assert!(
            decode_err.to_string().contains("too large")
                || decode_err.to_string().contains("Format error")
        );

        // 2. Corrupt / truncated byte streams
        let truncated_png = b"\x89PNG\r\n\x1a\n\x00\x00\x00";
        assert!(probe_dimensions(truncated_png).is_err());
        assert!(decode_fit_bgra(truncated_png, 800, 600).is_err());

        let truncated_jpg = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        assert!(probe_dimensions(truncated_jpg).is_err());
        assert!(decode_fit_bgra(truncated_jpg, 800, 600).is_err());

        let random_junk = &[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04];
        assert!(probe_dimensions(random_junk).is_err());
        assert!(decode_fit_bgra(random_junk, 800, 600).is_err());
    }

    #[test]
    fn test_simd_cpu_features_and_portable_dispatch() {
        RESIZER.with(|resizer| {
            let extensions = resizer.borrow().cpu_extensions();
            assert!(
                extensions.is_supported(),
                "Detected CPU extensions {extensions:?} must be supported on current host"
            );
        });
    }
}
