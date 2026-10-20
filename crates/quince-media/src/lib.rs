//! Turning what Apple stores into what the interface draws.
//!
//! Apple keeps a contact's picture at full size and records, separately, the square its owner
//! chose to show. Contacts on a phone draws that square; anything that ignores it shows the
//! middle of the original instead — which for a portrait photo is a chest, not a face. So the
//! square is cut here, in Rust, and the interface is handed a small image that is already
//! right.

/// The square of a picture its owner chose to show, in the stored image's own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Crop {
    /// Reads the compact form kept in the library: `x,y,width,height`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(|p| p.trim().parse::<i64>().ok());
        let (x, y, width, height) = (
            parts.next()??,
            parts.next()??,
            parts.next()??,
            parts.next()??,
        );
        if parts.next().is_some() || width <= 0 || height <= 0 {
            return None;
        }
        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub fn to_text(self) -> String {
        format!("{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

/// A picture cut to its chosen square and sized for display.
///
/// `size` is the edge of the result. Serving 256 rather than the stored 1696×2528 is most of
/// why a list of faces scrolls: the webview decodes a few kilobytes per row instead of a few
/// hundred.
///
/// # Errors
/// When the bytes are not an image this can decode.
pub fn avatar(bytes: &[u8], crop: Option<Crop>, size: u32) -> Result<Vec<u8>, image::ImageError> {
    let image = image::load_from_memory(bytes)?;
    let square = cut(image, crop);
    // Lanczos: these are faces, and a cheaper filter shows on skin.
    let scaled = square.resize_to_fill(size, size, image::imageops::FilterType::Lanczos3);
    encode(&scaled)
}

/// Cuts out the chosen square, ignoring a rectangle that does not fit the picture it came with.
///
/// A crop is only trusted where it actually lands inside the image. Apple's own API has been
/// seen to send fields that do not match what it serves, and a bad rectangle should cost a
/// centred picture, never a panic or a blank one.
fn cut(image: image::DynamicImage, crop: Option<Crop>) -> image::DynamicImage {
    use image::GenericImageView;
    let (w, h) = image.dimensions();
    let Some(c) = crop else { return image };
    let (Ok(x), Ok(y), Ok(cw), Ok(ch)) = (
        u32::try_from(c.x),
        u32::try_from(c.y),
        u32::try_from(c.width),
        u32::try_from(c.height),
    ) else {
        return image;
    };
    if cw == 0 || ch == 0 || x.saturating_add(cw) > w || y.saturating_add(ch) > h {
        return image;
    }
    // Apple's crop is in the Mac's coordinates: measured from the bottom-left corner, `y`
    // rising up the picture. Read from the top, it drew the wrong band — measured on a real
    // card, 1696×2528 with a 1279 square at y = 1249: from the top that is the bottom half,
    // the body; from the bottom it is the top square, the face, as the phone shows it.
    let top = h - y - ch;
    image.crop_imm(x, top, cw, ch)
}

/// PNG where the picture needs transparency, JPEG where it does not.
///
/// A photograph re-encoded as PNG is several times the size for no visible gain, and these are
/// read back on every scroll.
/// A HEIC (or HEIF) original as a high-quality JPEG, turned, cropped and mirrored as the file
/// says. Apple keeps no full-size JPEG of any HEIC photo (none of 495 in his library) and, for
/// screenshots and the like, often nothing larger than a thumbnail: without this, the sharpest a
/// HEIC photo could be shown was 2048 pixels, and some only a thumbnail stretched — or a broken
/// picture where none could be made. Decoded here from the original, as Photos on a Mac does.
///
/// # Errors
/// When the bytes are not a HEIF image libheif can decode.
pub fn heic_to_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes).map_err(|e| e.to_string())?;
    let handle = ctx.primary_image_handle().map_err(|e| e.to_string())?;
    let image = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(|e| e.to_string())?;
    let plane = image
        .planes()
        .interleaved
        .ok_or("the picture had no colour plane")?;
    let (w, h, stride) = (plane.width, plane.height, plane.stride);
    let row = w as usize * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        rgb.extend_from_slice(&plane.data[y * stride..y * stride + row]);
    }
    let picture = image::RgbImage::from_raw(w, h, rgb).ok_or("the picture had the wrong size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&picture)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// RGB rows (`stride` bytes apart) made a JPEG, as heic_to_jpeg makes one.
///
/// # Errors
/// When the rows are shorter than the size says.
pub fn rgb_to_jpeg(data: &[u8], w: u32, h: u32, stride: usize) -> Result<Vec<u8>, String> {
    let row = w as usize * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        rgb.extend_from_slice(
            data.get(y * stride..y * stride + row)
                .ok_or("the frame was cut short")?,
        );
    }
    let picture = image::RgbImage::from_raw(w, h, rgb).ok_or("the picture had the wrong size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&picture)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// How light RGB rows are, 0–255 on average: a frame of nothing (a recording's black first
/// frame) is near 0.
#[must_use]
pub fn rgb_lightness(data: &[u8], w: u32, h: u32, stride: usize) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    // Every fourth pixel of every fourth row is plenty to tell a picture from black.
    for y in (0..h as usize).step_by(4) {
        let Some(line) = data.get(y * stride..y * stride + w as usize * 3) else {
            break;
        };
        for px in line.as_chunks::<3>().0.iter().step_by(4) {
            sum += u64::from(px[0]) * 3 + u64::from(px[1]) * 6 + u64::from(px[2]);
            n += 10;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

/// Whether a picture shows nothing: every part of it black, as Apple's poster for a video
/// whose first frame is black (a screen recording's) is — 0.0 on average, to the last pixel.
#[must_use]
pub fn is_blank(bytes: &[u8]) -> bool {
    let Ok(picture) = image::load_from_memory(bytes) else {
        return false;
    };
    let gray = picture.to_luma8();
    let brightest = gray.pixels().map(|p| p.0[0]).max().unwrap_or(0);
    brightest < 24
}

/// A video's turn as its file records it: the picture track's matrix (QuickTime/MP4 `tkhd`) as
/// `[a, b, c, d]`, each -1, 0 or 1, and where in the file the matrix's 36 bytes are. A phone
/// records upright video sideways and says how to turn it here; the front camera's is also a
/// mirror (`[0, 1, 1, 0]`). `None` for a file with no picture track or no such header.
///
/// # Errors
/// When the file cannot be read.
pub fn video_matrix(path: &std::path::Path) -> std::io::Result<Option<(u64, [i32; 4])>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    // The `moov` box, wherever among the top-level boxes it is (after the film, often).
    let mut at = 0u64;
    let (moov_at, moov) = loop {
        if at + 8 > len {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; 16];
        let got = file.read(&mut head)?;
        if got < 8 {
            return Ok(None);
        }
        let mut size = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
        let kind = &head[4..8];
        if size == 1 && got >= 16 {
            size = u64::from_be_bytes(head[8..16].try_into().unwrap_or_default());
        } else if size == 0 {
            size = len - at;
        }
        if size < 8 {
            return Ok(None);
        }
        if kind == b"moov" {
            // Its tables only: a few megabytes at most for the longest film.
            if size > 64 * 1024 * 1024 {
                return Ok(None);
            }
            let mut moov = vec![0u8; usize::try_from(size).unwrap_or(0)];
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut moov)?;
            break (at, moov);
        }
        at += size;
    };
    let boxes = |from: usize, to: usize| {
        let mut out = Vec::new();
        let mut at = from;
        while at + 8 <= to {
            let size =
                u32::from_be_bytes([moov[at], moov[at + 1], moov[at + 2], moov[at + 3]]) as usize;
            if size < 8 || at + size > to {
                break;
            }
            out.push((&moov[at + 4..at + 8], at, size));
            at += size;
        }
        out
    };
    for (kind, trak, size) in boxes(8, moov.len()) {
        if kind != b"trak" {
            continue;
        }
        for (kind, tkhd, tsize) in boxes(trak + 8, trak + size) {
            if kind != b"tkhd" || tsize < 92 {
                continue;
            }
            let body = tkhd + 8;
            let matrix_at = body + if moov[body] == 1 { 52 } else { 40 };
            if matrix_at + 44 > tkhd + tsize {
                continue;
            }
            let word = |i: usize| {
                i32::from_be_bytes(
                    moov[matrix_at + i * 4..matrix_at + i * 4 + 4]
                        .try_into()
                        .unwrap_or_default(),
                )
            };
            // A picture track has a size; a sound track's is 0 by 0.
            if word(9) == 0 {
                continue;
            }
            let unit = |v: i32| (f64::from(v) / 65536.0).round().clamp(-1.0, 1.0) as i32;
            let m = [unit(word(0)), unit(word(1)), unit(word(3)), unit(word(4))];
            return Ok(Some((moov_at + matrix_at as u64, m)));
        }
    }
    Ok(None)
}

/// The 36 bytes of an upright matrix: what a film is served with once its turn is drawn by the
/// window instead (see `video_matrix`).
pub const UPRIGHT_MATRIX: [u8; 36] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0,
];

/// Whether bytes are a HEIF file (HEIC and its kin): an `ftyp` box naming a HEIF brand.
#[must_use]
pub fn is_heif(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp")
        && bytes.get(8..12).is_some_and(|b| {
            matches!(
                b,
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
            )
        })
}

/// A picture's longer side, read from its header alone (nothing decoded).
#[must_use]
pub fn long_side(bytes: &[u8]) -> Option<u32> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .map(|(w, h)| w.max(h))
}

/// A photo made the size it is shown at: its longer side `long` pixels (never larger than it
/// is), turned the way its camera said, resampled with Lanczos.
///
/// WebKitGTK shrinks a large picture by sampling it, with no smoothing first: a 4032-pixel
/// photo drawn 1000 wide came out with broken letters and moiré (its own output, compared with
/// an ideal resample). Drawn at its own size, a picture is exactly as sharp as it can be. This
/// is what the viewer and the grid show; the whole picture only when zoomed in past it.
///
/// `None` when the picture is already no larger: then the original is the one to show.
///
/// # Errors
/// When the bytes are not an image this can decode.
pub fn fit(bytes: &[u8], long: u32) -> Result<Option<Vec<u8>>, image::ImageError> {
    use fast_image_resize as fr;
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let (w, h) = decoder.dimensions();
    if w.max(h) <= long {
        return Ok(None);
    }
    let mut picture = image::DynamicImage::from_decoder(decoder)?;
    picture.apply_orientation(orientation);
    let (w, h) = (picture.width(), picture.height());
    let k = f64::from(long) / f64::from(w.max(h));
    let (tw, th) = (
        ((f64::from(w) * k).round() as u32).max(1),
        ((f64::from(h) * k).round() as u32).max(1),
    );
    let alpha = transparent(&picture);
    let (pixels, kind) = if alpha {
        (picture.to_rgba8().into_raw(), fr::PixelType::U8x4)
    } else {
        (picture.to_rgb8().into_raw(), fr::PixelType::U8x3)
    };
    let to_err = |e: String| {
        image::ImageError::Parameter(image::error::ParameterError::from_kind(
            image::error::ParameterErrorKind::Generic(e),
        ))
    };
    let src =
        fr::images::Image::from_vec_u8(w, h, pixels, kind).map_err(|e| to_err(e.to_string()))?;
    let mut dst = fr::images::Image::new(tw, th, kind);
    fr::Resizer::new()
        .resize(
            &src,
            &mut dst,
            &fr::ResizeOptions::new()
                .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3)),
        )
        .map_err(|e| to_err(e.to_string()))?;
    let mut out = std::io::Cursor::new(Vec::new());
    if alpha {
        image::RgbaImage::from_raw(tw, th, dst.into_vec())
            .ok_or_else(|| to_err("resized picture had the wrong size".into()))?
            .write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        let rgb = image::RgbImage::from_raw(tw, th, dst.into_vec())
            .ok_or_else(|| to_err("resized picture had the wrong size".into()))?;
        // High quality: this is the picture people look at, not a row's avatar.
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&rgb)?;
    }
    Ok(Some(out.into_inner()))
}

fn encode(image: &image::DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut out = std::io::Cursor::new(Vec::new());
    if transparent(image) {
        image.write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        let rgb = image.to_rgb8();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
        encoder.encode_image(&rgb)?;
    }
    Ok(out.into_inner())
}

fn transparent(image: &image::DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|p| p.0[3] != 255)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_photo_is_made_its_shown_size_turned_the_camera_s_way() {
        // 400×300 with a red left half; JPEG.
        let img = image::RgbImage::from_fn(400, 300, |x, _| {
            if x < 200 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut jpg, image::ImageFormat::Jpeg)
            .unwrap();
        let out = super::fit(jpg.get_ref(), 200).unwrap().unwrap();
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (200, 150));
        let left = back.to_rgb8().get_pixel(20, 75).0;
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        // Never made larger than it is.
        assert!(super::fit(jpg.get_ref(), 800).unwrap().is_none());
    }

    use super::*;
    use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

    /// A tall picture with a red square at its top right — where a face sits in a portrait.
    fn portrait() -> Vec<u8> {
        let mut img = RgbaImage::from_pixel(100, 200, Rgba([0, 0, 255, 255]));
        for y in 0..50 {
            for x in 50..100 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn middle_pixel(bytes: &[u8]) -> [u8; 3] {
        let img = image::load_from_memory(bytes).unwrap();
        let p = img.get_pixel(img.width() / 2, img.height() / 2);
        [p.0[0], p.0[1], p.0[2]]
    }

    /// The whole point: a portrait photo cropped where its owner said, not through the middle.
    /// `y` is from the bottom, as Apple measures it: 150 up a 200-tall picture, a 50 square
    /// is the top one — the face — and read from the top it would have been the bottom.
    #[test]
    fn the_chosen_square_is_what_gets_drawn() {
        let source = portrait();
        let chosen = Crop {
            x: 50,
            y: 150,
            width: 50,
            height: 50,
        };

        let cropped = avatar(&source, Some(chosen), 64).unwrap();
        assert!(
            middle_pixel(&cropped)[0] > 200,
            "the chosen square is the red one"
        );

        let centred = avatar(&source, None, 64).unwrap();
        assert!(
            middle_pixel(&centred)[2] > 200,
            "with no crop it centres, which is why the crop matters"
        );
    }

    #[test]
    fn the_result_is_the_size_that_was_asked_for() {
        let out = avatar(&portrait(), None, 48).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(img.dimensions(), (48, 48));
    }

    /// Apple's own API sends fields that do not match what it serves. A rectangle falling off
    /// the picture must cost a centred face, not a crash or an empty frame.
    #[test]
    fn a_rectangle_that_does_not_fit_is_ignored_rather_than_obeyed() {
        for bad in [
            Crop {
                x: 0,
                y: 0,
                width: 500,
                height: 500,
            },
            Crop {
                x: 90,
                y: 190,
                width: 50,
                height: 50,
            },
            Crop {
                x: -10,
                y: 0,
                width: 20,
                height: 20,
            },
        ] {
            let out = avatar(&portrait(), Some(bad), 32).unwrap();
            assert_eq!(
                image::load_from_memory(&out).unwrap().dimensions(),
                (32, 32)
            );
        }
    }

    #[test]
    fn a_crop_survives_the_round_trip_through_the_library() {
        let c = Crop {
            x: 251,
            y: 1249,
            width: 1279,
            height: 1279,
        };
        assert_eq!(Crop::parse(&c.to_text()), Some(c));
        assert_eq!(Crop::parse("1,2,3"), None);
        assert_eq!(Crop::parse("1,2,3,4,5"), None);
        assert_eq!(Crop::parse("1,2,0,4"), None, "an empty square is no square");
        assert_eq!(Crop::parse("a,b,c,d"), None);
    }

    /// A photograph should not be shipped as a PNG; a picture with holes in it must be.
    #[test]
    fn the_format_follows_what_the_picture_needs() {
        let opaque = avatar(&portrait(), None, 64).unwrap();
        assert_eq!(&opaque[..3], b"\xff\xd8\xff", "opaque becomes a JPEG");

        let mut img = RgbaImage::from_pixel(20, 20, Rgba([0, 0, 0, 0]));
        img.put_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let mut src = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut src, image::ImageFormat::Png)
            .unwrap();
        let holes = avatar(&src.into_inner(), None, 64).unwrap();
        assert_eq!(&holes[..4], b"\x89PNG", "transparency stays a PNG");
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_an_error_not_a_panic() {
        assert!(avatar(b"404 Not Found", None, 64).is_err());
        assert!(avatar(b"", None, 64).is_err());
    }
}
