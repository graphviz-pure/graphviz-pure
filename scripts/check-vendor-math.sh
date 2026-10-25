#!/usr/bin/env bash
# Fails if vendored raster code calls a platform libm transcendental.
#
# The methods are the f32 and f64 entries in clippy.toml `disallowed-methods`
# (docs/porting/libm.md). `sqrt` is not among them. Exceptions live in
# scripts/vendor-math.allow: Vec2's zero-argument hypot and atan2, kurbo's powi,
# and tiny-skia's f32x8 powf approximation.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - << 'PY'
import pathlib
import re
import sys

root = pathlib.Path("vendor")
allow_path = pathlib.Path("scripts/vendor-math.allow")
methods = [
    "sin_cos", "asinh", "acosh", "atanh", "atan2", "sinh", "cosh", "tanh",
    "log10", "log2", "ln_1p", "exp_m1", "exp2", "asin", "acos", "atan",
    "hypot", "powf", "powi", "cbrt", "sin", "cos", "tan", "exp", "ln", "log",
]
call = re.compile(
    r"(?<!p_)\.(?P<name>" + "|".join(methods) + r")\s*\((?P<args>[^)]*)\)"
    r"|\b(?:f32|f64)::(?P<name2>" + "|".join(methods) + r")\s*\("
)
rules = []
for raw in allow_path.read_text().splitlines():
    line = raw.split("#", 1)[0].strip()
    if not line:
        continue
    method, path = line.split(None, 1)
    zero_arg = method.endswith("()")
    if zero_arg:
        method = method[:-2]
    rules.append((method, zero_arg, path))

hits = {rule: 0 for rule in rules}
failures = []
block = re.compile(r"/\*.*?\*/", re.S)
for path in sorted(root.rglob("*.rs")):
    text = path.read_text(encoding="utf-8", errors="replace")
    text = block.sub("", text)
    rel = path.as_posix()
    for lineno, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        for match in call.finditer(code):
            name = match.group("name") or match.group("name2")
            matched = False
            zero = match.group("name") is not None and (match.group("args") or "").strip() == ""
            for rule in rules:
                method, zero_arg, rule_path = rule
                if method != name or zero_arg != zero:
                    continue
                if rule_path == "*" or rel == rule_path:
                    hits[rule] += 1
                    matched = True
                    break
            if not matched:
                failures.append(f"{rel}:{lineno}: {line.strip()}")

missing = [
    f"{method}{'()' if zero else ''} {path}"
    for (method, zero, path), count in hits.items()
    if count == 0 and path != "*"
]
if failures or missing:
    if failures:
        print("vendored raster code calls a platform transcendental:", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
    if missing:
        print("allow-list rules matched nothing:", file=sys.stderr)
        print("\n".join(missing), file=sys.stderr)
    sys.exit(1)
print(f"vendor math check ok ({sum(hits.values())} allowed calls)")
PY
