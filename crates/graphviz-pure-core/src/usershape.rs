//! User-shape files: how C finds and opens the files named by `image`, `shapefile` and an
//! HTML `<IMG SRC>` (`safefile` in `lib/common/utils.c`, `gvusershape_file_access`,
//! `gvusershape_open` and `gvusershape_size` in `lib/gvc/gvusershape.c`, `user_init` and
//! `epsf_init` in `lib/common/psusershape.c`), with the warnings C prints when it cannot.
//!
//! As C, the port reads no image data: it classifies the file's header as `imagetype`
//! does and reads the size from it (the readers in [`size`]), and keeps what it read by
//! name, as `ImageDict` does, for the renderers (`gvrender_usershape`). For `shape=epsf`
//! it reads the `%%BoundingBox` comment, which gives the node its size.
//!
//! Paths follow C's Unix rules on every platform: `imagepath` and `--filepath` list
//! directories separated by `:`, joined to the file name with `/`, and a name starting with
//! `/` is absolute. The WASM build has no filesystem: every file is missing there, so it
//! reports `No such file or directory` as C does for a file that does not exist.

mod size;

pub use size::ImageType;

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::sync::Mutex;

/// `agwarningf`, except in the port's repeat sizings ([`crate::diag::quietly`]).
fn agwarningf(msg: &str) {
    if crate::diag::reporting() {
        crate::diag::agwarningf(msg);
    }
}

/// `Gvfilepath`: the directories of `dot --filepath=` (process-wide in C).
static FILEPATH: Mutex<Option<String>> = Mutex::new(None);
/// `HTTPServerEnVar`: `$SERVER_NAME`, read by `dotneato_args_initialize`.
static SERVER_NAME: Mutex<Option<String>> = Mutex::new(None);

/// Sets `Gvfilepath` (`dot --filepath=DIRS`).
pub fn set_filepath(dirs: Option<&str>) {
    *FILEPATH.lock().unwrap_or_else(|e| e.into_inner()) = dirs.map(str::to_string);
}

/// Sets `HTTPServerEnVar` (the CLI's `$SERVER_NAME`): file loading is then disabled.
pub fn set_server_name(name: Option<&str>) {
    *SERVER_NAME.lock().unwrap_or_else(|e| e.into_inner()) = name.map(str::to_string);
}

fn filepath() -> Option<String> {
    FILEPATH.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn server_name() -> Option<String> {
    SERVER_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The warnings a read gave in one of the port's quiet sizings. C reads each file once, in
/// the sizing that reports, so the port's first sizing that reports prints them.
#[derive(Debug, Clone, Default)]
struct Deferred(Vec<String>);

impl Deferred {
    /// The warnings of a read: printed now in a sizing that reports, else kept.
    fn of(warnings: Vec<String>) -> Self {
        let mut d = Deferred(warnings);
        d.deliver();
        d
    }

    /// Prints what is kept, in a sizing that reports.
    fn deliver(&mut self) {
        if crate::diag::reporting() {
            for m in std::mem::take(&mut self.0) {
                crate::diag::agwarningf(&m);
            }
        }
    }
}

/// What C keeps for the whole process, which the port keeps for one run: a top-level call
/// of the library, or the CLI ([`reset_run`]).
#[derive(Default)]
struct RunState {
    /// The names in `UserShape` (`user_shape` in `shapes.c`), which `gvusershape_open` takes
    /// for a file it cannot classify.
    user_shapes: HashSet<String>,
    /// `safefile`'s `onetime` flag.
    server_warned: bool,
    /// `findPath`'s static buffer: the last path it tried.
    find_path_buf: String,
    /// `ImageDict`: the files `gvusershape_open` loaded, by name.
    image_dict: HashMap<String, (UserShape, Deferred)>,
    /// `gvusershape_size`'s `static char *oldpath`, as a [`UserShapes::imagepath_id`].
    oldpath: u64,
    /// The last id given to a graph's `imagepath` or layout.
    last_id: u64,
    /// `EPSF_contents`, cdt's `Dtoset` splay tree. Its keys are pointers: a key that is a
    /// graph's string matches no lookup once that graph is closed, and a key that is
    /// `findPath`'s buffer has the text of the last path built there, so a lookup through the
    /// buffer matches it whatever file it names, if the search reaches it (#380). Not
    /// modelled, as C reads freed memory there: a buffer that moved when it grew (past its
    /// 30 characters of inline storage, then on reallocation), and a closed graph's key
    /// whose memory was reused ([`EpsfTree::cmp`]).
    epsf_contents: EpsfTree,
    /// `N_EPSF_files`.
    n_epsf_files: i32,
}

impl RunState {
    /// `dtmatch` (`ins` is `None`) or `dtinsert` of `EPSF_contents`, with `findPath`'s buffer
    /// as it is now.
    fn epsf_dttree(
        &mut self,
        key: &str,
        ins: Option<(EpsfKey, EpsfShape)>,
        layout: u64,
    ) -> Option<EpsfShape> {
        self.epsf_contents
            .dttree(key, ins, &self.find_path_buf, layout)
    }
}

/// The usershape context of one graph's layout (`graph_init`'s globals and what the sizings
/// found), which the layout leaves on the graph (`GraphLayout::usershapes`) and the
/// renderers draw from ([`rendering`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserShapes {
    /// `Gvimagepath`: the graph's `imagepath`, else `Gvfilepath`.
    pub imagepath: Option<String>,
    /// `Gvimagepath` as `gvusershape_size` compares it: a new value for each graph with its
    /// own `imagepath`, 0 for `Gvfilepath`.
    pub imagepath_id: u64,
    /// The graph, for the keys of `EPSF_contents`.
    pub layout_id: u64,
    /// Some node declares `shapefile`.
    pub shapefile_declared: bool,
    /// `GD_drawing(g)->dpi`.
    pub dpi: f64,
    /// `ImageDict` as the layout left it.
    pub images: BTreeMap<String, UserShape>,
    /// What `epsf_init` gave each node.
    pub epsf: Vec<EpsfNode>,
}

/// `epsf_init` of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct EpsfNode {
    pub node: String,
    pub shapefile: Option<String>,
    pub shape: Option<EpsfShape>,
}

/// The live [`UserShapes`] of the graph being laid out or drawn.
#[derive(Default)]
struct GraphState {
    imagepath: Option<String>,
    imagepath_id: u64,
    layout_id: u64,
    shapefile_declared: bool,
    dpi: f64,
    /// C runs `epsf_init` once per node; the port's repeat sizings (quiet passes, the
    /// renderers' model) get the same answer.
    epsf_nodes: HashMap<(String, Option<String>), (Option<EpsfShape>, Deferred)>,
}

thread_local! {
    static RUN: RefCell<RunState> = RefCell::new(RunState::default());
    static GRAPH: RefCell<GraphState> = RefCell::new(GraphState::default());
}

fn run<R>(f: impl FnOnce(&mut RunState) -> R) -> R {
    RUN.with(|r| f(&mut r.borrow_mut()))
}

fn graph<R>(f: impl FnOnce(&mut GraphState) -> R) -> R {
    GRAPH.with(|g| f(&mut g.borrow_mut()))
}

/// Starts a run, as a new C process starts: no file is loaded, no epsf file numbered and no
/// user shape known.
pub fn reset_run() {
    run(|r| *r = RunState::default());
    graph(|g| *g = GraphState::default());
}

/// `graph_init`: `Gvimagepath` is the root graph's `imagepath`, or `Gvfilepath` when the
/// graph has none. `shapefile_declared`: some node declares `shapefile`. `dpi` is
/// `GD_drawing(g)->dpi` ([`crate::cnum::drawing_dpi`]).
pub fn begin_layout(imagepath: Option<&str>, shapefile_declared: bool, dpi: f64) {
    // a graph's own `imagepath` is a new string, which `gvusershape_size` sees as a new
    // path (and empties `ImageDict` for); `Gvfilepath` stays the same. (C compares the
    // pointers: an allocator that gave the next graph's string the same address would keep
    // the files, which the reference build was not seen to do.)
    let (imagepath_id, layout_id) = run(|r| {
        let path = if imagepath.is_some() {
            r.last_id += 1;
            r.last_id
        } else {
            0
        };
        r.last_id += 1;
        (path, r.last_id)
    });
    let imagepath = imagepath.map(str::to_string).or_else(filepath);
    graph(|g| {
        *g = GraphState {
            imagepath,
            imagepath_id,
            layout_id,
            shapefile_declared,
            dpi,
            epsf_nodes: HashMap::new(),
        }
    });
}

/// The usershape context the layout leaves on the graph.
pub fn layout_result() -> UserShapes {
    let images = run(|r| {
        r.image_dict
            .iter()
            .map(|(k, (us, _))| (k.clone(), us.clone()))
            .collect()
    });
    graph(|g| UserShapes {
        imagepath: g.imagepath.clone(),
        imagepath_id: g.imagepath_id,
        layout_id: g.layout_id,
        shapefile_declared: g.shapefile_declared,
        dpi: g.dpi,
        images,
        epsf: g
            .epsf_nodes
            .iter()
            .map(|((node, shapefile), (shape, _))| EpsfNode {
                node: node.clone(),
                shapefile: shapefile.clone(),
                shape: *shape,
            })
            .collect(),
    })
}

/// Restores the state [`rendering`] replaced.
struct Restore {
    graph: Option<GraphState>,
    images: HashMap<String, (UserShape, Deferred)>,
    oldpath: u64,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(saved) = self.graph.take() {
            graph(|g| *g = saved);
        }
        let images = std::mem::take(&mut self.images);
        let oldpath = self.oldpath;
        run(|r| {
            r.image_dict = images;
            r.oldpath = oldpath;
        });
    }
}

/// Runs `f`, which draws a laid-out graph, in the usershape context its layout left (`s`):
/// the renderers find the files and the epsf shapes the layout found, whatever was laid out
/// since or on whichever thread.
pub fn rendering<R>(s: &UserShapes, f: impl FnOnce() -> R) -> R {
    let live = GraphState {
        imagepath: s.imagepath.clone(),
        imagepath_id: s.imagepath_id,
        layout_id: s.layout_id,
        shapefile_declared: s.shapefile_declared,
        dpi: s.dpi,
        epsf_nodes: s
            .epsf
            .iter()
            .map(|e| {
                (
                    (e.node.clone(), e.shapefile.clone()),
                    (e.shape, Deferred::default()),
                )
            })
            .collect(),
    };
    let images: HashMap<_, _> = s
        .images
        .iter()
        .map(|(k, us)| (k.clone(), (us.clone(), Deferred::default())))
        .collect();
    let saved = graph(|g| std::mem::replace(g, live));
    let (images, oldpath) = run(|r| {
        (
            std::mem::replace(&mut r.image_dict, images),
            std::mem::replace(&mut r.oldpath, s.imagepath_id),
        )
    });
    let _restore = Restore {
        graph: Some(saved),
        images,
        oldpath,
    };
    f()
}

/// `agget(n, "shapefile")` for a node whose own value is `value`: the declared default `""`
/// when it has none, NULL when no node declares the attribute.
pub fn agget_shapefile(value: Option<&str>) -> Option<&str> {
    value.or_else(|| graph(|g| g.shapefile_declared).then_some(""))
}

/// `user_shape` records `name` in `UserShape`.
pub fn register_user_shape(name: &str) {
    run(|r| {
        if !r.user_shapes.contains(name) {
            r.user_shapes.insert(name.to_string());
        }
    });
}

/// `find_user_shape`: `name` is in `UserShape`.
pub fn find_user_shape(name: &str) -> bool {
    run(|r| r.user_shapes.contains(name))
}

/// `mkDirlist`: the directories of a `:`-separated list, as `tok` splits it (a leading or a
/// trailing separator gives an empty directory).
fn mk_dirlist(list: &str) -> Vec<&str> {
    let b = list.as_bytes();
    let cspn = |i: usize| b[i..].iter().take_while(|&&c| c != b':').count();
    let mut dirs = vec![&list[..cspn(0)]];
    let mut end = dirs[0].len();
    while end < b.len() {
        let start = end + b[end..].iter().take_while(|&&c| c == b':').count();
        end = start + cspn(start);
        dirs.push(&list[start..end]);
    }
    dirs
}

/// `findPath`: the first `dir/name` that can be read. C builds each name in one static
/// buffer (`safefilename`), which keeps the last one tried.
fn find_path(dirs: &[&str], name: &str) -> Option<String> {
    for d in dirs {
        let f = format!("{d}/{name}");
        run(|r| r.find_path_buf.clone_from(&f));
        if open(&f).is_ok() {
            return Some(f);
        }
    }
    None
}

/// `safefile`: the path C opens for `filename`, or `None` when it may not be read.
pub fn safefile(filename: &str) -> Option<String> {
    safefile_from(filename).map(|(path, _)| path)
}

/// [`safefile`], and whether C returns `findPath`'s static buffer (`true`) or the name it
/// was given, a string of the graph.
fn safefile_from(filename: &str) -> Option<(String, bool)> {
    if filename.is_empty() {
        return None;
    }
    if let Some(server) = server_name() {
        if crate::diag::reporting() && !run(|r| std::mem::replace(&mut r.server_warned, true)) {
            agwarningf(&format!(
                "file loading is disabled because the environment contains SERVER_NAME=\"{server}\"\n"
            ));
        }
        return None;
    }
    if let Some(list) = filepath() {
        // only the last component of the name, looked up in `--filepath`
        let base = filename.rsplit(['/', '\\', ':']).next().unwrap_or(filename);
        return find_path(&mk_dirlist(&list), base).map(|p| (p, true));
    }
    let list = graph(|g| g.imagepath.clone()).unwrap_or_default();
    if filename.starts_with('/') || list.is_empty() {
        Some((filename.to_string(), false))
    } else {
        find_path(&mk_dirlist(&list), filename).map(|p| (p, true))
    }
}

/// `bind_shape`: a non-empty `shapefile` that `safefile` accepts makes the shape `custom`
/// (except for `epsf`).
pub fn binds_custom(shape: &str, shapefile: Option<&str>) -> bool {
    shape != "epsf" && shapefile.is_some_and(|f| safefile(f).is_some())
}

/// The bytes C passes to `fopen` for `path`.
fn os_path(path: &str) -> std::path::PathBuf {
    let bytes = crate::charset::encode(path);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&bytes))
    }
    #[cfg(not(unix))]
    {
        std::path::PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// An opened file, or a directory, which `fopen` opens on Unix and which then reads nothing.
enum Opened {
    File(File),
    Dir,
}

impl Opened {
    /// The stream the size readers read.
    fn into_cfile(self) -> size::CFile<File> {
        match self {
            Opened::File(f) => size::CFile::new(f),
            Opened::Dir => size::CFile::unreadable(),
        }
    }

    fn all(&mut self) -> Vec<u8> {
        let mut buf = Vec::new();
        if let Opened::File(f) = self {
            let _ = f.read_to_end(&mut buf);
        }
        buf
    }
}

/// `fopen(path, "r")`.
fn open(path: &str) -> io::Result<Opened> {
    let p = os_path(path);
    if std::fs::metadata(&p).is_ok_and(|m| m.is_dir()) {
        return Ok(Opened::Dir);
    }
    File::open(p).map(Opened::File)
}

/// `strerror(errno)` for a failed `fopen`, with POSIX's texts. Without a filesystem (WASM)
/// a file is missing.
fn strerror(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::Unsupported => {
            "No such file or directory".to_string()
        }
        io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
        io::ErrorKind::NotADirectory => "Not a directory".to_string(),
        io::ErrorKind::IsADirectory => "Is a directory".to_string(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error ") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }
    }
}

/// `usershape_t` as `gvusershape_open` leaves it: a file it loaded, with the type and
/// the size its header gives (`dpi` 0: the size is in pixels at the graph's resolution).
/// A name C finds as a shape library member (`find_user_shape`) has type `(lib)` and
/// size 0x0.
#[derive(Debug, Clone, PartialEq)]
pub struct UserShape {
    pub name: String,
    pub ty: ImageType,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dpi: i32,
}

impl UserShape {
    /// `us->stringtype`.
    pub fn stringtype(&self) -> &'static str {
        self.ty.stringtype()
    }
}

/// `gvusershape_open`: the file `name` loaded, from `ImageDict` or opened and read, or
/// `None` with C's warning when it cannot be opened or is neither an image nor a shape
/// library member.
fn gvusershape_open(name: &str) -> Option<UserShape> {
    let found = run(|r| {
        let (us, pending) = r.image_dict.get_mut(name)?;
        Some((us.clone(), std::mem::take(pending)))
    });
    if let Some((us, mut pending)) = found {
        pending.deliver();
        // what a quiet sizing still could not print stays for the next one
        run(|r| {
            if let Some(e) = r.image_dict.get_mut(name) {
                e.1 = pending;
            }
        });
        return Some(us);
    }
    // gvusershape_file_access
    let Some(path) = safefile(name) else {
        agwarningf(&format!("Filename \"{name}\" is unsafe\n"));
        return None;
    };
    let mut f = match open(&path) {
        Ok(f) => f.into_cfile(),
        Err(e) => {
            agwarningf(&format!("{} while opening {path}\n", strerror(&e)));
            return None;
        }
    };
    let ty = size::imagetype(&mut f);
    if ty == ImageType::Null && !find_user_shape(name) {
        agwarningf(&format!(
            "\"{name}\" was not found as a file or as a shape library member\n"
        ));
        return None;
    }
    let mut warnings = Vec::new();
    let d = size::read_size(&mut f, ty, name, &mut |m| warnings.push(m.to_string()));
    let us = UserShape {
        name: name.to_string(),
        ty,
        x: d.x,
        y: d.y,
        w: d.w,
        h: d.h,
        dpi: d.dpi,
    };
    let pending = Deferred::of(warnings);
    run(|r| r.image_dict.insert(name.to_string(), (us.clone(), pending)));
    Some(us)
}

/// `gvusershape_find`: the file `name` if `gvusershape_open` loaded it.
pub fn find(name: &str) -> Option<UserShape> {
    run(|r| r.image_dict.get(name).map(|e| e.0.clone()))
}

/// `gvusershape_size_dpi`: the size of `us` in points, at its own resolution or at `dpi`.
pub fn size_dpi(us: &UserShape, dpi: (f64, f64)) -> (i32, i32) {
    let dpi = if us.dpi != 0 {
        (f64::from(us.dpi), f64::from(us.dpi))
    } else {
        dpi
    };
    // the conversion to int saturates on the reference platform
    ((us.w * 72.0 / dpi.0) as i32, (us.h * 72.0 / dpi.1) as i32)
}

/// `gvusershape_size`: the size in points of the file `name` at the graph's `dpi` (96
/// below 1), loading it if needed; `(-1, -1)` without a name or when it does not load.
pub fn gvusershape_size(name: Option<&str>) -> (i32, i32) {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return (-1, -1);
    };
    let (path_id, dpi) = graph(|g| (g.imagepath_id, g.dpi));
    if server_name().is_none() {
        run(|r| {
            if r.oldpath != path_id {
                r.oldpath = path_id;
                r.image_dict.clear();
            }
        });
    }
    let dpi = if dpi >= 1.0 { dpi } else { 96.0 };
    gvusershape_open(name).map_or((-1, -1), |us| size_dpi(&us, (dpi, dpi)))
}

/// `user_init`'s scan of the file, line by line as `fgets` reads them (`BUFSIZ` is 1024 on
/// the reference platform): every `sscanf` stores the numbers it matches, so the last
/// `%%BoundingBox` line wins, and the scan stops early only once it has also seen a line
/// that is not a comment and contains `read` (`must_inline`).
fn bounding_box(data: &[u8]) -> Option<[i32; 4]> {
    const BUFSIZ: usize = 1024;
    let mut bb = [0i32; 4];
    let (mut saw_bb, mut must_inline) = (false, false);
    let mut rest = data;
    while !rest.is_empty() {
        let len = rest
            .iter()
            .take(BUFSIZ - 1)
            .position(|&c| c == b'\n')
            .map_or(rest.len().min(BUFSIZ - 1), |i| i + 1);
        let (line, tail) = rest.split_at(len);
        rest = tail;
        // sscanf and strstr read the line as a C string
        let line = &line[..line.iter().position(|&c| c == 0).unwrap_or(line.len())];
        if scan_bounding_box(line, &mut bb) == 4 {
            saw_bb = true;
        }
        if line.first() != Some(&b'%') && line.windows(4).any(|w| w == b"read") {
            must_inline = true;
        }
        if saw_bb && must_inline {
            break;
        }
    }
    saw_bb.then_some(bb)
}

/// `isspace` in the C locale.
fn c_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// `sscanf(line, "%%%%BoundingBox: %d %d %d %d", ...)`: stores each number it matches in
/// `v` and returns how many. `%%` skips white space before its `%`, as conversions do.
fn scan_bounding_box(line: &[u8], v: &mut [i32; 4]) -> usize {
    let skip = |s: &[u8]| s.iter().take_while(|&&c| c_isspace(c)).count();
    let mut s = line;
    for _ in 0..2 {
        s = &s[skip(s)..];
        match s.strip_prefix(b"%") {
            Some(t) => s = t,
            None => return 0,
        }
    }
    let Some(t) = s.strip_prefix(b"BoundingBox:") else {
        return 0;
    };
    s = t;
    for (k, x) in v.iter_mut().enumerate() {
        s = &s[skip(s)..];
        let sign = usize::from(matches!(s.first(), Some(b'+' | b'-')));
        let digits = s[sign..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return k;
        }
        // strtol: clamped to the long range, then stored into an int
        let neg = s.first() == Some(&b'-');
        let mut n: i64 = 0;
        for &d in &s[sign..sign + digits] {
            let d = i64::from(d - b'0');
            n = n.saturating_mul(10);
            n = if neg {
                n.saturating_sub(d)
            } else {
                n.saturating_add(d)
            };
        }
        *x = n as i32;
        s = &s[sign + digits..];
    }
    4
}

/// An epsf file `user_init` read: its `%%BoundingBox` and its `macro_id`, the number of
/// the PostScript procedure `user_shape_N` that `epsf_gencode` calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpsfShape {
    pub bb: [i32; 4],
    pub macro_id: i32,
}

/// The key `user_init` stores a file under: the pointer `safefile` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EpsfKey {
    /// `findPath`'s static buffer: its text is that of the last path it built.
    Static,
    /// A string of the graph (with its [`UserShapes::layout_id`]), freed with it.
    Graph(u64, String),
}

/// A node of `EPSF_contents`.
#[derive(Debug)]
struct EpsfEntry {
    key: EpsfKey,
    us: EpsfShape,
    left: Option<usize>,
    right: Option<usize>,
}

/// `EPSF_contents`: cdt's ordered set (`lib/cdt/dttree.c`), a top-down splay tree whose keys
/// compare by `strcmp` at the time of each comparison. A key that is `findPath`'s buffer
/// compares by the buffer's current text, so it sits where the text it held at insertion
/// placed it, and a later search, steered by other keys, can miss it (#380).
#[derive(Debug, Default)]
struct EpsfTree {
    nodes: Vec<EpsfEntry>,
    root: Option<usize>,
}

impl EpsfTree {
    /// `strcmp(key, k)` for the search key `key` and the entry `k`. A key of a closed graph
    /// points at freed memory in C (undefined behaviour): what C reads there depends on the
    /// platform's allocator. The port models the reference build's (macOS arm64), which
    /// clears freed small blocks, so the key reads as `""`: it matches nothing and every
    /// search passes it on the right. The choice is the port's, the same on every platform;
    /// where a later allocation reused the memory, C reads other bytes
    /// (docs/porting/crashes.md, "Freed keys in EPSF_contents").
    fn cmp(&self, key: &str, k: usize, buf: &str, layout: u64) -> Ordering {
        match &self.nodes[k].key {
            EpsfKey::Static => key.cmp(buf),
            EpsfKey::Graph(g, p) if *g == layout => key.cmp(p.as_str()),
            EpsfKey::Graph(..) => key.cmp(""),
        }
    }

    /// `dttree(dt, obj, DT_MATCH)` (`ins` is `None`) or `DT_INSERT` (`ins` given, whose key's
    /// text is `key`): the matching entry, splayed to the root, or for an insert that matches
    /// none, `ins` as the new root. `None` when a match finds nothing. A `Dtoset` insert that
    /// matches returns the entry it found and drops `ins`.
    fn dttree(
        &mut self,
        key: &str,
        ins: Option<(EpsfKey, EpsfShape)>,
        buf: &str,
        layout: u64,
    ) -> Option<EpsfShape> {
        // C's `link`: the LEFT tree (`link.right`) and the RIGHT tree (`link.left`), and `l`
        // and `r`, the entries last linked into each (`None` while it is `&link`).
        let mut left = Link::default();
        let mut right = Link::default();
        let mut root = self.root;
        while let Some(rt) = root {
            let cmp = self.cmp(key, rt, buf, layout);
            if cmp == Ordering::Equal {
                break;
            }
            if cmp == Ordering::Less {
                let Some(t) = self.nodes[rt].left else {
                    right.rlink(&mut self.nodes, rt);
                    root = None;
                    break;
                };
                match self.cmp(key, t, buf, layout) {
                    Ordering::Less => {
                        // rrotate(root, t); rlink(r, t)
                        self.nodes[rt].left = self.nodes[t].right;
                        self.nodes[t].right = Some(rt);
                        right.rlink(&mut self.nodes, t);
                        root = self.nodes[t].left;
                    }
                    Ordering::Equal => {
                        right.rlink(&mut self.nodes, rt);
                        root = Some(t);
                        break;
                    }
                    Ordering::Greater => {
                        left.llink(&mut self.nodes, t);
                        right.rlink(&mut self.nodes, rt);
                        root = self.nodes[t].right;
                    }
                }
            } else {
                let Some(t) = self.nodes[rt].right else {
                    left.llink(&mut self.nodes, rt);
                    root = None;
                    break;
                };
                match self.cmp(key, t, buf, layout) {
                    Ordering::Greater => {
                        // lrotate(root, t); llink(l, t)
                        self.nodes[rt].right = self.nodes[t].left;
                        self.nodes[t].left = Some(rt);
                        left.llink(&mut self.nodes, t);
                        root = self.nodes[t].right;
                    }
                    Ordering::Equal => {
                        left.llink(&mut self.nodes, rt);
                        root = Some(t);
                        break;
                    }
                    Ordering::Less => {
                        right.rlink(&mut self.nodes, t);
                        left.llink(&mut self.nodes, rt);
                        root = self.nodes[t].left;
                    }
                }
            }
        }
        let root = match (root, ins) {
            (Some(rt), _) => {
                // found it, now isolate it
                let (rl, rr) = (self.nodes[rt].left, self.nodes[rt].right);
                left.set_end(&mut self.nodes, rl, false);
                right.set_end(&mut self.nodes, rr, true);
                rt
            }
            (None, Some((key, us))) => {
                // not found: finish up the LEFT and RIGHT trees, then dt_insert
                left.set_end(&mut self.nodes, None, false);
                right.set_end(&mut self.nodes, None, true);
                self.nodes.push(EpsfEntry {
                    key,
                    us,
                    left: None,
                    right: None,
                });
                self.nodes.len() - 1
            }
            (None, None) => {
                // no_root: the LEFT tree hangs under the RIGHT tree's smallest entry, which
                // is `r` (its left was just cleared), or is the RIGHT tree when that is empty
                left.set_end(&mut self.nodes, None, false);
                let lt = left.head;
                right.set_end(&mut self.nodes, lt, true);
                self.root = right.head;
                return None;
            }
        };
        // has_root
        self.nodes[root].left = left.head;
        self.nodes[root].right = right.head;
        self.root = Some(root);
        Some(self.nodes[root].us)
    }
}

/// One side of `dttree`'s `link`: the tree's head, and the entry last linked into it.
#[derive(Default)]
struct Link {
    head: Option<usize>,
    last: Option<usize>,
}

impl Link {
    /// `llink(l, x)`: `x` hangs at the right of the LEFT tree.
    fn llink(&mut self, nodes: &mut [EpsfEntry], x: usize) {
        self.set_end(nodes, Some(x), false);
        self.last = Some(x);
    }

    /// `rlink(r, x)`: `x` hangs at the left of the RIGHT tree.
    fn rlink(&mut self, nodes: &mut [EpsfEntry], x: usize) {
        self.set_end(nodes, Some(x), true);
        self.last = Some(x);
    }

    /// `l->right = x` (`left` false) or `r->left = x` (`left` true), where `l`/`r` may be
    /// `&link` itself, whose field is the tree's head.
    fn set_end(&mut self, nodes: &mut [EpsfEntry], x: Option<usize>, left: bool) {
        match (self.last, left) {
            (Some(i), false) => nodes[i].right = x,
            (Some(i), true) => nodes[i].left = x,
            (None, _) => self.head = x,
        }
    }
}

/// `dtmatch(EPSF_contents, str)`.
fn epsf_lookup(path: &str) -> Option<EpsfShape> {
    let layout = graph(|g| g.layout_id);
    run(|r| r.epsf_dttree(path, None, layout))
}

/// `epsf_init`: the `%%BoundingBox` (`lx ly ux uy`) and macro number of a `shape=epsf`
/// node's `shapefile`, or `None`, with C's warning, when there is none; the node then keeps
/// no shape information and is not drawn.
pub fn epsf_init(shapefile: Option<&str>, node: &str) -> Option<EpsfShape> {
    let key = (node.to_string(), shapefile.map(str::to_string));
    let found = graph(|g| {
        g.epsf_nodes
            .get_mut(&key)
            .map(|(us, pending)| (*us, std::mem::take(pending)))
    });
    let (us, mut pending) = match found {
        Some(found) => {
            if crate::diag::reporting() && server_name().is_some() {
                // safefile's one warning, if a quiet sizing made the first call
                let _ = shapefile.map(safefile);
            }
            found
        }
        None => {
            let (us, warnings) = user_init(shapefile, node);
            (us, Deferred(warnings))
        }
    };
    pending.deliver();
    graph(|g| g.epsf_nodes.insert(key, (us, pending)));
    us
}

/// `epsf_init`'s `safefile` and `user_init`, with the warnings they give.
fn user_init(shapefile: Option<&str>, node: &str) -> (Option<EpsfShape>, Vec<String>) {
    let Some((path, via_buffer)) = shapefile.and_then(safefile_from) else {
        let m = format!("shapefile not set or not found for epsf node {node}\n");
        return (None, vec![m]);
    };
    if let Some(us) = epsf_lookup(&path) {
        return (Some(us), Vec::new());
    }
    let Ok(mut f) = open(&path) else {
        return (None, vec![format!("couldn't open epsf file {path}\n")]);
    };
    let Some(bb) = bounding_box(&f.all()) else {
        let m = format!("BoundingBox not found in epsf file {path}\n");
        return (None, vec![m]);
    };
    let layout = graph(|g| g.layout_id);
    let us = run(|r| {
        let us = EpsfShape {
            bb,
            macro_id: r.n_epsf_files,
        };
        r.n_epsf_files += 1;
        let key = if via_buffer {
            EpsfKey::Static
        } else {
            EpsfKey::Graph(layout, path.clone())
        };
        // a `Dtoset` insert that finds an equal key keeps that entry; C still returns `us`
        r.epsf_dttree(&path, Some((key, us)), layout);
        us
    });
    (Some(us), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounding_box_follows_user_init() {
        assert_eq!(
            bounding_box(b"%!PS\n%%BoundingBox: 10 20 110 70\n"),
            Some([10, 20, 110, 70])
        );
        assert_eq!(
            bounding_box(b"%%BoundingBox:-1 +2\t3   4 junk"),
            Some([-1, 2, 3, 4])
        );
        // sscanf skips white space (\v too) before `%%` and before each number
        assert_eq!(
            bounding_box(b" %%BoundingBox:\x0b1 2 30 40\n"),
            Some([1, 2, 30, 40])
        );
        assert_eq!(
            bounding_box(b"% %BoundingBox: 1 2 30 40\n"),
            Some([1, 2, 30, 40])
        );
        assert_eq!(bounding_box(b"%%BoundingBox: (atend)\n"), None);
        assert_eq!(bounding_box(b"%%BoundingBox: 1 2 3\n"), None);
        // the last line wins, and a partial match overwrites the numbers it matched
        assert_eq!(
            bounding_box(b"%%BoundingBox: 0 0 100 50\n%%BoundingBox: 10 10 30 40\n"),
            Some([10, 10, 30, 40])
        );
        assert_eq!(
            bounding_box(b"%%BoundingBox: 0 0 100 50\n%%BoundingBox: 7 8 (atend)\n"),
            Some([7, 8, 100, 50])
        );
        // strtol clamps, and the int keeps the low bits
        assert_eq!(
            bounding_box(b"%%BoundingBox: -99999999999999999999 0 100 50\n"),
            Some([0, 0, 100, 50])
        );
        // a NUL ends the line for sscanf and strstr
        assert_eq!(
            bounding_box(b"%%BoundingBox: 0 0 100 50\n\0read\n%%BoundingBox: 0 0 10 10\n"),
            Some([0, 0, 10, 10])
        );
        // ... unless a non-comment line with `read` came before
        assert_eq!(
            bounding_box(b"%%BoundingBox: 0 0 100 50\nreadimage\n%%BoundingBox: 1 1 2 2\n"),
            Some([0, 0, 100, 50])
        );
    }

    #[test]
    fn safefile_resolves_like_c() {
        begin_layout(None, false, 0.0);
        assert_eq!(safefile(""), None);
        assert_eq!(safefile("x.png").as_deref(), Some("x.png"));
        // imagepath: relative names must be found in one of its directories
        begin_layout(Some("/nonexistent-dir-c18"), false, 0.0);
        assert_eq!(safefile("x.png"), None);
        assert_eq!(safefile("/abs/x.png").as_deref(), Some("/abs/x.png"));
        begin_layout(Some(""), false, 0.0);
        assert_eq!(safefile("x.png").as_deref(), Some("x.png"));
        begin_layout(None, false, 0.0);
    }

    fn scratch_file(name: &str, data: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("gvp-us347-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn sizes_are_in_points_at_the_graph_dpi() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend(40u32.to_be_bytes());
        png.extend(20u32.to_be_bytes());
        png.extend([0; 9]);
        let png = scratch_file("a.png", &png);
        let ps = scratch_file("a.ps", b"%!PS-Adobe-3.0\n%%BoundingBox: 10 20 110 70\n");
        begin_layout(None, false, 0.0);
        // 96 dpi by default; PostScript is at 72 dpi whatever the graph's
        assert_eq!(gvusershape_size(Some(&png)), (30, 15));
        assert_eq!(gvusershape_size(Some(&ps)), (100, 50));
        assert_eq!(gvusershape_size(None), (-1, -1));
        begin_layout(None, false, 150.0);
        assert_eq!(gvusershape_size(Some(&png)), (19, 9));
        let us = find(&png).unwrap();
        assert_eq!(
            (us.stringtype(), us.w, us.h, us.dpi),
            ("png", 40.0, 20.0, 0)
        );
        assert_eq!(size_dpi(&us, (72.0, 72.0)), (40, 20));
        assert_eq!(size_dpi(&find(&ps).unwrap(), (300.0, 300.0)), (100, 50));
        // the files stay loaded from graph to graph, until a graph has its own imagepath
        begin_layout(None, false, 0.0);
        assert!(find(&png).is_some());
        begin_layout(Some(""), false, 0.0);
        assert_eq!(gvusershape_size(Some(&ps)), (100, 50));
        assert!(find(&png).is_none());
        begin_layout(None, false, 0.0);
    }

    /// The tree's shape, in order: `(left key right)`, `.` for an empty subtree.
    fn shape(t: &EpsfTree, n: Option<usize>, out: &mut String) {
        let Some(n) = n else {
            out.push('.');
            return;
        };
        let EpsfKey::Graph(_, k) = &t.nodes[n].key else {
            unreachable!()
        };
        out.push('(');
        shape(t, t.nodes[n].left, out);
        out.push_str(&format!(" {k} "));
        shape(t, t.nodes[n].right, out);
        out.push(')');
    }

    // #380: EpsfTree is cdt's Dtoset. The expected shapes come from a C harness linking
    // Graphviz 16.1.0's libcdt: a Dtoset with string keys (`.size = -1`), the same
    // operations, and the tree printed from `dt->data.here` after each one. The sequence
    // reaches every branch of dttree: both zig-zigs and zig-zags, a match at the child on
    // either side, misses past both ends (`r` and `l` still `&link`) and in the middle,
    // and inserts of a new and of an existing key.
    #[test]
    fn epsf_tree_splays_like_cdt() {
        let mut t = EpsfTree::default();
        let ops = "+m +d +t +b +f +r +w +a +c +e +g ?a ?w ?e ?q ?z ?0 ?m ?c ?t ?f +m +q ?r ?b";
        let want = [
            "+m -> m : (. m .)",
            "+d -> d : (. d (. m .))",
            "+t -> t : (((. d .) m .) t .)",
            "+b -> b : (. b ((. d .) m (. t .)))",
            "+f -> f : ((. b (. d .)) f (. m (. t .)))",
            "+r -> r : ((((. b (. d .)) f .) m .) r (. t .))",
            "+w -> w : ((((((. b (. d .)) f .) m .) r .) t .) w .)",
            "+a -> a : (. a (((. b ((. d .) f .)) m (. r .)) t (. w .)))",
            "+c -> c : ((. a (. b .)) c (((. d (. f .)) m (. r .)) t (. w .)))",
            "+e -> e : (((. a (. b .)) c (. d .)) e (((. f .) m (. r .)) t (. w .)))",
            "+g -> g : ((((. a (. b .)) c (. d .)) e (. f .)) g ((. m (. r .)) t (. w .)))",
            "?a -> a : (. a (((. b .) c (. d .)) e ((. f .) g ((. m (. r .)) t (. w .)))))",
            "?w -> w : (((. a ((. b .) c (. d .))) e (((. f .) g (. m (. r .))) t .)) w .)",
            "?e -> e : ((. a ((. b .) c (. d .))) e ((((. f .) g (. m (. r .))) t .) w .))",
            "?q -> - : (((((. a ((. b .) c (. d .))) e ((. f .) g (. m .))) r .) t .) w .)",
            "?z -> - : (((((. a ((. b .) c (. d .))) e ((. f .) g (. m .))) r .) t .) w .)",
            "?0 -> - : (((. a ((. b .) c (. d .))) e (((. f .) g (. m .)) r .)) t (. w .))",
            "?m -> m : (((. a ((. b .) c (. d .))) e ((. f .) g .)) m ((. r .) t (. w .)))",
            "?c -> c : ((. a (. b .)) c ((. d .) e (((. f .) g .) m ((. r .) t (. w .)))))",
            "?t -> t : ((((. a (. b .)) c (. d .)) e (((. f .) g .) m (. r .))) t (. w .))",
            "?f -> f : ((((. a (. b .)) c (. d .)) e .) f ((. g (. m (. r .))) t (. w .)))",
            "+m -> m : (((((. a (. b .)) c (. d .)) e .) f (. g .)) m ((. r .) t (. w .)))",
            "+q -> q : ((((((. a (. b .)) c (. d .)) e .) f (. g .)) m .) q ((. r .) t (. w .)))",
            "?r -> r : (((((((. a (. b .)) c (. d .)) e .) f (. g .)) m .) q .) r (. t (. w .)))",
            "?b -> b : ((. a .) b (((. c ((. d .) e .)) f ((. g .) m .)) q (. r (. t (. w .)))))",
        ];
        for (op, want) in ops.split(' ').zip(want) {
            let (kind, k) = op.split_at(1);
            let ins = (kind == "+").then(|| {
                let us = EpsfShape {
                    bb: [0; 4],
                    macro_id: 0,
                };
                (EpsfKey::Graph(1, k.to_string()), us)
            });
            let found = t.dttree(k, ins, "", 1).is_some();
            let mut got = format!("{op} -> {} : ", if found { k } else { "-" });
            shape(&t, t.root, &mut got);
            assert_eq!(got, want);
        }
    }

    #[test]
    fn epsf_files_are_numbered_once() {
        let ps = scratch_file("e.ps", b"%!PS-Adobe-3.0\n%%BoundingBox: 10 20 110 70\n");
        let other = scratch_file("f.ps", b"%%BoundingBox: 0 0 1 2\n");
        begin_layout(None, false, 0.0);
        let a = epsf_init(Some(&ps), "a").unwrap();
        assert_eq!(a.bb, [10, 20, 110, 70]);
        assert_eq!(epsf_init(Some(&ps), "b"), Some(a));
        let c = epsf_init(Some(&other), "c").unwrap();
        assert_ne!(c.macro_id, a.macro_id);
        // a later graph's lookup by its own string does not find the earlier graph's
        begin_layout(None, false, 0.0);
        let a2 = epsf_init(Some(&ps), "a").unwrap();
        assert_ne!(a2.macro_id, a.macro_id);
    }

    // the scratch directory is an `imagepath` entry only under Unix paths (`:` separates)
    #[cfg(unix)]
    #[test]
    fn epsf_nodes_are_looked_up_once_per_layout() {
        let ps = scratch_file("g.ps", b"%%BoundingBox: 10 20 110 70\n");
        let nobb = scratch_file("nobb.ps", b"%!PS\n");
        let dir = std::path::Path::new(&ps)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // through imagepath every key is findPath's buffer, whose text the last lookup set
        begin_layout(Some(&dir), false, 0.0);
        assert_eq!(epsf_init(Some("nobb.ps"), "a"), None);
        let b = epsf_init(Some("g.ps"), "b").unwrap();
        // a's repeat sizing gets its first answer, not b's file under the buffer's key
        assert_eq!(epsf_init(Some("nobb.ps"), "a"), None);
        std::fs::write(&ps, b"no box any more\n").unwrap();
        assert_eq!(epsf_init(Some("g.ps"), "b"), Some(b));
        let _ = nobb;
        begin_layout(None, false, 0.0);
    }

    #[test]
    fn dirlist_splits_like_tok() {
        assert_eq!(mk_dirlist("a:b"), ["a", "b"]);
        assert_eq!(mk_dirlist("a::b:"), ["a", "b", ""]);
        assert_eq!(mk_dirlist(":a"), ["", "a"]);
        assert_eq!(mk_dirlist(""), [""]);
    }

    #[test]
    fn strerror_uses_posix_texts() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(strerror(&e), "No such file or directory");
        let e = io::Error::from(io::ErrorKind::Unsupported);
        assert_eq!(strerror(&e), "No such file or directory");
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(strerror(&e), "Permission denied");
    }
}
