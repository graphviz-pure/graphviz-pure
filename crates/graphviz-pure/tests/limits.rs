//! The limits and cancellation API (`Limits::run`): each limit stops a call with its error,
//! and a call that is not stopped returns the bytes an unlimited call returns.

use graphviz_pure::{
    CancelToken, GlobalAttrs, GraphvizErrorKind, LayoutEngine, LayoutOptions, Limit, Limits,
    OutputFormat,
};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn generous_limits_return_the_unlimited_bytes() {
    let src = fixture("unix.gv");
    let unlimited = graphviz_pure::render_dot(&src, "dot", "svg").unwrap();
    let limits = Limits::new()
        .max_input_bytes(1 << 20)
        .max_nodes(1000)
        .max_edges(1000)
        .max_steps(u64::MAX / 2)
        .max_allocation(1 << 30)
        .deadline(std::time::Instant::now() + std::time::Duration::from_secs(3600))
        .cancel_token(CancelToken::new())
        .cancel_when(|| false);
    let limited = limits.run(|| graphviz_pure::render_dot(&src, "dot", "svg").unwrap());
    assert_eq!(limited.output(), unlimited.output());
    assert_eq!(limited.diagnostics(), unlimited.diagnostics());
}

#[test]
fn input_and_graph_size_limits() {
    let src = "digraph { a -> b; b -> c; c -> a }";
    let err = Limits::new()
        .max_input_bytes(10)
        .run(|| graphviz_pure::render_dot(src, "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.kind(), GraphvizErrorKind::LimitExceeded);
    assert_eq!(err.limit(), Some(Limit::InputBytes));
    assert_eq!(
        err.to_string(),
        "Limit exceeded: the input has 34 bytes, over the limit of 10"
    );

    let err = Limits::new()
        .max_nodes(2)
        .run(|| graphviz_pure::parse_dot(src))
        .err()
        .unwrap();
    assert_eq!(err.limit(), Some(Limit::Nodes));
    assert_eq!(err.message(), "the graph has more than 2 nodes");

    let err = Limits::new()
        .max_edges(2)
        .run(|| graphviz_pure::render_dot(src, "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.limit(), Some(Limit::Edges));

    // At the limit is allowed.
    assert!(
        Limits::new()
            .max_nodes(3)
            .max_edges(3)
            .run(|| graphviz_pure::render_dot(src, "dot", "svg"))
            .is_ok()
    );
}

#[test]
fn a_size_limit_fails_only_its_own_call() {
    let limits = Limits::new().max_nodes(2);
    limits.run(|| {
        assert!(graphviz_pure::render_dot("digraph { a; b; c }", "dot", "svg").is_err());
        assert!(graphviz_pure::render_dot("digraph { a; b }", "dot", "svg").is_ok());
    });
}

#[test]
fn the_work_budget_stops_network_simplex() {
    let src = fixture("unix.gv");
    let mut graph = graphviz_pure::parse_dot(&src).unwrap().into_output();
    let err = Limits::new()
        .max_steps(0)
        .run(|| graphviz_pure::layout_graph(&mut graph, LayoutEngine::Dot))
        .unwrap_err();
    assert_eq!(err.kind(), GraphvizErrorKind::LimitExceeded);
    assert_eq!(err.limit(), Some(Limit::Steps));
    assert_eq!(err.to_string(), "Limit exceeded: the work budget ran out");
}

#[test]
fn the_work_budget_is_deterministic() {
    let src = fixture("unix.gv");
    let unlimited = graphviz_pure::render_dot(&src, "dot", "svg").unwrap();
    // The smallest budget that finishes, found twice, is the same; one less stops.
    let finishes = |steps: u64| {
        Limits::new()
            .max_steps(steps)
            .run(|| graphviz_pure::render_dot(&src, "dot", "svg"))
    };
    let mut lo = 0u64;
    let mut hi = 1u64 << 40;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if finishes(mid).is_ok() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    assert!(lo > 0);
    assert_eq!(finishes(lo).unwrap().output(), unlimited.output());
    for _ in 0..3 {
        assert_eq!(
            finishes(lo - 1).unwrap_err().limit(),
            Some(Limit::Steps),
            "budget {}",
            lo - 1
        );
        assert!(finishes(lo).is_ok());
    }
}

#[test]
fn a_stopped_run_fails_every_later_call() {
    let src = fixture("unix.gv");
    Limits::new().max_steps(0).run(|| {
        assert!(graphviz_pure::render_dot(&src, "dot", "svg").is_err());
        let err = graphviz_pure::render_dot("digraph { a }", "dot", "svg").unwrap_err();
        assert_eq!(err.limit(), Some(Limit::Steps));
    });
    // The run is over.
    assert!(graphviz_pure::render_dot(&src, "dot", "svg").is_ok());
}

#[test]
fn a_cancelled_token_cancels() {
    let token = CancelToken::new();
    token.cancel();
    let err = Limits::new()
        .cancel_token(token)
        .run(|| graphviz_pure::render_dot("digraph { a -> b }", "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.kind(), GraphvizErrorKind::Cancelled);
    assert_eq!(err.to_string(), "Cancelled: the call was cancelled");
}

#[test]
fn a_token_cancelled_on_another_thread() {
    let token = CancelToken::new();
    let remote = token.clone();
    std::thread::spawn(move || remote.cancel()).join().unwrap();
    let err = Limits::new()
        .cancel_token(token)
        .run(|| graphviz_pure::render_dot("digraph { a -> b }", "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.kind(), GraphvizErrorKind::Cancelled);
}

#[test]
fn partial_renders_return_the_graphs_before_the_limit() {
    let src = "digraph one { a -> b }\ndigraph two { a -> b -> c -> d }\n";
    let (outs, err) = Limits::new().max_nodes(3).run(|| {
        graphviz_pure::render_dot_all_partial(
            src,
            "dot",
            "svg",
            &GlobalAttrs::default(),
            LayoutOptions::default(),
        )
    });
    assert_eq!(outs.output().len(), 1);
    assert!(outs.output()[0].contains("<title>one</title>"));
    assert_eq!(err.unwrap().limit(), Some(Limit::Nodes));
}

#[test]
fn rendering_a_laid_out_graph_respects_a_stopped_run() {
    let mut parsed = graphviz_pure::parse_dot("digraph { a -> b }").unwrap();
    let _ = graphviz_pure::layout_graph(parsed.output_mut(), LayoutEngine::Dot).unwrap();
    let token = CancelToken::new();
    token.cancel();
    let err = Limits::new()
        .cancel_token(token)
        .run(|| graphviz_pure::render_graph(parsed.output(), OutputFormat::Svg))
        .unwrap_err();
    assert_eq!(err.kind(), GraphvizErrorKind::Cancelled);
}

/// Stops `engine` at a range of work budgets on every fixture: each call returns the
/// unlimited bytes or the work budget's error, and never panics.
fn every_budget_ends_cleanly(engine: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".gv"))
        .collect();
    names.sort();
    for name in names {
        let src = fixture(&name);
        let Ok(unlimited) = graphviz_pure::render_dot(&src, engine, "svg") else {
            continue;
        };
        let mut steps = 0u64;
        loop {
            let limited = Limits::new()
                .max_steps(steps)
                .run(|| graphviz_pure::render_dot(&src, engine, "svg"));
            match limited {
                Ok(out) => {
                    assert_eq!(out.output(), unlimited.output(), "{name} at {steps} steps");
                    break;
                }
                Err(e) => assert_eq!(e.limit(), Some(Limit::Steps), "{name} at {steps}: {e}"),
            }
            steps += 1 + steps / 8;
        }
    }
}

#[test]
fn every_dot_budget_ends_cleanly() {
    every_budget_ends_cleanly("dot");
}

#[test]
fn a_cancel_when_closure_may_call_the_library() {
    let out = Limits::new()
        .cancel_when(|| {
            let _ = graphviz_pure::render_dot("digraph { a }", "dot", "svg").unwrap();
            false
        })
        .run(|| graphviz_pure::render_dot("digraph { a -> b }", "dot", "svg"));
    assert!(out.is_ok());
}

#[test]
fn the_edge_limit_stops_inside_one_statement() {
    // One statement asks for 3,000 x 3,000 edges.
    let list = |p: &str| {
        (0..3000)
            .map(|i| format!("{p}{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let src = format!("digraph {{ {{{}}} -> {{{}}} }}", list("a"), list("b"));
    let started = std::time::Instant::now();
    let err = Limits::new()
        .max_edges(1000)
        .run(|| graphviz_pure::parse_dot(&src))
        .err()
        .unwrap();
    assert_eq!(err.limit(), Some(Limit::Edges));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn a_limit_reached_while_parsing_is_seen_when_the_next_phase_starts() {
    // The heap probe runs when the run starts, when the call starts, and when the layout
    // starts (the layout crate's `a_stopped_run_does_not_lay_out` pins that check); the
    // third reading is over the limit.
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&reads);
    let err = Limits::new()
        .heap_limit(1000, move || {
            if seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2 {
                2000
            } else {
                0
            }
        })
        .run(|| graphviz_pure::render_dot("digraph { a -> b }", "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.limit(), Some(Limit::Heap));
    assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 3);
}

#[test]
fn a_long_minlen_chain_stops_while_it_is_built() {
    // 20 million virtual nodes in C and on master; the budget stops dot's class2 early.
    let src = "digraph { a -> b [minlen=20000000] }";
    let started = std::time::Instant::now();
    let err = Limits::new()
        .max_steps(100_000)
        .run(|| graphviz_pure::render_dot(src, "dot", "svg"))
        .unwrap_err();
    assert_eq!(err.limit(), Some(Limit::Steps));
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}
