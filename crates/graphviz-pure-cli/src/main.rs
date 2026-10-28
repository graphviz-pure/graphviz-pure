use graphviz_pure_core::Graph;
use graphviz_pure_core::charset::{Decoded, decode_slice};
use graphviz_pure_core::diag::{AgErrLevel, agerrorf, agseterr, agwarningf, fprintf_stderr};
use graphviz_pure_layout::fdpgen::tlayout::FdpParms;
use graphviz_pure_layout::{LayoutEngine, LayoutOptions};
use graphviz_pure_parser::GlobalAttrs;
use graphviz_pure_render::decoded::{self, GraphJobs, LayoutJobs, graph_stream};
use graphviz_pure_render::plugins::{self, DEVICE_PLUGINS, LAYOUT_PLUGINS};
use graphviz_pure_render::{OutputFormat, set_y_invert};
use std::borrow::Cow;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

/// `gvc->common.info`: `"graphviz version 16.1.0 (graphviz-pure X.Y.Z)"`. The path to
/// `env!` keeps it apart from the `std::env` import when examples/heap_counts.rs includes this file.
macro_rules! version_info {
    () => {
        concat!(
            "graphviz version 16.1.0 (graphviz-pure ",
            ::core::env!("CARGO_PKG_VERSION"),
            ")"
        )
    };
}
const VERSION: &str = concat!("graphviz-pure - ", version_info!());
/// What `-v` writes after the command name.
const VERSION_INFO: &str = version_info!();

// `lib/common/input.c`: `dotneato_usage`'s texts.
const USAGE_FMT: &str = " [-Vv?] [-(GNEA)name=val] [-(KTlso)<val>] <dot files>\n";
const NEATO_FLAGS: &str = "(additional options for neato)    [-x] [-n<v>]\n";
const FDP_FLAGS: &str = "(additional options for fdp)      [-L(gO)] [-L(nUCT)<val>]\n";
const CONFIG_FLAGS: &str = "(additional options for config)  [-cv]\n";
// The trailing spaces of three lines are C's.
const GENERIC_ITEMS: &str = concat!(
    "\n",
    " -V          - Print version and exit\n",
    " -v[n]       - Enable verbose mode at level 1, or set to the level 'n' if provided\n",
    " -Gname=val  - Set graph attribute 'name' to 'val'\n",
    " -Nname=val  - Set node attribute 'name' to 'val'\n",
    " -Ename=val  - Set edge attribute 'name' to 'val'\n",
    " -Aname=val  - Set attribute 'name' to 'val' for graph, node, and edge\n",
    " -Tv         - Set output format to 'v'\n",
    " -Kv         - Set layout engine to 'v' (overrides default based on command name)\n",
    " -lv         - Use external library 'v'\n",
    " -ofile      - Write output to 'file'\n",
    " -O          - Automatically generate an output filename based on the input filename \
     with a .'format' appended. (Causes all -ofile options to be ignored.) \n",
    " -P          - Internally generate a graph of the current plugins. \n",
    " -q[l]       - Set level of message suppression (=1)\n",
    " -s[v]       - Scale input by 'v' (=72)\n",
    " -y          - Invert y coordinate in output\n",
);
const NEATO_ITEMS: &str = concat!(
    "\n",
    " -n[v]       - No layout mode 'v' (=1)\n",
    " -x          - Reduce graph\n",
);
const FDP_ITEMS: &str = concat!(
    "\n",
    " -Lg         - Don't use grid\n",
    " -LO         - Use old attractive force\n",
    " -Lm<l>      - Set m-limit to l\n",
    " -Ln<i>      - Set number of iterations to i\n",
    " -LU<i>      - Set unscaled factor to i\n",
    " -LC<v>      - Set overlap expansion factor to v\n",
    " -LT[*]<v>   - Set temperature (temperature factor) to v\n",
);
const CONFIG_ITEMS: &str = concat!(
    "\n",
    " -c          - Configure plugins (Writes $prefix/lib/graphviz/config \n",
    "               with available plugin information.  Needs write privilege.)\n",
    " -?          - Print usage and exit\n",
);

/// One `-T`/`-o` job (`GVJ_t`): the output format and file, and the file once opened.
struct Job {
    /// `output_langname`: the `-T` value as given (`svg`, `png:cairo`).
    langname: Option<String>,
    /// `output_filename`; `None` writes to stdout.
    filename: Option<OsString>,
    /// `output_file` while the job's device is open.
    file: Option<Sink>,
}

enum Sink {
    Stdout,
    File(io::BufWriter<fs::File>),
}

impl Sink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Sink::Stdout => io::stdout().write_all(bytes),
            Sink::File(f) => f.write_all(bytes),
        }
    }
}

struct Options {
    /// The command name (`gvc->common.cmdname`), for messages.
    cmdname: String,
    engine: LayoutEngine,
    jobs: Vec<Job>,
    auto_output: bool,
    globals: GlobalAttrs,
    layout: LayoutOptions,
    y_invert: bool,
    inputs: Vec<OsString>,
    /// `-P`: lay out and render the plugin graph instead of the inputs (`P_graph`), built on
    /// the `-G`/`-N`/`-E`/`-A` attributes given before it.
    plugin_graph: Option<GlobalAttrs>,
}

enum Parsed {
    Run(Box<Options>),
    Exit(ExitCode),
}

/// `dotneato_usage(argv0, exval)`: the usage text, on stdout for `-?` and on stderr after
/// an error, which then ends the run with status 1.
fn usage(argv0: &str, exval: u8) -> ExitCode {
    let text = format!(
        "Usage: {argv0}{USAGE_FMT}{NEATO_FLAGS}{FDP_FLAGS}{CONFIG_FLAGS}{GENERIC_ITEMS}\
         {NEATO_ITEMS}{FDP_ITEMS}{CONFIG_ITEMS}"
    );
    if exval > 0 {
        fprintf_stderr(&text);
    } else {
        print!("{text}");
    }
    ExitCode::from(exval)
}

/// An argument as text. C's `main` gets each argument as bytes, so the bytes of one that is
/// not valid UTF-8 are kept as raw-byte chars (`graphviz_pure_core::charset`): an attribute
/// set with `-G` carries them into the output, and an error message names them as C does.
fn arg_text(arg: &OsStr) -> String {
    decode_slice(&arg_bytes(arg))
}

/// `strerror(errno)` for an I/O error: its text without Rust's ` (os error N)`.
fn strerror(e: &io::Error) -> String {
    let text = e.to_string();
    match (e.raw_os_error(), text.rfind(" (os error ")) {
        (Some(_), Some(at)) => text[..at].to_string(),
        _ => text,
    }
}

/// `strtol(s, &end, 10)`: the value, or `None` when no digits were read (`end == s`).
fn strtol(s: &str) -> Option<i64> {
    match graphviz_pure_core::cnum::strtol(s) {
        (_, 0) => None,
        (v, _) => Some(v),
    }
}

use graphviz_pure_core::cnum::atoi;

/// `strtod(s, &end)`: the value, or `None` when nothing was read (`end == s`).
fn strtod(s: &str) -> Option<f64> {
    match graphviz_pure_core::strtod::strtod(s) {
        (_, 0) => None,
        (v, _) => Some(v),
    }
}

/// The bytes of an argument, as C's `argv` holds them (UTF-8 where the OS has UTF-16).
/// Flags are split on these, not on [`arg_text`], whose chars can stand for several bytes.
fn arg_bytes(arg: &OsStr) -> Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Cow::Borrowed(arg.as_bytes())
    }
    #[cfg(not(unix))]
    {
        Cow::Owned(arg.to_string_lossy().into_owned().into_bytes())
    }
}

/// The file name after a two-letter flag (`-ofile`), with the argument's own bytes.
fn os_tail(arg: &OsStr) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        OsStr::from_bytes(&arg.as_bytes()[2..]).to_os_string()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        OsString::from_wide(&arg.encode_wide().skip(2).collect::<Vec<_>>())
    }
    #[cfg(not(any(unix, windows)))]
    {
        OsString::from(String::from_utf8_lossy(&arg_bytes(arg)[2..]).into_owned())
    }
}

/// Mirrors `getFlagOpt`: the text after the flag letter, else the next argument unless it
/// starts with `-`.
fn flag_value(args: &[OsString], i: &mut usize) -> Option<String> {
    let arg = arg_bytes(&args[*i]);
    if arg.len() > 2 {
        return Some(decode_slice(&arg[2..]));
    }
    let next = arg_text(args.get(*i + 1)?);
    if !next.is_empty() && !next.starts_with('-') {
        *i += 1;
        return Some(next);
    }
    None
}

/// Mirrors `global_def`: `name=value`, or a bare `name` meaning `true`.
fn global_def(spec: &str) -> (String, String) {
    match spec.split_once('=') {
        Some((name, value)) => (name.to_string(), value.to_string()),
        None => (spec.to_string(), "true".to_string()),
    }
}

/// `dotneato_basename`: the command name in an `argv[0]`. On Windows, as there, without its
/// `.exe` and in lower case.
fn basename(argv0: &str) -> String {
    #[cfg(windows)]
    let argv0 = match argv0.rfind('.') {
        Some(dot) if argv0[dot + 1..].eq_ignore_ascii_case("exe") => &argv0[..dot],
        _ => argv0,
    };
    let trimmed = argv0.trim_end_matches(['/', '\\']);
    let name = if trimmed.is_empty() {
        &argv0[..argv0.len().min(1)]
    } else {
        match trimmed.rfind(['/', '\\']) {
            Some(at) => &trimmed[at + 1..],
            None => trimmed,
        }
    };
    if cfg!(windows) {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

/// `setFDPAttr`: one `-L` flag (`arg` is what follows `-L`). `Err` after `bad value`, which
/// ends the run with the usage text.
fn set_fdp_attr(fdp: &mut FdpParms, arg: &str) -> Result<(), ()> {
    let mut chars = arg.chars();
    let flag = chars.next();
    let val = chars.as_str();
    // `setDouble`/`setInt` name the flag from the character before the value they were
    // given: the flag letter, or the `*` of `-LT*`.
    let bad = |v: &str| {
        let named = &arg[arg.len() - v.len() - 1..];
        agerrorf(&format!("bad value in flag -L{named} - ignored\n"));
    };
    let double = |v: &str| strtod(v).ok_or_else(|| bad(v));
    let int = |v: &str| strtol(v).map(|i| i as i32).ok_or_else(|| bad(v));
    match flag {
        Some('g') => fdp.use_grid = false,
        Some('O') => fdp.use_new = false,
        Some('m') => fdp.mlimit = double(val)?,
        Some('n') => fdp.num_iters = int(val)?,
        Some('U') => fdp.unscaled = int(val)?,
        Some('C') => fdp.c = double(val)?,
        Some('T') => match val.strip_prefix('*') {
            Some(v) => fdp.tfact = double(v)?,
            None => fdp.t0 = double(val)?,
        },
        _ => agwarningf(&format!("unknown flag -L{arg} - ignored\n")),
    }
    Ok(())
}

/// `gvjobs_output_langname`/`gvjobs_output_filename`: the `n`th `-T` and the `n`th `-o`
/// fill the `n`th job.
fn job_at(jobs: &mut Vec<Job>, n: usize) -> &mut Job {
    while jobs.len() <= n {
        jobs.push(Job {
            langname: None,
            filename: None,
            file: None,
        });
    }
    &mut jobs[n]
}

/// Mirrors `gvParseArgs`: `neato_extra_args`, `fdp_extra_args` and `config_extra_args` take
/// their flags out of the arguments in turn, then `dotneato_args_initialize` reads the rest.
/// File names (`-o` and the inputs) are kept as the OS gave them, so a name that is not valid
/// UTF-8 still opens.
fn parse_args(argv0: &str, os_args: &[OsString]) -> Parsed {
    let cmdname = basename(argv0);
    let mut layout = LayoutOptions::default();
    let mut auto_output = false;
    let mut config = false;

    // `neato_extra_args`: `-n` and `-x`.
    let mut kept: Vec<OsString> = Vec::with_capacity(os_args.len());
    for os_arg in os_args {
        let bytes = arg_bytes(os_arg);
        if bytes.len() < 2 || bytes[0] != b'-' || !matches!(bytes[1], b'n' | b'x') {
            kept.push(os_arg.clone());
            continue;
        }
        if bytes[1] == b'x' {
            layout.set_reduce(true);
            continue;
        }
        let rest = decode_slice(&bytes[2..]);
        if rest.is_empty() {
            layout.set_nop(1);
            continue;
        }
        // `Nop = atoi(arg + 2)`, an error when not positive.
        match atoi(&rest) {
            n if n > 0 => layout.set_nop(n.min(255) as u8),
            _ => {
                agerrorf(&format!("Invalid parameter \"{rest}\" for -n flag\n"));
                return Parsed::Exit(usage(argv0, 1));
            }
        }
    }
    // `fdp_extra_args`: `-L`.
    let os_args = std::mem::take(&mut kept);
    for os_arg in os_args {
        let bytes = arg_bytes(&os_arg);
        if bytes.len() < 2 || bytes[0] != b'-' || bytes[1] != b'L' {
            kept.push(os_arg.clone());
            continue;
        }
        if set_fdp_attr(layout.fdp_mut(), &decode_slice(&bytes[2..])).is_err() {
            return Parsed::Exit(usage(argv0, 1));
        }
    }
    // `config_extra_args`: `-v`, `-O` and `-c`.
    let os_args = std::mem::take(&mut kept);
    for os_arg in os_args {
        let bytes = arg_bytes(&os_arg);
        match (bytes.first(), bytes.get(1)) {
            // `-v` is level 1, `-vN` level N when a digit follows.
            (Some(b'-'), Some(b'v')) => plugins::set_verbose(match bytes.get(2) {
                // `atoi`'s `int`: the value's low 32 bits, as C keeps of a longer one.
                Some(d) if d.is_ascii_digit() => atoi(&decode_slice(&bytes[2..])) as i32,
                _ => 1,
            }),
            (Some(b'-'), Some(b'O')) => auto_output = true,
            (Some(b'-'), Some(b'c')) => config = true,
            _ => kept.push(os_arg.clone()),
        }
    }

    // `dotneato_args_initialize`: running as a web server disables file loading.
    graphviz_pure_core::usershape::set_server_name(
        env::var_os("SERVER_NAME").map(|v| arg_text(&v)).as_deref(),
    );
    graphviz_pure_core::usershape::set_filepath(None);
    if env::var_os("GV_FILE_PATH").is_some() {
        fprintf_stderr(
            "$GV_FILE_PATH environment variable set; exiting\n\n\
             This sandboxing mechanism is no longer supported\n",
        );
        return Parsed::Exit(ExitCode::FAILURE);
    }
    // `dot -c` writes the plugin configuration and exits; the port's plugins are built in.
    if plugins::verbose() != 0 {
        fprintf_stderr(&format!("{cmdname} - {VERSION_INFO}\n"));
    }
    if config {
        return Parsed::Exit(ExitCode::SUCCESS);
    }
    let os_args = kept.as_slice();
    let mut opts = Options {
        cmdname,
        engine: LayoutEngine::Dot,
        jobs: Vec::new(),
        auto_output,
        globals: GlobalAttrs::default(),
        layout,
        y_invert: false,
        inputs: Vec::new(),
        plugin_graph: None,
    };
    let (mut n_lang, mut n_file) = (0, 0);
    let mut kflag = false;
    let missing = |flag: char| {
        fprintf_stderr(&format!("Missing argument for -{flag} flag\n"));
        Parsed::Exit(usage(argv0, 1))
    };

    let mut i = 0;
    while i < os_args.len() {
        let arg = arg_text(&os_args[i]);
        let bytes = arg_bytes(&os_args[i]);
        if arg.starts_with("-V") || arg == "--version" {
            fprintf_stderr(&format!("{VERSION}\n"));
            return Parsed::Exit(ExitCode::SUCCESS);
        } else if arg.starts_with("-?") || arg == "--help" {
            return Parsed::Exit(usage(argv0, 0));
        } else if let Some(dirs) = arg.strip_prefix("--filepath=") {
            // `Gvfilepath`: the directories image files are looked up in.
            graphviz_pure_core::usershape::set_filepath(Some(dirs));
        } else if bytes.first() == Some(&b'-') {
            // `dotneato_args_initialize` switches on the byte `arg[1]`, which need not start
            // a char (and is the terminating NUL for `-` alone).
            let flag = match bytes.get(1) {
                None => '\0',
                Some(&b) if b.is_ascii() => b as char,
                Some(&b) => graphviz_pure_core::charset::raw_char(b),
            };
            let rest = decode_slice(bytes.get(2..).unwrap_or_default());
            let rest = rest.as_str();
            match flag {
                'G' | 'N' | 'E' | 'A' => {
                    if rest.is_empty() {
                        return missing(flag);
                    }
                    let (name, value) = global_def(rest);
                    if matches!(flag, 'G' | 'A') {
                        opts.globals.graph.insert(name.clone(), value.clone());
                    }
                    if matches!(flag, 'N' | 'A') {
                        opts.globals.node.insert(name.clone(), value.clone());
                    }
                    if matches!(flag, 'E' | 'A') {
                        opts.globals.edge.insert(name, value);
                    }
                }
                'T' => {
                    let Some(v) = flag_value(os_args, &mut i) else {
                        return missing('T');
                    };
                    job_at(&mut opts.jobs, n_lang).langname = Some(v.clone());
                    n_lang += 1;
                    if plugins::load_device(&v).is_none() {
                        // `dotneato_args_initialize` writes these with `fprintf`.
                        fprintf_stderr(&format!(
                            "Format: \"{v}\" not recognized. Use one of:{}\n",
                            plugins::plugin_list(&DEVICE_PLUGINS, &v)
                        ));
                        return Parsed::Exit(ExitCode::FAILURE);
                    }
                }
                'K' => {
                    let Some(v) = flag_value(os_args, &mut i) else {
                        return missing('K');
                    };
                    match plugins::layout_select(&v) {
                        Some(e) => opts.engine = e,
                        None => {
                            fprintf_stderr(&format!(
                                "There is no layout engine support for \"{v}\"\n Use one of:{}\n",
                                plugins::plugin_list(&LAYOUT_PLUGINS, &v)
                            ));
                            return Parsed::Exit(ExitCode::FAILURE);
                        }
                    }
                    kflag = true;
                }
                // `-P` renders the graph of the plugins instead of the inputs.
                'P' => opts.plugin_graph = Some(opts.globals.clone()),
                'l' => {
                    if flag_value(os_args, &mut i).is_none() {
                        return missing('l');
                    }
                }
                'o' => {
                    let at = i;
                    let name = match flag_value(os_args, &mut i) {
                        Some(_) if i > at => os_args[i].clone(),
                        Some(_) => os_tail(&os_args[i]),
                        None => return missing('o'),
                    };
                    if !opts.auto_output {
                        job_at(&mut opts.jobs, n_file).filename = Some(name);
                        n_file += 1;
                    }
                }
                // `-q` is `-q1`: warnings are dropped; `-q2` and up drop errors too.
                'q' => {
                    if rest.is_empty() {
                        agseterr(AgErrLevel::Err);
                    } else {
                        match atoi(rest) {
                            ..=0 => fprintf_stderr(&format!(
                                "Invalid parameter \"{rest}\" for -q flag - ignored\n"
                            )),
                            1 => {
                                agseterr(AgErrLevel::Err);
                            }
                            _ => {
                                agseterr(AgErrLevel::Max);
                            }
                        }
                    }
                }
                // `PSinputscale`: `-s` alone and `-s0` are 72.
                's' => {
                    let scale = if rest.is_empty() {
                        72.0
                    } else {
                        graphviz_pure_core::strtod::atof(rest)
                    };
                    if scale < 0.0 {
                        fprintf_stderr(&format!("Invalid parameter \"{rest}\" for -s flag\n"));
                        return Parsed::Exit(usage(argv0, 1));
                    }
                    // is_exactly_zero: `-s-0` keeps -0, which then does not scale.
                    opts.layout.set_inputscale(
                        if graphviz_pure_core::cnum::is_exactly_zero(scale) {
                            72.0
                        } else {
                            scale
                        },
                    );
                }
                'y' => opts.y_invert = true,
                _ => {
                    agerrorf(&format!(
                        "{}: option -{flag} unrecognized\n\n",
                        opts.cmdname
                    ));
                    return Parsed::Exit(usage(argv0, 1));
                }
            }
        } else {
            opts.inputs.push(os_args[i].clone());
        }
        i += 1;
    }

    // Without `-K`, the layout is the command's name, `dot` for any other name (as C's
    // `dot_static` and `dot_builtins` are), selected as `gvlayout_select` does.
    if !kflag {
        let layout = match plugins::plugin_load(&LAYOUT_PLUGINS, &opts.cmdname) {
            Some(_) => opts.cmdname.as_str(),
            None => "dot",
        };
        opts.engine = plugins::layout_select(layout).unwrap_or(LayoutEngine::Dot);
    }
    // Without `-T`, the first job writes `dot`; a job with no `-T` repeats the one before
    // it (`gvjobs_next`).
    let first = job_at(&mut opts.jobs, 0);
    if first.langname.is_none() {
        first.langname = Some("dot".to_string());
        plugins::load_device("dot");
    }
    for n in 1..opts.jobs.len() {
        if opts.jobs[n].langname.is_none() {
            opts.jobs[n].langname = opts.jobs[n - 1].langname.clone();
        }
    }
    // `gvParseArgs`: the plugins, once the arguments are read.
    if graphviz_pure_core::diag::verbose_byte() != 0 {
        plugins::write_status(&plugins::PORT_PLUGINS);
    }
    Parsed::Run(Box::new(opts))
}

/// Reads a DOT file. C reads bytes; bytes that are not part of valid UTF-8 are
/// kept as raw-byte chars (`graphviz_pure_core::charset`), so labels are converted once,
/// where C converts them, and the output carries C's bytes.
fn read_input(path: Option<&OsStr>) -> io::Result<Decoded> {
    let bytes = match path {
        Some(p) => fs::read(p)?,
        None => {
            let mut buf = Vec::new();
            io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    Ok(Decoded::decode(bytes))
}

/// `FINISH()` in `gvRenderJobs` (`lib/common/emit.c`, at `line`): the `-v` timing.
fn render_jobs_finish(graph: &Graph, line: u32) {
    graphviz_pure_core::gv_info!(
        "emit.c",
        line,
        "gvRenderJobs {}: {} secs.",
        graph.root_name(),
        graphviz_pure_core::diag::fmt_f(graphviz_pure_core::diag::elapsed_sec(), 2)
    );
}

/// `agerrorf` of an input's error, as C's bytes. A syntax error names the file `aagerror`
/// knows: the input, or the one a `# N "file"` line directive named.
fn report_error(name: &str, e: &graphviz_pure_core::GraphvizError) {
    let name = e.file().unwrap_or(name);
    agerrorf(&format!("{name}: {e}\n"));
}

/// `auto_output_filename`: `<input>[.<graph number>].<format>`, with the parts of a
/// `format:renderer:package` name in reverse order (`noname.gv` for stdin).
fn auto_output_filename(input: Option<&OsStr>, graph_index: usize, langname: &str) -> OsString {
    let mut name = input.map_or_else(|| OsString::from("noname.gv"), OsStr::to_os_string);
    if graph_index > 0 {
        name.push(format!(".{}", graph_index + 1));
    }
    let parts: Vec<&str> = langname.split(':').collect();
    let mut suffix = String::new();
    for p in parts.iter().rev() {
        suffix.push('.');
        suffix.push_str(p);
    }
    name.push(suffix);
    name
}

/// The output of one job for one graph: text formats from their writer, raster formats
/// drawn from the SVG (graphviz-pure-raster).
fn render_job(
    jobs: &mut GraphJobs<'_>,
    job: usize,
    graph: &Graph,
    langname: &str,
) -> Result<Vec<u8>, graphviz_pure_core::GraphvizError> {
    let typ = langname.split(':').next().unwrap_or("");
    if let Some(fmt) = graphviz_pure_raster::RasterFormat::parse(typ) {
        // Graphviz draws raster formats at 96 dpi unless the graph sets `dpi` (or
        // `resolution`), which then scales the drawing.
        let explicit_dpi = graphviz_pure_core::cnum::drawing_dpi(&graph.graph_attrs) > 0.0;
        let svg = jobs.render_raster_svg_as(job, langname)?;
        return graphviz_pure_raster::rasterize(&svg, fmt, explicit_dpi)
            .map(|rendered| rendered.into_output());
    }
    let text = jobs.render_as(job, OutputFormat::parse(typ)?, langname)?;
    Ok(graphviz_pure_core::charset::encode(&text).into_owned())
}

/// The run's state across graphs: `gvRenderJobs`'s jobs and active job, and `dot`'s
/// error counts.
struct Run<'a> {
    opts: &'a mut Options,
    layout: LayoutJobs,
    /// `gvc->active_jobs`: the job whose device is open.
    active: Option<usize>,
    /// `rc` in `dot`'s `main`: the highest error level of a graph.
    rc: i32,
    /// `graphviz_errors`: inputs that could not be opened.
    graphviz_errors: i32,
}

impl Run<'_> {
    /// `gvrender_end_job`: the device of `n` is finalized; a file is closed, and the job
    /// then writes to stdout (`gvdevice_close` forgets its file name).
    fn end_job(&mut self, n: usize) -> io::Result<()> {
        let job = &mut self.opts.jobs[n];
        match job.file.take() {
            Some(Sink::File(mut f)) => {
                job.filename = None;
                f.flush()?;
            }
            Some(Sink::Stdout) => {
                io::stdout().flush()?;
                job.file = Some(Sink::Stdout);
            }
            None => {}
        }
        Ok(())
    }

    /// `gvRenderJobs` for one graph. `Err` is a write error.
    fn render_jobs(
        &mut self,
        graph: &Graph,
        input: Option<&OsStr>,
        graph_index: usize,
        name: &str,
    ) -> io::Result<()> {
        if graphviz_pure_core::diag::verbose_on() {
            graphviz_pure_core::diag::start_timer();
        }
        // FINISH() on every way out
        struct Finish<'g>(&'g Graph);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                render_jobs_finish(self.0, 4323);
            }
        }
        let _finish = Finish(graph);
        let mut graph_jobs = GraphJobs::new(graph);
        // the jobs have output files: C's `fprintf(job->output_file, ...)` text goes in them
        graph_jobs.write_output_file_text(true);
        for n in 0..self.opts.jobs.len() {
            // `gvrender_select`: the job's device, and the renderer it depends on, are
            // looked up for each graph (the `-T` value was checked already).
            let langname = self.opts.jobs[n].langname.as_deref().unwrap_or_default();
            plugins::load_device(langname);
            if plugins::verbose() != 0 && plugins::load_device_quietly(langname) == Some("json") {
                // `json_begin_graph` renders the graph as xdot first (`gvRender`), which
                // looks the device up three times: `gvjobs_output_langname`, its own
                // `gvrender_select` and `gvRenderJobs`'s.
                for _ in 0..3 {
                    plugins::load_device("xdot");
                }
                // that gvRender is a gvRenderJobs of its own, which restarts the shared
                // clock and reports when it finishes
                graphviz_pure_core::diag::start_timer();
                render_jobs_finish(graph, 4323);
            }
            // Every job's device writes its own output (no format of the port does pages),
            // so the device open before it is finalized.
            if let Some(active) = self.active.take() {
                self.end_job(active)?;
            }
            // `gvrender_begin_job` → `gvdevice_initialize`: open the job's file unless
            // it still is.
            let job = &mut self.opts.jobs[n];
            let langname = job.langname.clone().unwrap_or_default();
            if job.file.is_none() {
                if self.opts.auto_output {
                    job.filename = Some(auto_output_filename(input, graph_index, &langname));
                }
                job.file = Some(match &job.filename {
                    Some(path) => match fs::File::create(path) {
                        Ok(f) => Sink::File(io::BufWriter::new(f)),
                        Err(e) => {
                            agerrorf(&format!(
                                "Could not open \"{}\" for writing : {}\n",
                                arg_text(path),
                                strerror(&e)
                            ));
                            continue;
                        }
                    },
                    None => Sink::Stdout,
                });
            }
            self.active = Some(n);
            let rendered = render_job(&mut graph_jobs, n, graph, &langname);
            // what the job wrote to the standard output (`-Tjson`'s xdot job) came first
            let stdout = graph_jobs.take_stdout();
            if !stdout.is_empty() {
                let mut out = io::stdout();
                out.write_all(&graphviz_pure_core::charset::encode(&stdout))?;
                out.flush()?;
            }
            match rendered {
                Ok(bytes) => {
                    let sink = self.opts.jobs[n].file.as_mut().expect("opened above");
                    sink.write_all(&bytes)?;
                }
                Err(e) => report_error(name, &e),
            }
        }
        Ok(())
    }

    /// `gvLayoutJobs` and `gvRenderJobs` for one graph. `Ok(false)` when the layout failed
    /// where C exits or crashes, which ends the run; `Err` is a write error.
    fn graph(
        &mut self,
        graph: &mut Graph,
        input: Option<&OsStr>,
        graph_index: usize,
        name: &str,
    ) -> io::Result<bool> {
        match self.layout.layout(graph) {
            Ok(true) => self.render_jobs(graph, input, graph_index, name)?,
            Ok(false) => {
                if graphviz_pure_core::diag::verbose_on() {
                    graphviz_pure_core::diag::start_timer();
                }
                agerrorf("Layout was not done.  Missing layout plugins? \n");
                render_jobs_finish(graph, 4212);
            }
            // A layout the port stops where C exits (`graphviz_exit`) or crashes.
            Err(e) => {
                report_error(name, &e);
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `-P`: the plugin graph (`gvPluginsGraph`), named `<internal>`, laid out and rendered
    /// instead of the inputs, which are not read.
    fn plugin_graph(&mut self, globals: &GlobalAttrs) -> io::Result<bool> {
        // The port's tables are fixed and all quotable (`plugin_graph`'s test).
        let Some(mut graph) = decoded::plugin_graph(&plugins::PORT_PLUGINS, globals) else {
            agerrorf("the plugin graph could not be made\n");
            return Ok(true);
        };
        let name = "<internal>";
        self.graph(&mut graph, Some(OsStr::new(name)), 0, name)
    }

    /// One input (`gvNextInputGraph` over a file): each graph is read, laid out and
    /// rendered before the next is read. `Ok(false)` when a layout failed where C exits or
    /// crashes, which ends the run; `Err` is a write error.
    fn input(&mut self, input: Option<&OsStr>) -> io::Result<bool> {
        let name = input.map_or_else(|| "<stdin>".to_string(), arg_text);
        let source = match read_input(input) {
            Ok(s) => s,
            Err(e) => {
                agerrorf(&format!(
                    "{}: can't open {name}: {}\n",
                    self.opts.cmdname,
                    strerror(&e)
                ));
                self.graphviz_errors += 1;
                return Ok(true);
            }
        };
        // `agread` on the file: read through `fgets`, named as `agconcat`'s `filename`.
        graphviz_pure_parser::reading_file(&name, || {
            let globals = self.opts.globals.clone();
            let mut stream = graph_stream(&source, &globals);
            let mut graph_index = 0;
            while let Some(read) = stream.next_graph(|e| report_error(&name, e)) {
                let Ok(mut graph) = read else { break };
                if !self.graph(&mut graph, input, graph_index, &name)? {
                    return Ok(false);
                }
                graph_index += 1;
                // `agreseterrors`: the graph's errors set the status.
                let errors = graphviz_pure_core::diag::error_count();
                graphviz_pure_core::diag::reset_error_count();
                self.rc = self.rc.max(i32::from(errors > 0));
            }
            Ok(true)
        })
    }
}

/// The stack `dot`'s work runs on. The layouts and writers recurse once per subgraph level,
/// as C's do on its 8 MB main stack, and the parser takes the 3331 levels bison's stack
/// allows (#118). Measured on macOS arm64 with that nesting: the release build needs 2 to
/// 4 MB in every engine and format, the debug build less than 4 MB. Windows gives the main
/// thread 1 MB.
const DOT_STACK: usize = 64 << 20;

fn main() -> ExitCode {
    match std::thread::Builder::new()
        .name("dot".into())
        .stack_size(DOT_STACK)
        .spawn(dot_main)
    {
        Ok(work) => work
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(_) => dot_main(),
    }
}

fn dot_main() -> ExitCode {
    // `args_os`: `args` panics on an argument that is not valid UTF-8.
    let raw: Vec<OsString> = env::args_os().collect();
    let argv0 = raw
        .first()
        .map_or_else(|| "dot".to_string(), |a| arg_text(a));
    let mut opts = match parse_args(&argv0, raw.get(1..).unwrap_or_default()) {
        Parsed::Run(o) => o,
        Parsed::Exit(code) => return code,
    };
    set_y_invert(opts.y_invert);
    // `-P` reads no input.
    let inputs: Vec<Option<OsString>> = if opts.plugin_graph.is_some() {
        Vec::new()
    } else if opts.inputs.is_empty() {
        vec![None]
    } else {
        opts.inputs.iter().cloned().map(Some).collect()
    };
    let layout = LayoutJobs::new(opts.engine, opts.layout.clone());
    let mut run = Run {
        opts: &mut opts,
        layout,
        active: None,
        rc: 0,
        graphviz_errors: 0,
    };
    let mut written = Ok(());
    let mut stopped = false;
    if let Some(globals) = run.opts.plugin_graph.clone() {
        match run.plugin_graph(&globals) {
            Ok(true) => {}
            Ok(false) => stopped = true,
            Err(e) => written = Err(e),
        }
    }
    for input in &inputs {
        match run.input(input.as_deref()) {
            Ok(true) => {}
            Ok(false) => {
                stopped = true;
                break;
            }
            Err(e) => {
                written = Err(e);
                break;
            }
        }
    }
    // `gvFinalize`: the last active job is finalized.
    if written.is_ok()
        && let Some(active) = run.active.take()
    {
        written = run.end_job(active);
    }
    if let Err(e) = written {
        fprintf_stderr(&format!("{}: write error: {e}\n", run.opts.cmdname));
        return ExitCode::FAILURE;
    }
    if stopped {
        return ExitCode::FAILURE;
    }
    // `main` returns the larger of the graphs' statuses and `gvFreeContext`'s
    // `graphviz_errors + agerrors()`.
    let errors = i32::from(graphviz_pure_core::diag::error_count() > 0);
    let status = run.rc.max(run.graphviz_errors + errors);
    // `exit` keeps the status's low 8 bits.
    ExitCode::from(status as u8)
}
