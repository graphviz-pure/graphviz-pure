//! Full-pipeline ratchet: every input whose `-Tsvg` matched C byte for byte in
//! `tests/golden/e2e-baseline.libm-matched.portable.json` must still match. The reference is C
//! computing with the port's own math functions (`tests/golden/e2e.libm-matched.json`, made with
//! `scripts/dot-portable-libm.sh`), so the math library is out of the equation. The comparison
//! with stock C on macOS (`gv-diff e2e --stock`) is a report and is not checked here.
//! An input listed in `tests/golden/deviations.tsv`, where the port deliberately differs from C,
//! is checked against the port's pinned output instead of C's.
//!
//! Refresh the baseline after an improvement with
//! `cargo run --release -p graphviz-pure-diff -- e2e --update` (or run
//! `scripts/update-baselines.sh`, which refreshes every baseline).

use std::process::Command;

#[test]
fn e2e_svg_identity_does_not_regress() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let status = Command::new(env!("CARGO_BIN_EXE_gv-diff"))
        .current_dir(root)
        .arg("e2e")
        .status()
        .expect("run gv-diff e2e");
    assert!(
        status.success(),
        "e2e SVG identity regressed; see gv-diff output above"
    );
}

/// Every deliberate deviation names a corpus input that exists, once (`deviations` panics on a
/// malformed or repeated line), with a 16-digit hash, and is in the e2e baseline, so that
/// `gv-diff e2e` keeps checking it against its pinned output and a stale line cannot outlive
/// its input.
#[test]
fn deviations_name_existing_inputs_in_the_baseline() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let deviations = graphviz_pure_diff::deviations(root);
    assert!(!deviations.is_empty());
    let baseline =
        std::fs::read_to_string(root.join("tests/golden/e2e-baseline.libm-matched.portable.json"))
            .expect("read the e2e baseline");
    for (path, d) in &deviations {
        assert!(root.join(path).is_file(), "{path}: no such input");
        assert_eq!(d.svg_hash.len(), 16, "{path}");
        assert!(d.svg_hash.bytes().all(|b| b.is_ascii_hexdigit()), "{path}");
        // `gv-diff e2e`'s key: the corpus (tests/wild and tests/fixtures are "main"), the stem.
        let (dir, file) = path
            .strip_prefix("tests/")
            .and_then(|p| p.split_once('/'))
            .expect("tests/<corpus>/<input>");
        let corpus = if matches!(dir, "wild" | "fixtures") {
            "main"
        } else {
            dir
        };
        let stem = file.strip_suffix(".gv").expect(".gv input");
        assert!(
            baseline.contains(&format!("\"{corpus}/{stem}\"")),
            "{path}: not in the e2e baseline"
        );
    }
}

/// Issue #15 (fuzz case 006-fdp-cluster-port-sincos-fusion, now
/// `tests/fdp/fuzz_cluster_port_sincos.gv`). fdp places a cluster's boundary ports with
/// `T_Wd * cos(alpha)` and `T_Ht * sin(alpha)` (`lib/fdpgen/tlayout.c:501-502`), and C calls
/// `sin` and `cos` separately there. LLVM merged the two calls in the port into
/// `__sincos_stret`, whose sine differs from Apple's `sin()` in the last bit for some
/// arguments; the spring embedder amplified that into clusters and edges a tenth of a point
/// off. Checked against C with the port's portable functions (`e2e.libm-matched.json`).
#[test]
fn fdp_cluster_ports_call_sin_and_cos_separately() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let args = ["e2e", "--filter", "fdp/fuzz_cluster_port_sincos", "--list"];
    let out = Command::new(env!("CARGO_BIN_EXE_gv-diff"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("run gv-diff e2e");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let total = stdout
        .lines()
        .find(|l| l.starts_with("total"))
        .expect("gv-diff e2e prints a total line");
    let counts: Vec<&str> = total.split_whitespace().skip(1).take(2).collect();
    assert_eq!(counts, ["1", "1"], "port -Tsvg differs from C:\n{stdout}");
}
