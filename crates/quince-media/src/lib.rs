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
