//! The command line under dhat's counting allocator, for the perf ratchet
//! (scripts/bench/ratchet.py, docs/perf/ratchet.md).
//!
//! Takes the same arguments as `graphviz-pure`, runs it, and prints one JSON line of heap
//! statistics on stderr after it: `{"heap_blocks":…,"heap_bytes":…,"heap_peak_bytes":…}`,
//! the allocations, the bytes they asked for, and the largest heap live at once. They count
//! the whole process, so the argument parsing and the output too.
//!
//! An example, so `dhat` stays a dev-dependency: no shipped binary has it as its allocator.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

mod cli {
    include!("../src/main.rs");

    pub fn run() -> ExitCode {
        main()
    }
}

fn main() -> std::process::ExitCode {
    // Testing mode writes no dhat-heap.json; only the statistics below are read.
    let _profiler = dhat::Profiler::builder().testing().build();
    let code = cli::run();
    let stats = dhat::HeapStats::get();
    graphviz_pure_core::diag::fprintf_stderr(&format!(
        "{{\"heap_blocks\":{},\"heap_bytes\":{},\"heap_peak_bytes\":{}}}\n",
        stats.total_blocks, stats.total_bytes, stats.max_bytes
    ));
    code
}
