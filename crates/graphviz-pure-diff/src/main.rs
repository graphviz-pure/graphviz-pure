//! `gv-diff`: score the port against C Graphviz golden layouts and guard against regressions.
//!
//! ```text
//! gv-diff [--golden DIR] [--baseline FILE] [--update] [--filter SUBSTR] [--worst N] CORPUS_DIR...
//! ```
//!
//! Exits non-zero when the total score or the number of exactly matching files drops
//! below the baseline. `--update` rewrites the baseline with the current results.
//!
//! The committed baselines are `tests/golden/*.portable.json`: every build uses the same (portable)
//! math functions, so one set serves every platform.
//!
//! ```text
//! gv-diff render [--root DIR] [--baseline FILE] [--update] [--filter SUBSTR]
//!                [--format FMT[,FMT...]] [--worst N]
//! ```
//!
//! Render fidelity: compares the port's `-n2` renderings of the C layouts in
//! `tests/golden/render/laid` with C's (see `graphviz_pure_diff::render`; the references are unpacked
//! from `tests/golden/render.tar.xz` on first use).
//!
//! ```text
//! gv-diff pairs C_DIR PORT_DIR [--worst N]
//! ```
//!
//! Compares two directories of `-Tdot` layouts by file stem (C references vs
//! port outputs produced elsewhere, e.g. by `scripts/dot-broad-validate.sh`).
//! A stem present only in C_DIR counts as a port failure.

// A developer tool, not the port: its reports go to stderr directly.
#![allow(clippy::disallowed_macros)]

use graphviz_pure_diff::{Baseline, FileReport, compare_file};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;

mod e2e;
mod structure;

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("structure") {
        return structure::main(std::env::args().skip(2).collect());
    }
    if std::env::args().nth(1).as_deref() == Some("e2e") {
        return e2e::main(std::env::args().skip(2).collect());
    }
    if std::env::args().nth(1).as_deref() == Some("render") {
        return render_main(std::env::args().skip(2).collect());
    }
    if std::env::args().nth(1).as_deref() == Some("pairs") {
        return pairs_main(std::env::args().skip(2).collect());
    }
    let mut golden = PathBuf::from("tests/golden");
    let mut baseline_path: Option<PathBuf> = None;
    let mut update = false;
    let mut filter: Option<String> = None;
    let mut worst = 15usize;
    let mut corpora: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--golden" => golden = args.next().expect("--golden DIR").into(),
            "--baseline" => {
                baseline_path = Some(args.next().expect("--baseline FILE").into());
            }
            "--update" => update = true,
            "--filter" => filter = args.next(),
            "--worst" => worst = args.next().and_then(|s| s.parse().ok()).unwrap_or(15),
            _ => corpora.push(a.into()),
        }
    }
    if corpora.is_empty() {
        corpora = vec!["tests/wild".into(), "tests/fixtures".into()];
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &corpora {
        let Ok(rd) = std::fs::read_dir(dir) else {
            eprintln!("cannot read {}", dir.display());
            return ExitCode::FAILURE;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "gv") {
                files.push(p);
            }
        }
    }
    files.sort();
    if let Some(f) = &filter {
        files.retain(|p| p.to_string_lossy().contains(f.as_str()));
    }
    // A deliberate deviation is not compared with C's layout; `gv-diff e2e` pins its output.
    let deviations = graphviz_pure_diff::deviations(Path::new("."));
    let before = files.len();
    files.retain(|p| !deviations.contains_key(&graphviz_pure_diff::input_key(p)));
    if files.len() < before {
        println!(
            "{} deliberate deviations from C skipped ({})",
            before - files.len(),
            graphviz_pure_diff::DEVIATIONS
        );
    }

    let results = run_all(&files, &golden);
    let current = Baseline { files: results };
    print_summary(&current, worst);

    let Some(bpath) = baseline_path else {
        return ExitCode::SUCCESS;
    };
    print_baseline(&bpath);
    if update {
        let json = serde_json::to_string_pretty(&current).expect("serialize baseline");
        std::fs::write(&bpath, json + "\n").expect("write baseline");
        println!("baseline updated: {}", bpath.display());
        return ExitCode::SUCCESS;
    }
    let base: Baseline = match std::fs::read_to_string(&bpath) {
        Ok(s) => serde_json::from_str(&s).expect("parse baseline"),
        Err(_) => {
            println!("no baseline at {}; run with --update", bpath.display());
            return ExitCode::FAILURE;
        }
    };
    check_against(&base, &current, filter.is_some())
}

/// Names the baseline file the results are checked against.
pub(crate) fn print_baseline(baseline: &Path) {
    println!("baseline {}", baseline.display());
}

fn pairs_main(args: Vec<String>) -> ExitCode {
    let mut dirs = Vec::new();
    let mut worst = 15usize;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--worst" {
            worst = it.next().and_then(|s| s.parse().ok()).unwrap_or(15);
        } else {
            dirs.push(PathBuf::from(a));
        }
    }
    let [cdir, pdir] = dirs.as_slice() else {
        eprintln!("usage: gv-diff pairs C_DIR PORT_DIR [--worst N]");
        return ExitCode::FAILURE;
    };
    let Ok(rd) = std::fs::read_dir(cdir) else {
        eprintln!("cannot read {}", cdir.display());
        return ExitCode::FAILURE;
    };
    let mut stems: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "dot").then(|| p.file_stem().unwrap().to_string_lossy().to_string())
        })
        .collect();
    stems.sort();
    let mut files = BTreeMap::new();
    for stem in stems {
        // Latin-1 graphs produce non-UTF-8 output on both sides.
        let read = |p: PathBuf| std::fs::read(p).map(graphviz_pure_core::charset::decode);
        let c = read(cdir.join(format!("{stem}.dot"))).unwrap_or_default();
        let p = read(pdir.join(format!("{stem}.dot")));
        let report = match (
            graphviz_pure_diff::extract(&c),
            p.map(|p| graphviz_pure_diff::extract(&p)),
        ) {
            (Err(e), _) => FileReport {
                error: Some(format!("golden: {e}")),
                ..FileReport::default()
            },
            (Ok(g), Ok(Ok(a))) => graphviz_pure_diff::compare(&g, &a),
            (Ok(g), r) => FileReport {
                nodes: g.nodes.len(),
                edges: g.edges.len(),
                clusters: g.clusters.len(),
                error: Some(match r {
                    Ok(Err(e)) => format!("port output: {e}"),
                    _ => "no port output".to_string(),
                }),
                ..FileReport::default()
            },
        };
        files.insert(stem, report);
    }
    let cur = Baseline { files };
    print_summary(&cur, worst);
    ExitCode::SUCCESS
}

fn run_all(files: &[PathBuf], golden: &Path) -> BTreeMap<String, FileReport> {
    // Silence panic messages from the port; they are captured in the report.
    std::panic::set_hook(Box::new(|_| {}));
    let out = Mutex::new(BTreeMap::new());
    let next = Mutex::new(0usize);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(path) = files.get(i) else { break };
                    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                    let gpath = golden.join(format!("{stem}.dot"));
                    let read = |p: &Path| std::fs::read(p).map(graphviz_pure_core::charset::decode);
                    let report = match (read(path), read(&gpath)) {
                        (Ok(src), Ok(gold)) => compare_file(&src, &gold),
                        _ => FileReport {
                            error: Some(format!("missing golden {}", gpath.display())),
                            ..FileReport::default()
                        },
                    };
                    out.lock().unwrap().insert(stem, report);
                }
            });
        }
    });
    let _ = std::panic::take_hook();
    out.into_inner().unwrap()
}

fn print_summary(b: &Baseline, worst: usize) {
    let n = b.files.len();
    let sum = |f: fn(&FileReport) -> usize| b.files.values().map(f).sum::<usize>();
    let (nodes, nodes_ok) = (sum(|r| r.nodes), sum(|r| r.nodes_ok));
    let (edges, edges_ok) = (sum(|r| r.edges), sum(|r| r.edges_ok));
    let (cl, cl_ok) = (sum(|r| r.clusters), sum(|r| r.clusters_ok));
    let errors = b.files.values().filter(|r| r.error.is_some()).count();
    let pct = |a: usize, t: usize| {
        if t == 0 {
            100.0
        } else {
            100.0 * a as f64 / t as f64
        }
    };
    println!("files:     {n}  (errors: {errors})");
    println!(
        "exact:     {} / {n}  ({:.1}%)",
        b.exact_count(),
        pct(b.exact_count(), n)
    );
    println!(
        "score:     {:.3} / {n}  ({:.1}%)",
        b.total_score(),
        100.0 * b.total_score() / n.max(1) as f64
    );
    println!(
        "nodes ok:  {nodes_ok} / {nodes}  ({:.1}%)",
        pct(nodes_ok, nodes)
    );
    println!(
        "edges ok:  {edges_ok} / {edges}  ({:.1}%)",
        pct(edges_ok, edges)
    );
    let sizes_ok = sum(|r| r.sizes_ok);
    println!(
        "sizes ok:  {sizes_ok} / {nodes}  ({:.1}%)",
        pct(sizes_ok, nodes)
    );
    println!("clusters:  {cl_ok} / {cl}  ({:.1}%)", pct(cl_ok, cl));
    let rank_ok = b.files.values().filter(|r| r.rank_ok).count();
    let order_ok = b.files.values().filter(|r| r.order_ok).count();
    println!("ranks ok:  {rank_ok} / {n}  ({:.1}%)", pct(rank_ok, n));
    println!("order ok:  {order_ok} / {n}  ({:.1}%)", pct(order_ok, n));
    let bb_ok = b.files.values().filter(|r| r.bb_ok).count();
    println!("bb ok:     {bb_ok} / {n}  ({:.1}%)", pct(bb_ok, n));
    let mut v: Vec<_> = b.files.iter().collect();
    v.sort_by(|a, b| a.1.score().total_cmp(&b.1.score()));
    if worst > 0 {
        println!("\nlowest scores:");
        for (name, r) in v.iter().take(worst) {
            println!(
                "  {:.3}  {name}  nodes {}/{} edges {}/{} node_err {:.1} edge_err {:.1}{}",
                r.score(),
                r.nodes_ok,
                r.nodes,
                r.edges_ok,
                r.edges,
                r.node_err,
                r.edge_err,
                r.error
                    .as_deref()
                    .map(|e| format!("  [{e}]"))
                    .unwrap_or_default()
            );
        }
    }
}

fn check_against(base: &Baseline, cur: &Baseline, filtered: bool) -> ExitCode {
    let mut regressions = Vec::new();
    let mut improvements = 0usize;
    for (name, r) in &cur.files {
        let Some(old) = base.files.get(name) else {
            continue;
        };
        let (a, b) = (old.score(), r.score());
        if b + 1e-9 < a || (old.exact() && !r.exact()) {
            regressions.push(format!("  {name}: {a:.3} -> {b:.3}"));
        } else if b > a + 1e-9 {
            improvements += 1;
        }
    }
    let old_subset = Baseline {
        files: base
            .files
            .iter()
            .filter(|(k, _)| cur.files.contains_key(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    let (os, cs) = (old_subset.total_score(), cur.total_score());
    let (oe, ce) = (old_subset.exact_count(), cur.exact_count());
    println!(
        "\nvs baseline: score {os:.3} -> {cs:.3}, exact {oe} -> {ce}, {improvements} files improved, {} regressed",
        regressions.len()
    );
    for r in regressions.iter().take(40) {
        println!("{r}");
    }
    let failed = cs + 1e-6 < os || ce < oe;
    if failed {
        println!("FAIL: aggregate fidelity dropped below the baseline");
        ExitCode::FAILURE
    } else {
        if !filtered && (cs > os + 1e-6 || ce > oe) {
            println!("note: results beat the baseline; refresh it with --update");
        }
        ExitCode::SUCCESS
    }
}

fn render_main(args: Vec<String>) -> ExitCode {
    use graphviz_pure_diff::render::{FORMATS, RenderBaseline, check, print_summary, run_corpus};
    let mut root = PathBuf::from("tests/golden/render");
    let mut baseline_path: Option<PathBuf> = None;
    let mut update = false;
    let mut filter: Option<String> = None;
    let mut formats: Vec<String> = Vec::new();
    let mut worst = 0usize;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => root = it.next().expect("--root DIR").into(),
            "--baseline" => {
                baseline_path = Some(it.next().expect("--baseline FILE").into());
            }
            "--update" => update = true,
            "--filter" => filter = it.next(),
            "--format" => formats.extend(
                it.next()
                    .expect("--format FMT")
                    .split(',')
                    .map(str::to_string),
            ),
            "--worst" => worst = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let fmts: Vec<&str> = if formats.is_empty() {
        FORMATS.to_vec()
    } else {
        formats.iter().map(String::as_str).collect()
    };
    if let Err(e) = graphviz_pure_diff::render::ensure_unpacked(&root) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let cur = run_corpus(&root, &fmts, filter.as_deref());
    let base: Option<RenderBaseline> = baseline_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| serde_json::from_str(&s).expect("parse render baseline"));
    print_summary(&cur, base.as_ref());
    if worst > 0 {
        for (fmt, files) in &cur.formats {
            let mut v: Vec<_> = files.iter().filter(|(_, r)| !r.identical).collect();
            v.sort_by_key(|(name, r)| (r.diff_lines, (*name).clone()));
            println!("\n{fmt}: closest non-identical files");
            for (name, r) in v.iter().take(worst) {
                println!(
                    "  {:>6}  {name}{}",
                    r.diff_lines,
                    r.error
                        .as_deref()
                        .map(|e| format!("  [{e}]"))
                        .unwrap_or_default()
                );
            }
        }
    }
    let Some(bpath) = baseline_path else {
        return ExitCode::SUCCESS;
    };
    print_baseline(&bpath);
    if update {
        let mut merged = base.unwrap_or_default();
        for (fmt, files) in cur.formats {
            let entry = merged.formats.entry(fmt).or_default();
            if filter.is_none() {
                entry.clear();
            }
            entry.extend(files);
        }
        let json = serde_json::to_string_pretty(&merged).expect("serialize render baseline");
        std::fs::write(&bpath, json + "\n").expect("write render baseline");
        println!("render baseline updated: {}", bpath.display());
        return ExitCode::SUCCESS;
    }
    let Some(base) = base else {
        println!(
            "no render baseline at {}; run with --update",
            bpath.display()
        );
        return ExitCode::FAILURE;
    };
    if check(&base, &cur) {
        ExitCode::SUCCESS
    } else {
        println!("FAIL: render fidelity dropped below the baseline");
        ExitCode::FAILURE
    }
}
