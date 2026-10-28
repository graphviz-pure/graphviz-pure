//! `imagetype` and the size readers of `lib/gvc/gvusershape.c` (`png_size`, `gif_size`,
//! `jpeg_size`, `bmp_size`, `ico_size`, `webp_size`, `svg_size`, `ps_size`, `pdf_size`).
//!
//! They read headers only, through [`CFile`], which behaves as the C stdio calls they
//! make: `fgetc` past the end returns `EOF`, `fseek` may go past the end, and an `fseek`
//! before the start fails and leaves the position.

use std::io::{self, BufReader, Read, Seek, SeekFrom};

/// A `FILE *` opened for reading, with the stdio calls the readers use.
pub(super) struct CFile<R> {
    r: Option<BufReader<R>>,
}

impl<R: Read + Seek> CFile<R> {
    pub(super) fn new(r: R) -> Self {
        CFile {
            r: Some(BufReader::new(r)),
        }
    }

    /// A stream from which every read fails (`fopen` of a directory).
    pub(super) fn unreadable() -> Self {
        CFile { r: None }
    }

    /// `fgetc`: the next byte, or `None` for `EOF`.
    fn getc(&mut self) -> Option<u8> {
        let r = self.r.as_mut()?;
        let mut b = [0u8];
        loop {
            match r.read(&mut b) {
                Ok(0) => return None,
                Ok(_) => return Some(b[0]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
    }

    /// `ungetc` of the byte just read.
    fn ungetc(&mut self) {
        let _ = self.seek_cur(-1);
    }

    /// `fread(buf, 1, n, f)`.
    pub(super) fn read_upto(&mut self, n: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(r) = self.r.as_mut() {
            let _ = r.by_ref().take(n as u64).read_to_end(&mut buf);
        }
        buf
    }

    /// `rewind`.
    fn rewind(&mut self) {
        self.seek_set(0);
    }

    /// `fseek(f, off, SEEK_SET)`.
    fn seek_set(&mut self, off: u64) {
        if let Some(r) = self.r.as_mut() {
            let _ = r.seek(SeekFrom::Start(off));
        }
    }

    /// `fseek(f, off, SEEK_CUR) == 0`: a position before the start fails, and leaves the
    /// position where it was.
    fn seek_cur(&mut self, off: i64) -> bool {
        self.r
            .as_mut()
            .is_some_and(|r| r.seek_relative(off).is_ok())
    }

    /// `fgets(line, BUFSIZ, f)` (`BUFSIZ` is 1024 on the reference platform): up to 1023
    /// bytes, through the first newline; `None` at the end of the file.
    fn gets(&mut self) -> Option<Vec<u8>> {
        const BUFSIZ: usize = 1024;
        let mut line = Vec::new();
        while line.len() < BUFSIZ - 1 {
            match self.getc() {
                Some(c) => {
                    line.push(c);
                    if c == b'\n' {
                        break;
                    }
                }
                None => break,
            }
        }
        (!line.is_empty()).then_some(line)
    }
}

/// `imagetype_t`, with the `stringtype` C gives each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Null,
    Png,
    Ps,
    Bmp,
    Gif,
    Jpeg,
    Pdf,
    Eps,
    Xml,
    Svg,
    Riff,
    Webp,
    Ico,
}

impl ImageType {
    /// `us->stringtype`: the first half of the loadimage plugin's name (`png:svg`).
    pub fn stringtype(self) -> &'static str {
        match self {
            ImageType::Null => "(lib)",
            ImageType::Png => "png",
            ImageType::Ps => "ps",
            ImageType::Bmp => "bmp",
            ImageType::Gif => "gif",
            ImageType::Jpeg => "jpeg",
            ImageType::Pdf => "pdf",
            ImageType::Eps => "eps",
            ImageType::Xml => "xml",
            ImageType::Svg => "svg",
            ImageType::Riff => "riff",
            ImageType::Webp => "webp",
            ImageType::Ico => "ico",
        }
    }
}

/// `knowntypes`: the magic numbers `imagetype` recognises, in its order.
const KNOWN_TYPES: [(&[u8], ImageType); 10] = [
    (b"\x89PNG\x0D\x0A\x1A\x0A", ImageType::Png),
    (b"%!PS-Adobe-", ImageType::Ps),
    (b"BM", ImageType::Bmp),
    (b"GIF8", ImageType::Gif),
    (b"\xFF\xD8\xFF", ImageType::Jpeg),
    (b"%PDF-", ImageType::Pdf),
    (b"\xC5\xD0\xD3\xC6", ImageType::Eps),
    (b"<?xml", ImageType::Xml),
    (b"RIFF", ImageType::Riff),
    (b"\x00\x00\x01\x00", ImageType::Ico),
];

/// `HDRLEN`.
const HDRLEN: usize = 20;

/// `imagetype`: the type of the file from its first `HDRLEN` bytes. An XML file is SVG
/// when `<svg` follows the end of its declaration; a RIFF file is WEBP when its form type
/// is `WEBP`.
pub(super) fn imagetype<R: Read + Seek>(f: &mut CFile<R>) -> ImageType {
    let header = f.read_upto(HDRLEN);
    if header.len() != HDRLEN {
        return ImageType::Null;
    }
    let Some(&(_, ty)) = KNOWN_TYPES.iter().find(|(m, _)| header.starts_with(m)) else {
        return ImageType::Null;
    };
    match ty {
        ImageType::Xml => {
            // if we did not see the closing of the XML declaration, scan for it
            if !header.contains(&b'>') {
                loop {
                    match f.getc() {
                        None => return ty,
                        Some(b'>') => break,
                        Some(_) => {}
                    }
                }
            }
            let mut tag = f.read_upto(4);
            if tag.len() != 4 {
                return ty;
            }
            loop {
                if tag == b"<svg" {
                    return ImageType::Svg;
                }
                let Some(c) = f.getc() else {
                    return ty;
                };
                tag.remove(0);
                tag.push(c);
            }
        }
        ImageType::Riff if &header[8..12] == b"WEBP" => ImageType::Webp,
        _ => ty,
    }
}

/// What a size reader found: `us->x`, `us->y`, `us->w`, `us->h` (doubles in C) and
/// `us->dpi`. A reader that cannot read the size leaves the zeros it started with.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dims {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dpi: i32,
}

/// `get_int_lsb_first` (`msb == false`) and `get_int_msb_first`: `sz` bytes, or `None` at
/// the end of the file or for a value above `INT_MAX`.
fn get_int<R: Read + Seek>(f: &mut CFile<R>, sz: usize, msb: bool) -> Option<i32> {
    let mut value: u32 = 0;
    for i in 0..sz {
        let ch = u32::from(f.getc()?);
        if msb {
            value = (value << 8) | ch;
        } else {
            value |= ch << (8 * i);
        }
    }
    i32::try_from(value).ok()
}

/// The size of a file of type `ty` (`gvusershape_open`'s switch); `warn` gets
/// `jpeg_size`'s warning.
pub(super) fn read_size<R: Read + Seek>(
    f: &mut CFile<R>,
    ty: ImageType,
    name: &str,
    warn: &mut dyn FnMut(&str),
) -> Dims {
    let mut d = Dims::default();
    match ty {
        ImageType::Gif => wh(&mut d, f, 6, 2, false),
        ImageType::Png => wh(&mut d, f, 16, 4, true),
        ImageType::Bmp => bmp_size(&mut d, f),
        ImageType::Jpeg => jpeg_size(&mut d, f, name, warn),
        ImageType::Ps => ps_size(&mut d, f),
        ImageType::Webp => webp_size(&mut d, f),
        ImageType::Svg => svg_size(&mut d, f),
        ImageType::Pdf => pdf_size(&mut d, f),
        ImageType::Ico => wh(&mut d, f, 6, 1, true),
        // no eps_size code available
        _ => {}
    }
    d
}

/// `png_size`, `gif_size` and `ico_size`: width then height, `sz` bytes each from `at`.
fn wh<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>, at: u64, sz: usize, msb: bool) {
    f.seek_set(at);
    if let Some(w) = get_int(f, sz, msb)
        && let Some(h) = get_int(f, sz, msb)
    {
        d.w = f64::from(w);
        d.h = f64::from(h);
    }
}

/// `webp_size`: a `VP8X` chunk has 4-byte sizes at 24, any other 2-byte sizes at 26.
fn webp_size<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>) {
    f.seek_set(15);
    if f.getc() == Some(b'X') {
        wh(d, f, 24, 4, false);
    } else {
        wh(d, f, 26, 2, false);
    }
}

/// `bmp_size`: four 2-byte words from 16, read as the high and low halves of the width
/// and the height (`size_x_msw << 16 | size_x_lsw`).
fn bmp_size<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>) {
    f.seek_set(16);
    let mut words = [0i32; 4];
    for w in &mut words {
        match get_int(f, 2, false) {
            Some(v) => *w = v,
            None => return,
        }
    }
    // `int << 16` keeps the low 32 bits
    d.w = f64::from(((words[0] as u32) << 16) as i32 | words[1]);
    d.h = f64::from(((words[2] as u32) << 16) as i32 | words[3]);
}

/// `jpeg_size`: the size in the first `SOF0` or `SOF2` segment, skipping the others by
/// their lengths.
fn jpeg_size<R: Read + Seek>(
    d: &mut Dims,
    f: &mut CFile<R>,
    name: &str,
    warn: &mut dyn FnMut(&str),
) {
    // the markers that follow 0xff without a length field
    const STANDALONE: [i32; 11] = [
        0x01, 0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9,
    ];
    f.rewind();
    loop {
        let Some(marker) = get_int(f, 1, true) else {
            warn(&format!("Parsing of \"{name}\" failed\n"));
            return;
        };
        if marker == 0xff || STANDALONE.contains(&marker) {
            continue;
        }
        if marker == 0xc0 || marker == 0xc2 {
            // skip the length and the sample precision
            if f.seek_cur(3)
                && let Some(h) = get_int(f, 2, true)
                && let Some(w) = get_int(f, 2, true)
            {
                d.h = f64::from(h);
                d.w = f64::from(w);
            }
            return;
        }
        let Some(length) = get_int(f, 2, true) else {
            return;
        };
        f.seek_cur(i64::from(length) - 2);
    }
}

/// `ps_size`: the first `%%BoundingBox:` comment with four numbers, at 72 dpi.
fn ps_size<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>) {
    d.dpi = 72;
    f.rewind();
    while let Some(line) = f.gets() {
        // strstr and sscanf read the line as a C string
        let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        let Some(at) = line.windows(14).position(|w| w == b"%%BoundingBox:") else {
            continue;
        };
        let mut v = [0i32; 4];
        if super::scan_bounding_box(&line[at..], &mut v) == 4 {
            let [lx, ly, ux, uy] = v;
            d.x = f64::from(lx);
            d.y = f64::from(ly);
            // int differences, which wrap
            d.w = f64::from(ux.wrapping_sub(lx));
            d.h = f64::from(uy.wrapping_sub(ly));
            return;
        }
    }
}

/// `fstr`: scans the file for `needle` (whose bytes are distinct) and stops after it.
fn fstr<R: Read + Seek>(f: &mut CFile<R>, needle: &[u8]) -> bool {
    let mut offset = 0;
    loop {
        if offset == needle.len() {
            return true;
        }
        let Some(c) = f.getc() else {
            return false;
        };
        if needle[offset] == c {
            offset += 1;
        } else if needle[0] == c {
            offset = 1;
        } else {
            offset = 0;
        }
    }
}

/// `skipWS`.
fn skip_ws<R: Read + Seek>(f: &mut CFile<R>) {
    while let Some(c) = f.getc() {
        if !crate::cnum::is_c_space(c) {
            f.ungetc();
            break;
        }
    }
}

/// `getNum` then `scanNum`: digits and dots after white space, read with `strtod`.
fn get_num<R: Read + Seek>(f: &mut CFile<R>) -> Option<f64> {
    skip_ws(f);
    let mut tok = String::new();
    while let Some(c) = f.getc() {
        if c.is_ascii_digit() || c == b'.' {
            tok.push(char::from(c));
        } else {
            f.ungetc();
            break;
        }
    }
    crate::strtod::strtod_rest(&tok).map(|(v, _)| v)
}

/// `pdf_size`: the first `/MediaBox [llx lly urx ury]`, numbers without signs.
fn pdf_size<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>) {
    f.rewind();
    if !fstr(f, b"/MediaBox") {
        return;
    }
    // boxof
    skip_ws(f);
    if f.getc() != Some(b'[') {
        return;
    }
    let mut b = [0.0; 4];
    for v in &mut b {
        match get_num(f) {
            Some(n) => *v = n,
            None => return,
        }
    }
    let [llx, lly, urx, ury] = b;
    d.x = llx;
    d.y = lly;
    d.w = urx - llx;
    d.h = ury - lly;
}

/// `svg_units_convert`: a length in points, or 0 for a unit C does not know.
fn svg_units_convert(n: f64, u: &[u8]) -> f64 {
    const POINTS_PER_INCH: f64 = 72.0;
    const POINTS_PER_CM: f64 = POINTS_PER_INCH * 0.393700787;
    const POINTS_PER_MM: f64 = POINTS_PER_INCH * 0.0393700787;
    match u {
        b"in" => (n * POINTS_PER_INCH).round(),
        b"px" => (n * POINTS_PER_INCH / 96.0).round(),
        b"pc" => (n * POINTS_PER_INCH / 6.0).round(),
        // without units, `%2s` gets the closing quote
        b"pt" | b"\"" => n.round(),
        b"cm" => (n * POINTS_PER_CM).round(),
        b"mm" => (n * POINTS_PER_MM).round(),
        _ => 0.0,
    }
}

/// `find_attribute`: the next `([a-z][a-zA-Z]*)="([^"]*)"` in `s`, as (key, value, the
/// rest after the closing quote).
fn find_attribute(s: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let mut i = 0;
    while i < s.len() {
        if !s[i].is_ascii_lowercase() {
            i += 1;
            continue;
        }
        let k0 = i;
        i += 1;
        while i < s.len() && s[i].is_ascii_alphabetic() {
            i += 1;
        }
        let key = &s[k0..i];
        if s.get(i) == Some(&b'=') && s.get(i + 1) == Some(&b'"') {
            i += 2;
            let v0 = i;
            while i < s.len() && s[i] != b'"' {
                i += 1;
            }
            if i < s.len() {
                return Some((key, &s[v0..i], &s[i + 1..]));
            }
        }
    }
    None
}

/// The `width` or `height` value: `sscanf(value, "%lf%2s", &n, u) == 2` converts with
/// the unit, else `sscanf(value, "%lf", &n) == 1` takes points. (A unit with a byte
/// outside ASCII matches none of C's, before and after the lossy conversion.)
fn svg_length(value: &[u8]) -> Option<f64> {
    let s = String::from_utf8_lossy(value);
    let (n, rest) = crate::strtod::scan_lf(&s)?;
    let unit: Vec<u8> = rest
        .bytes()
        .skip_while(|&c| crate::cnum::is_c_space(c))
        .take_while(|&c| !crate::cnum::is_c_space(c))
        .take(2)
        .collect();
    Some(if unit.is_empty() {
        svg_units_convert(n, b"pt")
    } else {
        svg_units_convert(n, &unit)
    })
}

/// `sscanf(value, "%*f %*f %lf %lf", &w, &h) == 2` (each conversion skips white space).
fn svg_viewbox(value: &[u8]) -> Option<(f64, f64)> {
    let s = String::from_utf8_lossy(value);
    let (_, rest) = crate::strtod::scan_f(&s)?;
    let (_, rest) = crate::strtod::scan_f(rest)?;
    let (w, rest) = crate::strtod::scan_lf(rest)?;
    let (h, _) = crate::strtod::scan_lf(rest)?;
    Some((w, h))
}

/// `svg_size`: `width` and `height` with their units, or the size of the `viewBox` when
/// a line ends without them, in points (`dpi` 0).
fn svg_size<R: Read + Seek>(d: &mut Dims, f: &mut CFile<R>) {
    let (mut hard_w, mut hard_h) = (None, None);
    let (mut soft_w, mut soft_h) = (None, None);
    let mut eof = false;
    f.rewind();
    while !eof && (hard_w.is_none() || hard_h.is_none()) {
        let mut line = Vec::new();
        loop {
            match f.getc() {
                None => {
                    eof = true;
                    break;
                }
                Some(b'\n') => break,
                Some(c) => line.push(c),
            }
        }
        // the line is read as a C string
        let mut s = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        while let Some((key, value, rest)) = find_attribute(s) {
            s = rest;
            match key {
                b"width" => {
                    if let Some(w) = svg_length(value) {
                        hard_w = Some(w);
                    }
                    if hard_h.is_some() {
                        break;
                    }
                }
                b"height" => {
                    if let Some(h) = svg_length(value) {
                        hard_h = Some(h);
                    }
                    if hard_w.is_some() {
                        break;
                    }
                }
                b"viewBox" => {
                    if let Some((w, h)) = svg_viewbox(value) {
                        soft_w = Some(w);
                        soft_h = Some(h);
                    }
                }
                _ => {}
            }
        }
        // at the end of a line with a `viewBox`, it gives the sizes still missing
        if let (Some(sw), Some(sh)) = (soft_w, soft_h) {
            hard_h = hard_h.or(Some(sh));
            hard_w = hard_w.or(Some(sw));
            break;
        }
    }
    // C asserts that each size is in `0..=INT_MAX` (and aborts on a negative, too large
    // or NaN size); the port takes 0, the size of a length it cannot read.
    let int = |v: Option<f64>| {
        let v = v.unwrap_or(0.0);
        if (0.0..=f64::from(i32::MAX)).contains(&v) {
            f64::from(v as i32)
        } else {
            0.0
        }
    };
    d.w = int(hard_w);
    d.h = int(hard_h);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn size_of(data: &[u8]) -> (ImageType, Dims, Vec<String>) {
        let mut f = CFile::new(Cursor::new(data.to_vec()));
        let ty = imagetype(&mut f);
        let mut warnings = Vec::new();
        let d = read_size(&mut f, ty, "x", &mut |m| warnings.push(m.to_string()));
        (ty, d, warnings)
    }

    fn dims(w: i32, h: i32) -> Dims {
        Dims {
            w: f64::from(w),
            h: f64::from(h),
            ..Dims::default()
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    #[test]
    fn imagetype_reads_the_magic_numbers() {
        assert_eq!(size_of(&png(40, 20)).0, ImageType::Png);
        // fewer than HDRLEN bytes are no known type
        assert_eq!(size_of(b"\x89PNG\r\n\x1a\n").0, ImageType::Null);
        let svg = b"<?xml version=\"1.0\"?>\n<!-- c --><svg width=\"1in\"/>";
        assert_eq!(size_of(svg).0, ImageType::Svg);
        let svg = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?><svg";
        assert_eq!(size_of(svg).0, ImageType::Svg);
        assert_eq!(
            size_of(b"<?xml version=\"1.0\"?><html/>   ").0,
            ImageType::Xml
        );
        assert_eq!(
            size_of(b"<svg width=\"10\" height=\"10\"/>").0,
            ImageType::Null
        );
        assert_eq!(size_of(b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0").0, ImageType::Webp);
        assert_eq!(size_of(b"RIFF\0\0\0\0AVI LIST\0\0\0\0").0, ImageType::Riff);
        assert_eq!(ImageType::Riff.stringtype(), "riff");
        assert_eq!(ImageType::Null.stringtype(), "(lib)");
    }

    #[test]
    fn raster_headers() {
        assert_eq!(size_of(&png(40, 20)).1, dims(40, 20));
        // a value above INT_MAX is not read, and neither is the other one
        assert_eq!(size_of(&png(0x8000_0000, 3)).1, dims(0, 0));
        assert_eq!(size_of(&png(40, 20)[..22]).1, dims(0, 0));
        let gif = b"GIF89a\x1e\x00\x0f\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(size_of(gif).1, dims(30, 15));
        let ico =
            b"\x00\x00\x01\x00\x01\x00\x10\x20\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert_eq!(size_of(ico).1, dims(16, 32));
        // bmp: the words at 16..24 are the header size's high half, then the width
        let mut bmp = b"BM\0\0\0\0\0\0\0\0\0\0\0\0\x28\x00\x05\x00".to_vec();
        bmp.extend(25i32.to_le_bytes());
        bmp.extend(35i32.to_le_bytes());
        assert_eq!(size_of(&bmp).1, dims(5 << 16 | 25, 35));
        let mut vp8x = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        vp8x.extend([54, 0, 0, 0, 10, 0, 0, 0]);
        assert_eq!(size_of(&vp8x).1, dims(54, 10));
        let mut vp8 = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\0\0\0".to_vec();
        vp8.extend([44, 0, 22, 0]);
        assert_eq!(size_of(&vp8).1, dims(44, 22));
    }

    #[test]
    fn jpeg_segments() {
        let sof = |m: u8| {
            let mut v = b"\xff\xd8\xff\xe0\x00\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0".to_vec();
            v.extend([0xff, 0xff, 0x01, 0xff, m, 0, 17, 8, 0, 48, 0, 64, 3]);
            v
        };
        assert_eq!(size_of(&sof(0xc0)).1, dims(64, 48));
        assert_eq!(size_of(&sof(0xc2)).1, dims(64, 48));
        // the file ends before a marker: C warns
        let (_, d, w) = size_of(&sof(0xc4)[..22]);
        assert_eq!(d, dims(0, 0));
        assert_eq!(w, ["Parsing of \"x\" failed\n"]);
        // a length below 2 seeks back, and the scan still ends
        let (_, d, w) = size_of(
            b"\xff\xd8\xff\xe0\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        );
        assert_eq!(d, dims(0, 0));
        assert!(w.is_empty());
    }

    #[test]
    fn ps_and_pdf_boxes() {
        let (_, d, _) = size_of(b"%!PS-Adobe-3.0\n%%Title: t\r%%BoundingBox: -50 -20 150 30\n");
        assert_eq!(
            d,
            Dims {
                x: -50.0,
                y: -20.0,
                w: 200.0,
                h: 50.0,
                dpi: 72
            }
        );
        let (_, d, _) = size_of(b"%!PS-Adobe-3.0\n%%BoundingBox: (atend)\n");
        assert_eq!(
            d,
            Dims {
                dpi: 72,
                ..Dims::default()
            }
        );
        let (_, d, _) = size_of(b"%PDF-1.7\n<< /MediaBox [ 0.5 0 100.7 50.25 ] >>\n");
        assert_eq!(
            d,
            Dims {
                x: 0.5,
                y: 0.0,
                w: 100.2,
                h: 50.25,
                dpi: 0
            }
        );
        // getNum takes no sign
        assert_eq!(
            size_of(b"%PDF-1.7\n<< /MediaBox [-10 0 100 50] >>\n").1,
            dims(0, 0)
        );
        // fstr restarts only at the needle's first byte
        assert_eq!(
            size_of(b"%PDF-1.7 /Media/MediaBox[0 0 7 8]\n").1,
            dims(7, 8)
        );
        assert_eq!(size_of(b"%PDF-1.7 /MeMediaBox[0 0 7 8]\n").1, dims(0, 0));
    }

    #[test]
    fn svg_lengths() {
        let svg = |attrs: &str| {
            let s = format!("<?xml version=\"1.0\"?>\n<svg {attrs}>\n</svg>\n");
            size_of(s.as_bytes()).1
        };
        assert_eq!(svg(r#"width="2in" height="1in""#), dims(144, 72));
        assert_eq!(svg(r#"width="96px" height="3pc""#), dims(72, 36));
        assert_eq!(svg(r#"width="2.5cm" height="10mm""#), dims(71, 28));
        assert_eq!(svg(r#"width="50" height="25.5""#), dims(50, 26));
        assert_eq!(svg(r#"width="100%" height="3em""#), dims(0, 0));
        assert_eq!(svg(r#"width="2 in" height="x""#), dims(144, 0));
        assert_eq!(svg(r#"viewBox="0 0 120 60""#), dims(120, 60));
        assert_eq!(svg(r#"width="30pt" viewBox="0 0 120 60""#), dims(30, 60));
        assert_eq!(svg(r#"viewBox="0,0,120,60""#), dims(0, 0));
        // `stroke-width` is a `width`
        assert_eq!(svg(r#"stroke-width="3" height="4""#), dims(3, 4));
        // C asserts on a size out of range
        assert_eq!(svg(r#"width="-5" height="1e300""#), dims(0, 0));
        assert_eq!(svg(r#"width="nan" height="7""#), dims(0, 7));
        // the sizes may be on later lines; a viewBox ends the scan at its line's end
        let s = b"<?xml version=\"1.0\"?>\n<svg\nwidth=\"80pt\"\nheight=\"40pt\">\n";
        assert_eq!(size_of(s).1, dims(80, 40));
        let s = b"<?xml version=\"1.0\"?>\n<svg viewBox=\"0 0 10 20\"\nwidth=\"80pt\">\n";
        assert_eq!(size_of(s).1, dims(10, 20));
    }
}
