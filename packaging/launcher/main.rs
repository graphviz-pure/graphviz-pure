//! Baseline x86-64 launcher shared by wheels and the container. No dependencies.
#![forbid(unsafe_code)]

use std::{env, io, path::PathBuf, process::Command};

fn has_v3() -> bool {
    // v3 includes all v2 features too. Rust's AVX detection checks OSXSAVE/XCR0,
    // so a CPU advertising AVX with no OS support must take the compat path.
    // LAHF/SAHF in long mode has no safe std detection macro; all AVX2 CPUs
    // provide it (LLVM assumes it for x86-64-v2/v3).
    is_x86_feature_detected!("sse3")
        && is_x86_feature_detected!("ssse3")
        && is_x86_feature_detected!("sse4.1")
        && is_x86_feature_detected!("sse4.2")
        && is_x86_feature_detected!("popcnt")
        && is_x86_feature_detected!("cmpxchg16b")
        && is_x86_feature_detected!("avx")
        && is_x86_feature_detected!("avx2")
        && is_x86_feature_detected!("fma")
        && is_x86_feature_detected!("f16c")
        && is_x86_feature_detected!("bmi1")
        && is_x86_feature_detected!("bmi2")
        && is_x86_feature_detected!("lzcnt")
        && is_x86_feature_detected!("movbe")
        && is_x86_feature_detected!("xsave")
}

fn run() -> io::Result<()> {
    // Some Unix environments do not mount /proc. Try argv[0] (including PATH
    // lookup for a bare command) when the platform cannot locate this executable.
    let path = env::current_exe()
        .or_else(|error| {
            let arg = env::args_os().next().ok_or(error)?;
            let path = PathBuf::from(&arg);
            if path.components().count() > 1 || path.is_absolute() {
                return Ok(path);
            }
            env::var_os("PATH")
                .and_then(|paths| {
                    env::split_paths(&paths)
                        .map(|dir| dir.join(&arg))
                        .find(|p| p.is_file())
                })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "cannot locate launcher via argv[0] or PATH",
                    )
                })
        })?
        .canonicalize()?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("launcher has no parent"))?;
    let variant = if has_v3() { "v3" } else { "compat" };
    let binary = parent
        .join(variant)
        .join(format!("graphviz-pure{}", env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "install the launcher together with v3/graphviz-pure and compat/graphviz-pure (with .exe on Windows)",
        ));
    }
    let mut command = Command::new(binary);
    command.args(env::args_os().skip(1));
    // Internal launcher/CLI handoff: Command cannot set argv[0] on Windows.
    // Always replace any inherited value with the name used for this invocation.
    command.env(
        "GRAPHVIZ_PURE_ARGV0",
        env::args_os().next().unwrap_or_default(),
    );
    // Command inherits all three standard streams. exec preserves Unix signals
    // and status exactly, without a shell or an unsafe system-call wrapper.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0(env::args_os().next().unwrap_or_default());
        Err(command.exec())
    }
    #[cfg(windows)]
    {
        std::process::exit(command.status()?.code().unwrap_or(1));
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("graphviz-pure launcher: {error}");
        std::process::exit(1);
    }
}
