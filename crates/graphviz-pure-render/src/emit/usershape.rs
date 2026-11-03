//! Drawing user shapes and images: `gvrender_usershape` (`lib/gvc/gvrender.c`) and
//! `gvloadimage` (`lib/gvc/gvloadimage.c`), with the loadimage plugins the reference build
//! installs for each render type ([`LoadImage`]).

use super::job::{Emitter, Engine, LoadImage};
use graphviz_pure_core::usershape;
use graphviz_pure_core::{BoundingBox, Point};

/// `imagescale_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageScale {
    False,
    True,
    Width,
    Height,
    Both,
}

/// `get_imagescale`.
fn get_imagescale(s: &str) -> ImageScale {
    if s.is_empty() {
        ImageScale::False
    } else if s.eq_ignore_ascii_case("width") {
        ImageScale::Width
    } else if s.eq_ignore_ascii_case("height") {
        ImageScale::Height
    } else if s.eq_ignore_ascii_case("both") {
        ImageScale::Both
    } else if graphviz_pure_core::cnum::mapbool(Some(s)) {
        ImageScale::True
    } else {
        ImageScale::False
    }
}

/// `get_imagepos`: the vertical (`t`, `m`, `b`) and horizontal (`l`, `c`, `r`) place,
/// `mc` for anything else.
fn get_imagepos(s: &str) -> (u8, u8) {
    const POS: [&str; 9] = ["tl", "tc", "tr", "ml", "mc", "mr", "bl", "bc", "br"];
    let p = POS
        .iter()
        .find(|p| s.eq_ignore_ascii_case(p))
        .unwrap_or(&"mc")
        .as_bytes();
    (p[0], p[1])
}

impl<E: Engine> Emitter<'_, E> {
    /// `gvrender_usershape`: draws the file `name` in the bounding box of the polygon `a`,
    /// scaled by `imagescale` and placed at `imagepos`.
    pub(crate) fn usershape(
        &mut self,
        name: &str,
        a: &[Point],
        filled: bool,
        imagescale: &str,
        imagepos: &str,
    ) {
        let Some(us) = usershape::find(name) else {
            if usershape::find_user_shape(name) {
                self.eng.library_shape(&self.job, name, a, filled);
            }
            // a name that never loaded: C draws it here only as a shape library member, with
            // PostScript's `library_shape` (a member `gvusershape_open` took leaves below,
            // at the 0x0 check)
            return;
        };
        let isz = usershape::size_dpi(&us, (self.job.dpi.x, self.job.dpi.y));
        if isz.0 <= 0 && isz.1 <= 0 {
            return;
        }
        let Some((&a0, rest)) = a.split_first() else {
            return;
        };
        // expandbp
        let mut b = BoundingBox { ll: a0, ur: a0 };
        for p in rest {
            b.ll.x = b.ll.x.min(p.x);
            b.ll.y = b.ll.y.min(p.y);
            b.ur.x = b.ur.x.max(p.x);
            b.ur.y = b.ur.y.max(p.y);
        }
        let pw = b.ur.x - b.ll.x;
        let ph = b.ur.y - b.ll.y;
        let mut ih = f64::from(isz.1);
        let mut iw = f64::from(isz.0);
        let scalex = pw / iw;
        let scaley = ph / ih;
        match get_imagescale(imagescale) {
            ImageScale::True => {
                // keep the aspect ratio by using the smaller scale
                let s = if scalex < scaley { scalex } else { scaley };
                iw *= s;
                ih *= s;
            }
            ImageScale::Width => iw *= scalex,
            ImageScale::Height => ih *= scaley,
            ImageScale::Both => {
                iw *= scalex;
                ih *= scaley;
            }
            ImageScale::False => {}
        }
        // an image smaller than the box in a dimension is placed at `imagepos`
        let (v, h) = get_imagepos(imagepos);
        if iw < pw {
            match h {
                b'l' => b.ur.x = b.ll.x + iw,
                b'r' => {
                    b.ll.x += pw - iw;
                    b.ur.x = b.ll.x + iw;
                }
                _ => {
                    b.ll.x += (pw - iw) / 2.0;
                    b.ur.x -= (pw - iw) / 2.0;
                }
            }
        }
        if ih < ph {
            match v {
                b't' => b.ll.y = b.ur.y - ih,
                b'b' => {
                    b.ll.y += ih;
                    b.ur.y = b.ll.y - ih;
                }
                _ => {
                    b.ll.y += (ph - ih) / 2.0;
                    b.ur.y -= (ph - ih) / 2.0;
                }
            }
        }
        // (every renderer the port has does its own transform: no gvrender_ptf)
        if b.ll.x > b.ur.x {
            std::mem::swap(&mut b.ll.x, &mut b.ur.x);
        }
        if b.ll.y > b.ur.y {
            std::mem::swap(&mut b.ll.y, &mut b.ur.y);
        }
        let _ = filled;
        self.gvloadimage(&us, b);
    }

    /// `gvloadimage`: draws `us` with the loadimage plugin for its type and the job's render
    /// type. Without one, C warns, and draws with the plugin the job selected last, if any.
    fn gvloadimage(&mut self, us: &usershape::UserShape, b: BoundingBox) {
        let render_type = self.eng.render_type().to_string();
        match LoadImage::select(us.stringtype(), &render_type) {
            Some(li) => {
                // `gvplugin_load(gvc, API_loadimage, "TYPE:RENDER")`, after its renderer.
                if graphviz_pure_core::diag::verbose() >= 1 {
                    // The renderer and the loader come from the same package here.
                    let package = li.package();
                    graphviz_pure_core::diag::fprintf_stderr(&format!(
                        "Using render: {render_type}:{package}\nUsing loadimage: {}:{render_type}:{package}\n",
                        us.stringtype()
                    ));
                }
                self.loadimage = Some(li);
            }
            None => graphviz_pure_core::diag::agwarningf(&format!(
                "No loadimage plugin for \"{}:{render_type}\"\n",
                us.stringtype()
            )),
        }
        if let Some(li) = self.loadimage {
            self.eng.loadimage(&self.job, li, us, b);
        }
    }

    /// Text C writes with `fprintf(job->output_file, ...)`, past the renderer; dropped
    /// when the job has no output file.
    pub(crate) fn write_output_file(&mut self, s: &str) {
        if let Some(out) = self.output_file.as_mut()
            && !self.eng.write_output_file(s)
        {
            out.push_str(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagescale_and_imagepos_parse_like_c() {
        assert_eq!(get_imagescale(""), ImageScale::False);
        assert_eq!(get_imagescale("WIDTH"), ImageScale::Width);
        assert_eq!(get_imagescale("Both"), ImageScale::Both);
        assert_eq!(get_imagescale("yes"), ImageScale::True);
        assert_eq!(get_imagescale("2"), ImageScale::True);
        assert_eq!(get_imagescale("widths"), ImageScale::False);
        assert_eq!(get_imagepos("BR"), (b'b', b'r'));
        assert_eq!(get_imagepos("x"), (b'm', b'c'));
        assert_eq!(get_imagepos(""), (b'm', b'c'));
    }

    #[test]
    fn loadimage_plugins_of_the_reference_build() {
        assert_eq!(LoadImage::select("png", "svg"), Some(LoadImage::Svg));
        assert_eq!(LoadImage::select("svg", "svg"), Some(LoadImage::Svg));
        assert_eq!(LoadImage::select("ps", "svg"), None);
        assert_eq!(LoadImage::select("ps", "xdot"), Some(LoadImage::Xdot));
        assert_eq!(LoadImage::select("bmp", "xdot"), None);
        assert_eq!(LoadImage::select("eps", "dot"), Some(LoadImage::Null));
        assert_eq!(LoadImage::select("png", "json"), None);
        assert_eq!(LoadImage::select("(lib)", "svg"), None);
    }
}
