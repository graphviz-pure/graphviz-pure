#!/usr/bin/env python3
"""Collects the changelog entries of the pull requests merged in a range of master.

Every pull request carries its entry in its body, under a `## Changelog entry` heading
(.github/pull_request_template.md), not in CHANGELOG.md. This reads the pull request number
(the last `(#N)` of each squash-merged commit's subject) for every commit in BASE..HEAD, fetches
the bodies and labels with `gh` (reads only), and sorts each entry's bullets into the CHANGELOG's sections.

An entry's bullets go under the `### Section` heading they follow in the body (Breaking, Added,
Changed, Fixed, Deprecated, Removed, Security) or the section their own bold prefix names
(`**Changed (breaking):** ...` is Breaking). Other bullets go under Breaking if the pull request
has the `breaking` label and no bullet of its own is marked Breaking, else Changed if they start
with "Output change", Added if they start with "Added", and Fixed otherwise. A bullet that names
no `#N` gets the pull request's number. An entry with no bullets that says it is not
user-visible ("Not user-visible", "None", "n/a") counts as an entry with nothing to list; an
absent or empty section is missing. A Breaking bullet in a pull request without the `breaking`
label is reported as `unlabelled_breaking` (the label decides the version), and a labelled pull
request whose entry has no Breaking bullet as `breaking_without_entry`. Nested bullets stay with
their parent; a ```markdown fence around the entry is unwrapped, other fenced code is left out.
Footer boundaries end the section; recognized agent attributions are omitted. Collection
fails on genuine prose beside bullets, keeping that prose in the JSON and error diagnostic.

    release-changelog.py collect BASE HEAD [--exclude N]...     JSON on stdout; fails on prose beside bullets
    release-changelog.py render JSON VERSION [--intro TEXT]     the CHANGELOG section
                                                         (no --intro: a line counted from the entries)
    release-changelog.py next-version JSON PREVIOUS             the version the labels ask for
    release-changelog.py version-at-least SELECTED MINIMUM      exits 0 if SELECTED >= MINIMUM

The version rule (docs/versioning.md): while the version is 0.x, a pull request labelled
`breaking` bumps the minor version and everything else the patch version; from 1.0 `breaking`
bumps the major version. After a pre-release tag (v0.2.0-rc.1) the next version is the version
it pre-released (0.2.0). Python standard library only (3.9).
"""
import datetime
import json
import re
import subprocess
import sys
import textwrap

REPO = "graphviz-pure/graphviz-pure"
BREAKING_LABEL = "breaking"
# The CHANGELOG's sections, in the order they are written.
SECTIONS = ["Breaking", "Added", "Changed", "Fixed", "Deprecated", "Removed", "Security"]
# An entry that says there is nothing to list: "None.", "None (test only).", "N/A", "Not
# user-visible (CI only)." Prose that only starts with such a word ("None of the ...") is an entry.
NOT_VISIBLE = re.compile(r"^(none|n/?a|not user[- ]visible|no user[- ]visible changes?)\s*([.:;(—–-]|$)", re.I)
WIDTH = 96


def commits(base, head):
    out = subprocess.check_output(["git", "log", "--first-parent", "--format=%H%x00%s", f"{base}..{head}"], text=True)
    result = []
    for line in out.splitlines():
        sha, subject = line.split("\0", 1)
        m = re.search(r"\(#(\d+)\)\s*$", subject)
        result.append({"sha": sha, "subject": subject, "pr": int(m.group(1)) if m else None})
    return result


def query_prs(numbers):
    """The `repository` object of one GraphQL query for these pull requests. A number that is
    not a pull request (an issue) comes back null, with an error, and gh exits non-zero, but the
    other numbers' data is there."""
    owner, name = REPO.split("/")
    fields = " ".join(
        f"p{n}: pullRequest(number: {n}) {{ number title url body labels(first: 30) {{ nodes {{ name }} }} }}"
        for n in numbers)
    query = f'query {{ repository(owner: "{owner}", name: "{name}") {{ {fields} }} }}'
    run = subprocess.run(["gh", "api", "graphql", "-f", f"query={query}"], capture_output=True, text=True)
    try:
        return (json.loads(run.stdout).get("data") or {}).get("repository") or {}
    except ValueError:
        return None


def fetch_prs(numbers):
    """Body, title and labels of each pull request, through GraphQL, 25 at a time. A batch that
    fails as a whole is asked again one number at a time; numbers that still fail are left out
    (and reported as commits without a pull request)."""
    prs = {}
    numbers = sorted(set(numbers))
    for i in range(0, len(numbers), 25):
        chunk = numbers[i:i + 25]
        repo = query_prs(chunk)
        if repo is None:
            repo = {}
            for n in chunk:
                repo.update(query_prs([n]) or {})
        for n in chunk:
            pr = repo.get(f"p{n}")
            if pr:
                prs[n] = {"title": pr["title"], "url": pr["url"], "body": pr["body"] or "",
                          "labels": [label["name"] for label in pr["labels"]["nodes"]]}
    return prs


def section(body):
    """Extract entry text, stopping before footer boundaries outside fenced code.

    Leading template comments are ignored; a comment after entry content ends it.
    Quoted/inline comment markers and fenced examples are retained (#479).
    """
    out, inside, fence, comment = [], False, False, False
    for line in body.replace("\r\n", "\n").split("\n"):
        stripped = line.strip()
        if comment:
            comment = "-->" not in line
            continue
        if not fence and stripped.startswith("<!--"):
            if inside and any(part.strip() for part in out):
                break
            comment = "-->" not in line
            continue
        if not fence and re.match(r"^##\s+Changelog entry\s*$", line, re.I):
            inside = True
            continue
        if inside and not fence:
            if (re.match(r"^#{1,2}\s", line) or stripped == "---"
                    or re.match(r"^</?[A-Za-z][\w:-]*(?:\s|/?>)|^<!", stripped)):
                break
            if (re.fullmatch(r"Generated by .+ via T3 Code", stripped)
                    or stripped.startswith("🤖 Generated with")):
                continue
        if inside:
            out.append(line)
        if line.lstrip().startswith("```"):
            fence = not fence
    return "\n".join(out) if inside else None


def parse(text, default):
    """The entry's bullets as (section, text) pairs, and its prose paragraphs."""
    bullets, prose, current, heading, code = [], [], None, None, None
    for raw in text.split("\n"):
        line = raw.rstrip()
        # A ```markdown fence around the entry is unwrapped; any other fenced block (a code
        # example) is left out and reported with the prose.
        if line.strip().startswith("```"):
            if code is None:
                info = line.strip()[3:].strip().lower()
                code = "markdown" if info in ("", "markdown", "md") and current is None else "code"
                if code == "code":
                    prose.append("(a fenced code block, not copied)")
                    prose.append(None)
            else:
                code = None
            continue
        if code == "code":
            continue
        # `### Fixed`, or a line of its own such as `Under **Fixed**:` or `**Changed**:`.
        # `Under **Changed** (breaking):` is a Breaking heading.
        m = re.match(r"^(?:#{3,}\s+(\w+)(.*)|(?:Under\s+)?\*\*(\w+):?\*\*([^*]*))$", line.strip())
        if m and (m.group(1) or m.group(3)).capitalize() in SECTIONS:
            heading = (m.group(1) or m.group(3)).capitalize()
            if "breaking" in (m.group(2) or m.group(4) or "").lower():
                heading = "Breaking"
            current = None
            continue
        m = re.match(r"^(\s*)(?:[-*]|\d+[.)])\s+(.*)$", line)
        if m and len(m.group(1).expandtabs(4)) >= 2 and current is not None:
            # A nested bullet stays with its parent, one per line.
            current[1] += "\n- " + m.group(2).strip()
        elif m:
            text, section = m.group(2).strip(), heading
            # A bullet that names its section: `**Changed (breaking):** ...`, `**Breaking:** ...`,
            # `Added: ...`, `Fixed: ...`.
            p = re.match(r"^\*\*(\w+)([^*]*?):?\*\*:?\s+(.*)$", text) or re.match(r"^(\w+)( \(breaking\))?:\s+(.*)$", text)
            if p and p.group(1).capitalize() in SECTIONS:
                section = "Breaking" if "breaking" in (p.group(1) + (p.group(2) or "")).lower() else p.group(1).capitalize()
                text = p.group(3)[:1].upper() + p.group(3)[1:]
            current = [section, text]
            bullets.append(current)
        elif line.strip() and current is not None and (raw.startswith(" ") or raw.startswith("\t")):
            current[1] += " " + line.strip()
        elif line.strip():
            current = None
            if prose and prose[-1] is not None:
                prose[-1] += " " + line.strip()
            else:
                prose.append(line.strip())
        else:
            current = None
            if prose and prose[-1] is not None:
                prose.append(None)
    prose = [p for p in prose if p]
    result = []
    for heading, body in bullets:
        if heading is None:
            if default:
                heading = default
            elif body.lower().startswith("output change"):
                heading = "Changed"
            else:
                heading = "Added" if body.startswith("Added ") else "Fixed"
        result.append((heading, body))
    return result, prose


def collect(base, head, exclude):
    entries, missing, none, notes, without_pr = [], [], [], [], []
    rows = commits(base, head)
    prs = fetch_prs([c["pr"] for c in rows if c["pr"] and c["pr"] not in exclude])
    breaking, unlabelled, no_breaking_entry = [], [], []
    for c in reversed(rows):  # oldest first, as the changes landed
        n = c["pr"]
        if n in exclude:
            continue
        if n is None or n not in prs:
            without_pr.append({"sha": c["sha"], "subject": c["subject"],
                               "reason": "no (#N) in the subject" if n is None else f"#{n} is not a pull request"})
            continue
        pr = prs[n]
        is_breaking = BREAKING_LABEL in pr["labels"]
        if is_breaking:
            breaking.append(n)
        text = section(pr["body"])
        info = {"pr": n, "title": pr["title"], "url": pr["url"]}
        if text is None or not text.strip():
            missing.append(info)
            continue
        bullets, prose = parse(text, None)
        if is_breaking and not any(h == "Breaking" for h, _ in bullets):
            bullets, prose = parse(text, "Breaking")
        if not bullets:
            if prose and NOT_VISIBLE.match(prose[0]):
                none.append(dict(info, reason=prose[0]))
                continue
            if not prose:
                missing.append(info)
                continue
            # A paragraph with no bullet is the entry itself.
            bullets = [("Breaking" if is_breaking else "Fixed", " ".join(prose))]
            prose = []
        if not is_breaking and any(h == "Breaking" for h, _ in bullets):
            unlabelled.append(info)
        if is_breaking and not any(h == "Breaking" for h, _ in bullets):
            no_breaking_entry.append(info)
        for heading, body in bullets:
            first, _, rest = body.partition("\n")
            if not re.search(r"#\d+", body):
                first = re.sub(r"\.?$", f" (#{n}).", first, count=1)
            body = first + ("\n" + rest if rest else "")
            entries.append({"pr": n, "section": heading, "text": body})
        if prose:
            notes.append(dict(info, notes=prose))
    return {"base": base, "head": head, "entries": entries, "missing": missing, "none": none,
            "notes": notes, "breaking": sorted(breaking), "unlabelled_breaking": unlabelled,
            "breaking_without_entry": no_breaking_entry,
            "without_pr": without_pr}


def default_intro(data):
    """A one-line intro counted from the entries: "3 changes: 1 added, 2 fixed." """
    counts = {}
    for entry in data["entries"]:
        counts[entry["section"]] = counts.get(entry["section"], 0) + 1
    total = sum(counts.values())
    if total == 0:
        return "No user-visible changes."
    parts = [f"{counts[name]} {name.lower()}" for name in SECTIONS if counts.get(name)]
    noun = "change" if total == 1 else "changes"
    return f"{total} {noun}: {', '.join(parts)}."


def render(data, version, intro):
    out = [f"## [{version}] - {datetime.date.today().isoformat()}", "", intro or default_intro(data)]
    for name in SECTIONS:
        items = [e["text"] for e in data["entries"] if e["section"] == name]
        if not items:
            continue
        out += ["", f"### {name}", ""]
        for text in items:
            top, *nested = text.split("\n- ")
            out += textwrap.wrap(top, WIDTH, initial_indent="- ", subsequent_indent="  ",
                                 break_long_words=False, break_on_hyphens=False)
            for sub in nested:
                out += textwrap.wrap(sub, WIDTH, initial_indent="  - ", subsequent_indent="    ",
                                     break_long_words=False, break_on_hyphens=False)
    return "\n".join(out) + "\n"


def next_version(data, previous):
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)(-.*)?$", previous)
    major, minor, patch = (int(x) for x in m.groups()[:3])
    if m.group(4):
        # After a pre-release (0.2.0-rc.1), the next version is the one it pre-releases.
        return f"{major}.{minor}.{patch}"
    if not data["breaking"]:
        return f"{major}.{minor}.{patch + 1}"
    return f"0.{minor + 1}.0" if major == 0 else f"{major + 1}.0.0"


def version_at_least(selected, minimum):
    """Compare SemVer precedence, including numeric versus alphanumeric prerelease IDs."""
    def key(version):
        core, separator, prerelease = version.partition("-")
        identifiers = tuple((0, int(part)) if part.isdigit() else (1, part)
                            for part in prerelease.split(".")) if separator else ()
        return tuple(map(int, core.split("."))), not separator, identifiers
    return key(selected) >= key(minimum)


def main(argv):
    if len(argv) >= 3 and argv[0] == "collect":
        exclude = {int(argv[i + 1]) for i, a in enumerate(argv) if a == "--exclude"}
        data = collect(argv[1], argv[2], exclude)
        json.dump(data, sys.stdout, indent=1)
        print()
        if data["notes"]:
            print("Prose beside the bullets, not copied (check it):", file=sys.stderr)
            for note in data["notes"]:
                print(f"- #{note['pr']}: " + " / ".join(note["notes"]), file=sys.stderr)
            sys.exit(1)
    elif len(argv) >= 3 and argv[0] == "render":
        intro = argv[argv.index("--intro") + 1] if "--intro" in argv else None
        sys.stdout.write(render(json.load(open(argv[1])), argv[2], intro))
    elif len(argv) == 3 and argv[0] == "next-version":
        print(next_version(json.load(open(argv[1])), argv[2]))
    elif len(argv) == 3 and argv[0] == "version-at-least":
        sys.exit(0 if version_at_least(argv[1], argv[2]) else 1)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
