//! PNG and JPEG output for graphviz-pure (`-Tpng`, `-Tjpg`).
//!
//! C Graphviz draws raster formats with cairo and measures text with Pango. The port does
//! neither: it lays out and renders SVG exactly as C does with its built-in font tables, and
//! this crate rasterizes that SVG with [resvg]. So the layout of a PNG is the port's (and C's,
//! under the reference setup), and the pixels are resvg's: antialiasing and glyph shapes are not
//! cairo's.
//!
//! Text is drawn with fonts compiled into the crate (`fonts/`, SIL Open Font License 1.1): Tinos,
//! Arimo and Cousine, which have the metrics of Times New Roman, Arial and Courier New, the
//! families the port measures labels with. Graphviz's names for them (`Times-Roman`,
//! `Helvetica`, `Courier`, ...) resolve to these faces; any other name falls back to Tinos, as
//! the port's text measurement falls back to Times. No system font is read, so a PNG is the same
//! on every machine.
//!
//! Images (`image`, `shapefile`, `<IMG>`) are not drawn: the SVG's `<image>` elements are
//! left out, and no file is read. The C reference build has no raster output to draw them
//! with, and a C build with cairo loads them with plugins the port does not have.
//!
//! Pixel size follows Graphviz: without a `dpi` (or `resolution`) attribute, raster output is
//! at 96 dpi (1pt = 4/3 px); with one, the SVG is already scaled to that resolution and is
//! rasterized at one pixel per point.

pub use graphviz_pure_core::{
    Diagnostic, DiagnosticLevel, GraphvizError, GraphvizErrorKind, Rendered,
};
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};

/// A raster output format.
///
/// Non-exhaustive, so a new format can be added in a minor release. A `match`
/// outside this crate needs a wildcard arm.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RasterFormat {
    Png,
    Jpeg,
}

impl RasterFormat {
    /// Graphviz's `-T` names for the formats this crate writes.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" | "jpe" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

const FONTS: &[(&[u8], &[&str])] = &[
    (
        include_bytes!("../fonts/Tinos-Regular.ttf"),
        &["Times", "Times-Roman", "Times New Roman"],
    ),
    (
        include_bytes!("../fonts/Tinos-Bold.ttf"),
        &["Times", "Times-Roman", "Times New Roman"],
    ),
    (
        include_bytes!("../fonts/Tinos-Italic.ttf"),
        &["Times", "Times-Roman", "Times New Roman"],
    ),
    (
        include_bytes!("../fonts/Tinos-BoldItalic.ttf"),
        &["Times", "Times-Roman", "Times New Roman"],
    ),
    (
        include_bytes!("../fonts/Arimo[wght].ttf"),
        &["Helvetica", "Arial"],
    ),
    (
        include_bytes!("../fonts/Arimo-Italic[wght].ttf"),
        &["Helvetica", "Arial"],
    ),
    (
        include_bytes!("../fonts/Cousine-Regular.ttf"),
        &["Courier", "Courier New"],
    ),
    (
        include_bytes!("../fonts/Cousine-Bold.ttf"),
        &["Courier", "Courier New"],
    ),
    (
        include_bytes!("../fonts/Cousine-Italic.ttf"),
        &["Courier", "Courier New"],
    ),
    (
        include_bytes!("../fonts/Cousine-BoldItalic.ttf"),
        &["Courier", "Courier New"],
    ),
];

/// The bundled fonts, with Graphviz's family names added as aliases.
fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        for (data, aliases) in FONTS {
            let source = usvg::fontdb::Source::Binary(Arc::new(data.to_vec()));
            for id in db.load_font_source(source) {
                let Some(mut info) = db.face(id).cloned() else {
                    continue;
                };
                db.remove_face(id);
                let lang = info
                    .families
                    .first()
                    .map_or(usvg::fontdb::Language::English_UnitedStates, |f| f.1);
                for a in *aliases {
                    info.families.push(((*a).to_string(), lang));
                }
                db.push_face_info(info);
            }
        }
        db.set_serif_family("Tinos");
        db.set_sans_serif_family("Arimo");
        db.set_monospace_family("Cousine");
        db.set_cursive_family("Tinos");
        db.set_fantasy_family("Tinos");
        Arc::new(db)
    })
    .clone()
}

/// Rasterizes the port's SVG for one graph. `explicit_dpi` says whether the graph set `dpi`
/// or `resolution` (then the SVG is already scaled and is drawn at one pixel per point).
///
/// The bytes are [`Rendered<Vec<u8>>`]. Diagnostics from the rasterizing call are
/// on that value, or on the error. This writes nothing to stderr.
///
/// ```
/// use graphviz_pure_raster::{RasterFormat, rasterize};
///
/// let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;
/// let png = rasterize(svg, RasterFormat::Png, false)
///     .unwrap()
///     .into_output();
/// assert_eq!(&png[1..4], b"PNG");
/// ```
pub fn rasterize(
    svg: &str,
    format: RasterFormat,
    explicit_dpi: bool,
) -> Result<Rendered<Vec<u8>>, GraphvizError> {
    let (result, diagnostics) =
        graphviz_pure_core::diag::capture(|| rasterize_inner(svg, format, explicit_dpi));
    match result {
        Ok(output) => Ok(Rendered::__from_parts(output, diagnostics)),
        Err(error) => Err(error.__set_diagnostics(diagnostics)),
    }
}

/// Encodes straight (not premultiplied) RGBA pixels as a PNG.
///
/// This is what `tiny_skia`'s `PixmapRef::encode_png` does, on pixels that are not in a
/// pixmap: the same `png` calls on the same pixels give the same bytes.
fn encode_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, png::EncodingError> {
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixels)?;
    }
    Ok(data)
}

/// Draws the SVG and returns the picture.
///
/// `explicit_dpi` says whether the graph set `dpi` or `resolution` (then the SVG is
/// already scaled and is drawn at one pixel per point).
fn render_pixmap(svg: &str, explicit_dpi: bool) -> Result<tiny_skia::Pixmap, GraphvizError> {
    let opt = usvg::Options {
        fontdb: fontdb(),
        font_family: "Tinos".to_string(),
        // no `<image>` is resolved: neither a file nor a `data:` URL
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &opt)
        .map_err(|e| GraphvizError::render(format!("rasterizing the SVG: {e}")))?;
    // usvg reads the SVG's `pt` sizes as CSS pixels at 96 dpi (1pt = 4/3 px), Graphviz's default
    // raster resolution. With an explicit dpi the SVG's points are already the pixels.
    let scale = if explicit_dpi { 0.75 } else { 1.0 };
    let size = tree.size();
    let (w, h) = (
        (size.width() * scale).round().max(1.0) as u32,
        (size.height() * scale).round().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| GraphvizError::render("the drawing is too large to rasterize"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

fn rasterize_inner(
    svg: &str,
    format: RasterFormat,
    explicit_dpi: bool,
) -> Result<Vec<u8>, GraphvizError> {
    let pixmap = render_pixmap(svg, explicit_dpi)?;
    let (w, h) = (pixmap.width(), pixmap.height());
    match format {
        // `Pixmap::encode_png` copies the whole pixmap into a second buffer before it
        // encodes it, which for a big drawing is a copy of every pixel. The pixels are
        // taken out of the pixmap instead and encoded where they are; the bytes are the
        // same (the same `png` calls on the same pixels).
        RasterFormat::Png => {
            let pixels = pixmap.take_demultiplied();
            encode_png(&pixels, w, h)
                .map_err(|e| GraphvizError::render(format!("encoding the PNG: {e}")))
        }
        RasterFormat::Jpeg => {
            // JPEG has no alpha: composite onto white, as Graphviz's background is.
            let mut rgb = Vec::with_capacity((w * h * 3) as usize);
            for p in pixmap.pixels() {
                let c = p.demultiply();
                let a = u32::from(c.alpha());
                let over = |v: u8| ((u32::from(v) * a + 255 * (255 - a) + 127) / 255) as u8;
                rgb.extend([over(c.red()), over(c.green()), over(c.blue())]);
            }
            let mut out = Vec::new();
            jpeg_encoder::Encoder::new(&mut out, 90)
                .encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| GraphvizError::render(format!("encoding the JPEG: {e}")))?;
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r#"<svg width="62pt" height="116pt" viewBox="0.00 0.00 62.00 116.00" xmlns="http://www.w3.org/2000/svg">
<g transform="scale(1 1) rotate(0) translate(4 112)"><polygon fill="white" stroke="none" points="-4,4 -4,-112 58,-112 58,4 -4,4"/>
<ellipse fill="none" stroke="black" cx="27" cy="-90" rx="27" ry="18"/>
<text text-anchor="middle" x="27" y="-85.8" font-family="Times,serif" font-size="14.00">a</text></g></svg>"#;

    #[test]
    fn png_has_graphviz_default_size() {
        let png = rasterize(SVG, RasterFormat::Png, false).unwrap();
        assert_eq!(&png.output()[1..4], b"PNG");
        // IHDR width and height: 62pt and 116pt at 96 dpi, as C's -Tpng.
        assert_eq!(
            u32::from_be_bytes(png.output()[16..20].try_into().unwrap()),
            83
        );
        assert_eq!(
            u32::from_be_bytes(png.output()[20..24].try_into().unwrap()),
            155
        );
    }

    #[test]
    fn explicit_dpi_draws_one_pixel_per_point() {
        let png = rasterize(SVG, RasterFormat::Png, true).unwrap();
        assert_eq!(
            u32::from_be_bytes(png.output()[16..20].try_into().unwrap()),
            62
        );
    }

    // The PNG the raster path writes is the one `tiny_skia`'s `Pixmap::encode_png`
    // writes for the same drawing: the pixels are encoded where they are drawn,
    // without a copy of the whole pixmap first. Both sides demultiply the pixels
    // the same way, so what this pins is the encoder.
    //
    // The drawing is the one the raster path draws, so the two cannot drift; a
    // drawing with a transparent background covers the pixels that are not opaque.
    #[test]
    fn png_bytes_match_the_pixmap_encoder() {
        const TRANSPARENT: &str = r#"<svg width="62pt" height="116pt" viewBox="0.00 0.00 62.00 116.00" xmlns="http://www.w3.org/2000/svg">
<g transform="scale(1 1) rotate(0) translate(4 112)"><ellipse fill="none" stroke="black" cx="27" cy="-90" rx="27" ry="18"/>
<text text-anchor="middle" x="27" y="-85.8" font-family="Times,serif" font-size="14.00">a</text></g></svg>"#;
        for svg in [SVG, TRANSPARENT] {
            for explicit_dpi in [false, true] {
                let reference = render_pixmap(svg, explicit_dpi)
                    .unwrap()
                    .encode_png()
                    .unwrap();
                let ours = rasterize(svg, RasterFormat::Png, explicit_dpi).unwrap();
                assert_eq!(ours.output(), &reference);
            }
        }
    }

    #[test]
    fn graphviz_font_names_resolve_to_the_bundled_faces() {
        let db = fontdb();
        for name in [
            "Times",
            "Times-Roman",
            "Helvetica",
            "Arial",
            "Courier",
            "Courier New",
        ] {
            let q = usvg::fontdb::Query {
                families: &[usvg::fontdb::Family::Name(name)],
                ..Default::default()
            };
            assert!(db.query(&q).is_some(), "{name}");
        }
    }

    // #347: an `<image>` the SVG writer wrote for `image=` is not drawn, nor its file read
    // (resvg draws SVG images; it is built without raster-image decoding)
    #[test]
    fn images_are_not_drawn() {
        const RED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
        const RED64: &str = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSJyZWQiLz48L3N2Zz4=";
        let with = |href: &str| {
            SVG.replace(
                "<ellipse",
                &format!(
                    "<image xlink:href=\"{href}\" width=\"54px\" height=\"36px\" preserveAspectRatio=\"none\" x=\"0\" y=\"-108\"/>\n<ellipse"
                ),
            )
            .replace(
                "xmlns=",
                "xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns=",
            )
        };
        let plain = rasterize(SVG, RasterFormat::Png, false).unwrap();
        let data = with(&format!("data:image/svg+xml;base64,{RED64}"));
        assert_eq!(rasterize(&data, RasterFormat::Png, false).unwrap(), plain);
        let dir = std::env::temp_dir().join(format!("gvp-raster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("red.svg");
        std::fs::write(&file, RED).unwrap();
        let svg = with(&file.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(rasterize(&svg, RasterFormat::Png, false).unwrap(), plain);
    }

    #[test]
    fn jpeg_encodes() {
        let jpg = rasterize(SVG, RasterFormat::Jpeg, false).unwrap();
        assert_eq!(&jpg.output()[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn invalid_svg_is_a_render_error() {
        let err = rasterize("<not-svg>", RasterFormat::Png, false).unwrap_err();
        assert_eq!(err.kind(), GraphvizErrorKind::Render);
        assert_eq!(
            err.to_string(),
            format!("Rendering error: {}", err.message())
        );
        assert!(
            err.to_string()
                .starts_with("Rendering error: rasterizing the SVG: "),
            "{err}"
        );
    }

    // #339. `Vec2::atan2` is `y.atan2(x)`. Unpatched on Apple Silicon this input is
    // `0x3fe4978fa3269ee1` from the platform libm and `0x3fe4978fa3269ee0` from
    // `libm` 0.2.16, which is what `graphviz_pure_core::math::atan2` calls.
    #[test]
    fn issue_339_atan2_matches_the_libm_crate() {
        let angle = kurbo::Vec2::new(0.4, 0.3).atan2();
        assert_eq!(
            angle.to_bits(),
            graphviz_pure_core::math::atan2(0.3, 0.4).to_bits(),
        );
    }

    // Ellipse arcs, rotate, skew and a cubic exercise the patched sin, cos, tan,
    // acos and powf calls. A one-ulp difference does not move an 8-bit pixel, so
    // this PNG matches the unpatched render on Apple Silicon. The hash is the
    // value the patched crates produce, which the other platforms must match.
    #[test]
    fn issue_339_png_bytes_cover_ellipse_rotate_and_cubic() {
        use sha2::{Digest, Sha256};
        const SHAPES: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="90pt" height="70pt" viewBox="0 0 90 70">
<g transform="rotate(33 40 36) skewX(20)">
<ellipse cx="40" cy="30" rx="24" ry="14" fill="none" stroke="black"/>
<path d="M10 60 C 25 8, 70 8, 85 55" fill="none" stroke="black"/>
<text x="18" y="40" font-family="Times,serif" font-size="16">Été</text>
</g></svg>"#;
        let png = rasterize(SHAPES, RasterFormat::Png, false).unwrap();
        let hash: String = Sha256::digest(png.output())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hash,
            "30c8424e7946b977c41de493bdfd569c9bca183405e95584148b644417995d51"
        );
    }
}
