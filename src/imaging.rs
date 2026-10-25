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

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    if let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()
        && let Ok((w, h)) = reader.into_dimensions()
        && u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        bail!("image is too large ({w}x{h})");
    }
    let img = image::load_from_memory(bytes)?;
    if u64::from(img.width()) * u64::from(img.height()) > MAX_PIXELS {
        bail!("image is too large ({}x{})", img.width(), img.height());
    }
    Ok(img)
}

/// A decoded page.
pub struct Decoded {
    /// Pixels in **BGRA** order, already scaled down to the requested size.
    pub image: RgbaImage,
    /// Dimensions of the original, unscaled image.
    pub orig: (u32, u32),
}

fn rgb_to_bgra(rgb: &[u8], nw: u32, nh: u32) -> Result<RgbaImage> {
    let total = nw as usize * nh as usize;
    let mut bgra = vec![0u8; total * 4];
    for (src, dst) in rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(bgra.as_chunks_mut::<4>().0)
    {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = 255;
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
        let src = FirImage::from_vec_u8(w, h, pixels, PixelType::U8x4)?;
        let mut dst = FirImage::new(nw, nh, PixelType::U8x4);
        let resize_options =
            ResizeOptions::new().resize_alg(ResizeAlg::Convolution(ResizeFilter::CatmullRom));
        RESIZER.with(|resizer| resizer.borrow_mut().resize(&src, &mut dst, &resize_options))?;
        RgbaImage::from_raw(nw, nh, dst.into_vec()).context("resized buffer has the wrong size")?
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
    if bytes.starts_with(&[0xff, 0xd8])
        && let Ok(decoded) = decode_jpeg_fit_bgra(bytes, max_w, max_h)
    {
        return Ok(decoded);
    }

    let img = decode(bytes)?;
    let (w, h) = (img.width(), img.height());
    let alpha = img.color().has_alpha();
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);

    let bgra = if alpha {
        let mut rgba = img.into_rgba8();
        if scale < 1.0 {
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            rgba = resize_rgba(rgba, nw, nh, ResizeFilter::CatmullRom, true)?;
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
            let resized = resize_rgb(rgb, nw, nh, ResizeFilter::CatmullRom)?;
            rgb_to_bgra(resized.as_raw(), nw, nh)?
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
    if bytes.starts_with(&[0xff, 0xd8])
        && let Ok(cover) = make_jpeg_cover(bytes, max_w, max_h)
    {
        return Ok(cover);
    }

    let img = decode(bytes)?;
    let (w, h) = (img.width(), img.height());
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
}
