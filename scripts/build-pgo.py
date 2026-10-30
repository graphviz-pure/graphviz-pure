#!/usr/bin/env python3
"""Build a native release CLI with PGO, using a fixed training list and fresh profiles.

Rust's llvm-tools component must be installed. TARGET must run on this machine.
Only build flags change; cargo install and the workspace's release profile stay plain.
"""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
TRAINING = ROOT / "scripts/bench/sets/pgo.tsv"


def check_profile(path, profdata):
    # Normal profiles are several MiB and billions of counted block executions.
    # These deliberately low floors reject empty/trivial training without tying
    # the release to a particular compiler's function count or profile encoding.
    if path.stat().st_size < 64 * 1024:
        raise RuntimeError("merged PGO profile is smaller than 64 KiB")
    summary = subprocess.check_output([str(profdata), "show", str(path)], text=True)
    counts = {}
    for field in ["Total functions", "Maximum function count", "Total count"]:
        match = re.search(rf"^{field}:\s*(\d+)", summary, re.MULTILINE)
        if not match:
            raise RuntimeError(f"missing {field} in llvm-profdata summary")
        counts[field] = int(match[1])
    if (counts["Total functions"] < 1000 or counts["Maximum function count"] < 10000
            or counts["Total count"] < 1000000):
        raise RuntimeError(f"merged PGO profile contains too little training: {counts}")
    return counts


def train(binary, profiles, synth):
    cases = []
    for line in TRAINING.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            source, fmt = line.split()
            path = synth / source[6:] if source.startswith("synth/") else pathlib.Path(source)
            if not path.is_file():
                raise FileNotFoundError(path)
            cases.append((path, fmt))
    if not cases:
        raise ValueError("the PGO training list is empty")
    for index, (path, fmt) in enumerate(cases):
        # A fixed filename per invocation avoids PID-dependent merge order and runtime pooling.
        env = dict(os.environ, LLVM_PROFILE_FILE=str(profiles / f"{index:04d}.profraw"))
        result = subprocess.run([str(binary), f"-T{fmt}", str(path)], cwd=ROOT, env=env,
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=180)
        if result.returncode:
            raise RuntimeError(f"training failed: {path} -T{fmt}: "
                               f"exit {result.returncode}: {result.stderr.decode(errors='replace')}")
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--target-dir", default="target")
    parser.add_argument("--profiles", required=True, help="a new directory, outside the source tree")
    parser.add_argument("--cargo-wrapper", help="local build-slot wrapper (CI invokes cargo directly)")
    args = parser.parse_args()
    os.chdir(ROOT)
    target_dir = pathlib.Path(args.target_dir).resolve()
    profiles = pathlib.Path(args.profiles).resolve()
    # Never reuse counts from an earlier commit/build. Refuse rather than deleting user data.
    profiles.mkdir(parents=True, exist_ok=False)
    raw = profiles / "raw"
    raw.mkdir()
    synth = profiles / "synth"
    subprocess.run([sys.executable, "-I", "scripts/bench/synth.py", "--out", str(synth)],
                   check=True, stdout=subprocess.DEVNULL)
    rustc = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.split(": ", 1)[1] for line in rustc.splitlines() if line.startswith("host: "))
    sysroot = pathlib.Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    suffix = ".exe" if os.name == "nt" else ""
    profdata = sysroot / "lib/rustlib" / host / "bin" / f"llvm-profdata{suffix}"
    if not profdata.is_file():
        parser.error("install rustup's llvm-tools component for this toolchain")
    # Encoded flags preserve paths with spaces on every platform, including Windows.
    encoded = os.environ.get("CARGO_ENCODED_RUSTFLAGS")
    flags = shlex.split(os.environ.get("RUSTFLAGS", ""))
    if encoded is not None:
        flags = encoded.split("\x1f") if encoded else []
    cargo = ([args.cargo_wrapper] if args.cargo_wrapper else []) + ["cargo"]
    build_env = dict(os.environ)
    if args.target == "aarch64-pc-windows-msvc":
        # link.exe pads the one-byte LLVM profile section markers on ARM64, producing
        # malformed raw profiles (empty symbol names). The bundled lld-link preserves them.
        linker = sysroot / "lib/rustlib" / host / "bin/rust-lld.exe"
        if not linker.is_file():
            parser.error("the Windows ARM64 PGO build needs rustc's bundled rust-lld")
        build_env["CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER"] = str(linker)

    def build(pgo_flag):
        phase_flags = [*flags, pgo_flag]
        if pgo_flag.startswith("-Cprofile-use="):
            phase_flags.append("-Cllvm-args=-pgo-warn-missing-function")
        env = dict(build_env, CARGO_TARGET_DIR=str(target_dir),
                   CARGO_ENCODED_RUSTFLAGS="\x1f".join(phase_flags))
        env.pop("RUSTFLAGS", None)
        subprocess.run([*cargo, "build", "--release", "--locked", "--target", args.target,
                        "-p", "graphviz-pure-cli", "--bin", "graphviz-pure"], env=env, check=True)

    start = time.monotonic()
    build(f"-Cprofile-generate={raw}")
    generate_seconds = time.monotonic() - start
    binary = target_dir / args.target / "release" / f"graphviz-pure{suffix}"
    train_start = time.monotonic()
    count = train(binary, raw, synth)
    training_seconds = time.monotonic() - train_start
    print(f"PGO: {count} training commands in {training_seconds:.1f}s", flush=True)
    merged = profiles / "merged.profdata"
    raw_files = [raw / f"{index:04d}.profraw" for index in range(count)]
    if not all(path.is_file() for path in raw_files):
        raise RuntimeError("training did not write a profile for every command")
    merge_start = time.monotonic()
    subprocess.run([str(profdata), "merge", "-o", str(merged), *map(str, raw_files)], check=True)
    profile_counts = check_profile(merged, profdata)
    merge_seconds = time.monotonic() - merge_start
    # Diagnose a profile whose function names/hashes barely match the final build.
    use_start = time.monotonic()
    build(f"-Cprofile-use={merged}")
    use_seconds = time.monotonic() - use_start
    info = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "target": args.target,
        "rustc": rustc,
        "llvm_profdata": subprocess.check_output([str(profdata), "--version"], text=True),
        "base_rustflags": flags,
        "linker": build_env.get(f"CARGO_TARGET_{args.target.upper().replace('-', '_')}_LINKER",
                                "target default"),
        "training_commands": count,
        "training_sha256": hashlib.sha256(TRAINING.read_bytes()).hexdigest(),
        "profile_sha256": hashlib.sha256(merged.read_bytes()).hexdigest(),
        "profile_counts": profile_counts,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "generate_build_seconds": generate_seconds,
        "training_seconds": training_seconds,
        "merge_seconds": merge_seconds,
        "use_build_seconds": use_seconds,
        "elapsed_seconds": time.monotonic() - start,
    }
    (profiles / "build-info.json").write_text(json.dumps(info, indent=2) + "\n")
    print(f"PGO: built {binary} in {time.monotonic() - start:.1f}s", flush=True)


if __name__ == "__main__":
    main()
