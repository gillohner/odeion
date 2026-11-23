#!/usr/bin/env python3
"""Move the unit tests of crates/pm-api/src/lib.rs into one file per area under src/tests/.

lib.rs kept every test of the API in one `mod tests` (over 20,000 lines), and every branch that
added a test met every other at the same tail. This moves each test named in lib-tests.map to
`src/tests/<area>.rs` and declares that file inside `mod tests` (`mod <area>;`). The helpers the
tests share stay in `mod tests`; each file starts with `use super::*;`, so no visibility changes.

It works on the text, with a small Rust lexer for where items begin and end, and writes nothing
unless every test is still there exactly once afterwards.

    python3 -I tools/refactor/lib-tests-split.py [--base <lib.rs>] [--check] [--no-fmt]

Run again on its own result it moves nothing. A test of lib.rs that the map does not name stays
where it is and is listed; so does one that names its own place (`super::`, `tests::`,
`module_path!`, `file!`, `include_str!`, `include_bytes!`, `#[path]`).

--base <lib.rs>: for a branch made before the split, whose lib.rs still holds the tests that
are in the area files by now. The steps (SPLIT is the commit of the split):
  1. `git rebase SPLIT^ <branch>`, so that the branch starts exactly where the split did;
  2. `git show SPLIT^:crates/pm-api/src/lib.rs > /tmp/base.rs`;
  3. check out SPLIT, bring the branch's changes over (`git diff --binary SPLIT^ <branch> |
     git apply --3way`), then put the branch's whole lib.rs in place
     (`git show <branch>:crates/pm-api/src/lib.rs > crates/pm-api/src/lib.rs`: it is the file
     from before the split, with every test still in it), run this with `--base /tmp/base.rs`,
     `git add -A crates/pm-api/src` and commit;
  4. rebase onto main: git merges the area files.
What it does with each test the map names, by its text in --base and in the branch's lib.rs:
  - the same in both: dropped from lib.rs; the area file keeps its own copy;
  - changed by the branch: the area file's copy is replaced by the branch's, and listed;
  - removed (or renamed) by the branch: removed from its area file, and listed;
  - new on the branch: moved to the end of its area file, or it stays in lib.rs if the map
    does not name it.
Without --base a test that is in lib.rs and in its area file stops the script: it cannot know
which copy is the newer one. With --base a lib.rs that is already split stops it too (run a
second time, every test would look as if the branch had removed it).

--check: say what would move, write nothing. --no-fmt: don't run `cargo fmt -p pm-api` at the end
(the moved code is four columns narrower, so rustfmt joins some lines).
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
LIB = ROOT / "crates/pm-api/src/lib.rs"
TESTS = ROOT / "crates/pm-api/src/tests"
MAP = pathlib.Path(__file__).with_name("lib-tests.map")

HEADER = """//! {what}
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;
"""


def fail(msg):
    sys.exit(f"lib-tests-split: {msg}")


def lex(src):
    """Per line: bracket depth at its start, whether it starts inside a string or block comment,
    and its last character that is code (not white space or a comment)."""
    depth_at = [0]
    literal_at = [False]
    last_code = []
    depth = 0
    last = ""
    i = 0
    n = len(src)
    state = "code"  # code | line | block | str | raw
    nest = 0
    hashes = 0
    while i < n:
        c = src[i]
        if c == "\n":
            if state == "line":
                state = "code"
            last_code.append(last)
            last = ""
            depth_at.append(depth)
            literal_at.append(state in ("str", "raw", "block"))
            i += 1
            continue
        if state == "line":
            i += 1
        elif state == "block":
            if src.startswith("/*", i):
                nest += 1
                i += 2
            elif src.startswith("*/", i):
                nest -= 1
                i += 2
                if nest == 0:
                    state = "code"
            else:
                i += 1
        elif state == "str":
            if c == "\\":
                # an escaped line end stays a line end for the line count
                i += 1 if src[i + 1 : i + 2] == "\n" else 2
            elif c == '"':
                state = "code"
                last = c
                i += 1
            else:
                i += 1
        elif state == "raw":
            if c == '"' and src.startswith("#" * hashes, i + 1):
                state = "code"
                last = c
                i += 1 + hashes
            else:
                i += 1
        else:
            if src.startswith("//", i):
                state = "line"
                i += 2
            elif src.startswith("/*", i):
                state = "block"
                nest = 1
                i += 2
            elif c == '"':
                state = "str"
                i += 1
            elif c == "r" and not (i and (src[i - 1].isalnum() or src[i - 1] == "_")) or (
                c == "r" and i and src[i - 1] == "b" and not (i > 1 and (src[i - 2].isalnum() or src[i - 2] == "_"))
            ):
                m = re.compile(r'r(#*)"').match(src, i)
                if m:
                    state = "raw"
                    hashes = len(m.group(1))
                    i = m.end()
                else:
                    last = c
                    i += 1
            elif c == "'":
                if src[i + 1 : i + 2] == "\\":
                    j = src.index("'", i + 3)
                    last = "'"
                    i = j + 1
                elif src[i + 2 : i + 3] == "'":
                    last = "'"
                    i += 3
                else:
                    last = c
                    i += 1  # a lifetime
            else:
                if c in "{([":
                    depth += 1
                elif c in "})]":
                    depth -= 1
                if not c.isspace():
                    last = c
                i += 1
    last_code.append(last)
    return depth_at, literal_at, last_code


def chunks(lines, depth_at, literal_at, last_code, first, end, depth=1):
    """Lines first..end as the items of one level (`depth`: 1 inside `mod tests`, 0 for a file):
    ('blank', a, b, None, False) or ('item', a, b, name, is_test); b is exclusive. An item takes
    the comment and attribute lines right above it."""
    out = []
    j = first
    while j < end:
        if depth_at[j] != depth or literal_at[j]:
            fail(f"line {j + 1}: expected the start of an item")
        if not lines[j].strip():
            out.append(("blank", j, j + 1, None, False))
            j += 1
            continue
        a = j
        attrs = []
        # comment and attribute lines above the item
        while j < end:
            s = lines[j].strip()
            if s.startswith("//"):
                j += 1
            elif s.startswith("#["):
                k = j
                while depth_at[k + 1] != depth:
                    k += 1
                if last_code[k] != "]":
                    # an attribute with its item on the same line: the item starts here
                    break
                attrs.append(" ".join(l.strip() for l in lines[j : k + 1]))
                j = k + 1
            else:
                break
        if j >= end or not lines[j].strip():
            # comments with nothing under them
            out.append(("item", a, j, None, False))
            continue
        k = j
        while not (depth_at[k + 1] == depth and last_code[k] in ("}", ";")):
            k += 1
            if k + 1 >= len(depth_at):
                fail(f"line {j + 1}: this item never ends")
        m = re.match(r"\s*(?:pub(?:\([a-z]+\))? )?(?:async )?fn ([A-Za-z0-9_]+)", lines[j])
        is_test = bool(m) and any(re.match(r"#\[(tokio::)?test\b", t) for t in attrs)
        out.append(("item", a, k + 1, m.group(1) if m else None, is_test))
        j = k + 1
    return out


def tests_module(src):
    """lib.rs as lines, its lexing, and where `mod tests` begins and ends (first line of the
    body, line of the closing brace)."""
    lines = src.split("\n")
    depth_at, literal_at, last_code = lex(src)
    heads = [i for i, l in enumerate(lines) if l == "mod tests {" and i and lines[i - 1] == "#[cfg(test)]"]
    if len(heads) != 1:
        fail(f"expected one `#[cfg(test)] mod tests {{` in lib.rs, found {len(heads)}")
    first = heads[0] + 1
    end = next(i for i in range(first, len(lines)) if depth_at[i + 1] == 0)
    if lines[end] != "}":
        fail(f"lib.rs:{end + 1}: expected the end of `mod tests`")
    return lines, depth_at, literal_at, last_code, first, end


def dedent(lines, literal_at, a, b):
    """Lines a..b one level out; a line that starts inside a string is that string's text and
    is left as it is."""
    return "\n".join(l[4:] if l.startswith("    ") and not literal_at[i] else l for i, l in zip(range(a, b), lines[a:b]))


# A test that names its own place would mean something else in another file
PLACE = re.compile(r"include_(str|bytes)!\(\s*\"|#\[path\b|\bsuper::|\btests::|module_path!|\bfile!\(")
TEST_FN = re.compile(
    r"^[ \t]*#\[(?:tokio::)?test\b.*?^[ \t]*(?:pub(?:\([a-z]+\))? )?(?:async )?fn ([A-Za-z0-9_]+)", re.M | re.S
)


def main():
    args = sys.argv[1:]
    check = "--check" in args
    base_path = args[args.index("--base") + 1] if "--base" in args else None
    lines, depth_at, literal_at, last_code, first, end = tests_module(LIB.read_text())

    area_of = {}
    what = {}
    for ln in MAP.read_text().split("\n"):
        if ln.startswith("# area "):
            name, text = ln[len("# area ") :].split(": ", 1)
            what[name] = text
        elif ln and not ln.startswith("#"):
            test, area = ln.split()
            area_of[test] = area
    roots = set(re.findall(r"^(?:pub(?:\([a-z]+\))? )?mod ([a-z0-9_]+);", "\n".join(lines), re.M))
    for area in set(area_of.values()):
        if area in roots:
            fail(f"area {area} has the name of a module of the crate: tests would see the wrong one")
        if area not in what:
            fail(f"lib-tests.map: no `# area {area}: ...` line")

    # The tests of --base, by name, as they were written there (same indentation as lib.rs)
    base_text = {}
    if base_path:
        split_already = sorted(
            set(re.findall(r"^    mod ([a-z0-9_]+);", "\n".join(lines[first:end]), re.M)) & set(area_of.values())
        )
        if split_already:
            fail(
                f"lib.rs already declares the area files ({', '.join(split_already)}): it is split. --base is "
                "for a branch's lib.rs from before the split; put that file in place first (the steps are at "
                "the top of this script)"
            )
        bl, bd, bli, blc, bf, be = tests_module(pathlib.Path(base_path).read_text())
        for kind, a, b, name, is_test in chunks(bl, bd, bli, blc, bf, be):
            if is_test:
                base_text[name] = "\n".join(bl[a:b])

    # The area files as they are: their lines and where each test is
    area_lines = {}
    area_tests = {}
    for area in sorted(set(area_of.values())):
        path = TESTS / f"{area}.rs"
        if not path.exists():
            continue
        text = path.read_text()
        al = text.split("\n")
        ad, ali, alc = lex(text)
        area_lines[area] = al
        area_tests[area] = {
            name: (a, b) for kind, a, b, name, is_test in chunks(al, ad, ali, alc, 0, len(al), 0) if is_test
        }

    if base_path:
        # --base compares a branch's lib.rs with the area files of the split: all of them
        lacking = sorted(a for a in set(area_of.values()) if a not in area_lines)
        if lacking:
            fail(
                f"--base needs the area files of the split, and tests/{lacking[0]}.rs"
                f"{' and ' + str(len(lacking) - 1) + ' more' if len(lacking) > 1 else ''} are not here: check out the "
                "commit of the split first (the steps are at the top of this script)"
            )

    body = chunks(lines, depth_at, literal_at, last_code, first, end)
    in_lib = [c[3] for c in body if c[4]]
    if len(in_lib) != len(set(in_lib)):
        fail("two tests of lib.rs have the same name")

    appended = {}  # area -> [text]
    replaced = {}  # area -> {name: text}
    dropped = []
    keep = []
    stay = []
    skip_blank = False
    for kind, a, b, name, is_test in body:
        if kind == "blank":
            if skip_blank:
                skip_blank = False
                continue
            keep.extend(lines[a:b])
            continue
        skip_blank = False
        text = "\n".join(lines[a:b])
        area = area_of.get(name) if is_test else None
        if area and PLACE.search(text):
            print(f"{name} stays in lib.rs: it names its own place (super::, tests::, file!, include_str!, #[path] ...)")
            area = None
        if not area:
            if is_test:
                stay.append(name)
            keep.extend(lines[a:b])
            continue
        skip_blank = True
        if base_path and base_text.get(name) == text:
            # As it was where the branch started, so its area file has it at the commit of the
            # split. If not, these are not the split's area files (a newer main that has since
            # removed or renamed the test): dropping it here could lose it, so stop.
            if name not in area_tests.get(area, {}):
                fail(
                    f"{name} is unchanged on this branch but tests/{area}.rs does not have it: the area files "
                    "here are not the ones of the split. Run this at the commit of the split and rebase onto "
                    "main afterwards (the steps are at the top of this script)"
                )
            dropped.append(name)
        elif name not in area_tests.get(area, {}):
            appended.setdefault(area, []).append(dedent(lines, literal_at, a, b))
        elif not base_path:
            fail(
                f"{name} is in lib.rs and in tests/{area}.rs. For a branch made before the split, say where it "
                "started: --base <lib.rs as it was there> (see the top of this script)"
            )
        elif name not in base_text:
            fail(f"{name} is in lib.rs and in tests/{area}.rs but not in --base: two different tests of one name")
        else:
            replaced.setdefault(area, {})[name] = dedent(lines, literal_at, a, b)
    while keep and not keep[-1].strip():
        keep.pop()

    # A test of --base that the branch's lib.rs no longer has: the branch removed or renamed it
    removed = {}
    for name in sorted(set(base_text) - set(in_lib)):
        area = area_of.get(name)
        if area and name in area_tests.get(area, {}):
            removed.setdefault(area, set()).add(name)

    areas_written = sorted(set(appended) | set(replaced) | set(removed))
    declared = set(re.findall(r"^    mod ([a-z0-9_]+);", "\n".join(keep), re.M))
    # (every area that has a file: a branch's lib.rs from before the split declares none of them)
    new_mods = [a for a in sorted(set(areas_written) | set(area_lines)) if a not in declared]
    if new_mods:
        keep.append("")
        if not any(a in declared for a in set(area_of.values())):
            keep.append("    // The tests by area, one file each (tests/<area>.rs). A new test goes at the end of")
            keep.append("    // its area's file; the helpers above are shared by all of them.")
        keep.extend(f"    mod {a};" for a in new_mods)
    new_lib = "\n".join(lines[:first] + keep + lines[end:])

    files = {}
    for area in areas_written:
        path = TESTS / f"{area}.rs"
        if area in area_lines:
            al = area_lines[area]
            out = []
            at = 0
            for name, (a, b) in sorted(area_tests[area].items(), key=lambda x: x[1]):
                if name in replaced.get(area, {}):
                    out.extend(al[at:a])
                    out.append(replaced[area][name])
                    at = b
                elif name in removed.get(area, ()):
                    out.extend(al[at:a])
                    # (and the blank line after it)
                    at = b + 1 if b < len(al) and not al[b].strip() else b
            out.extend(al[at:])
            text = "\n".join(out)
        else:
            text = HEADER.format(what=what[area])
        if appended.get(area):
            text = text.rstrip("\n") + "\n\n" + "\n\n".join(appended[area]) + "\n"
        files[path] = text
        print(
            f"{area}.rs\t{len(appended.get(area, []))} tests moved\t{len(replaced.get(area, {}))} replaced"
            f"\t{len(removed.get(area, ()))} removed"
        )
    for area, names in sorted(replaced.items()):
        print(f"replaced in tests/{area}.rs with this branch's text (check against main's): {', '.join(sorted(names))}")
    for area, names in sorted(removed.items()):
        print(f"removed from tests/{area}.rs (this branch removed or renamed them): {', '.join(sorted(names))}")
    if dropped:
        print(f"{len(dropped)} tests dropped from lib.rs: unchanged since --base, their area files have them")
    if stay:
        print(f"{len(stay)} tests stay in lib.rs: {', '.join(stay)}")

    # Every test is still there, once: counted in the text as it is and as it will be written
    untouched = "".join(p.read_text() for p in sorted(TESTS.glob("*.rs")) if p not in files) if TESTS.exists() else ""
    was = "\n".join(lines[first:end]) + untouched + "".join(p.read_text() for p in files if p.exists())
    now = "\n".join(keep) + untouched + "".join(files.values())
    # (tests the branch removed from an area file are meant to be gone from there; one the branch
    # moved to another file of tests/ is still counted once below)
    meant = {n for names in removed.values() for n in names}
    names_was = set(TEST_FN.findall(was)) - meant
    names_now = sorted(TEST_FN.findall(now))
    if names_was != set(names_now) - meant:
        gone = sorted(names_was - set(names_now))
        new = sorted(set(names_now) - meant - names_was)
        fail(f"the tests after the move are not the tests before it (gone: {gone}, new: {new}): nothing written")
    twice = sorted({n for n in names_now if names_now.count(n) > 1})
    if twice:
        fail(f"a test would be there twice ({twice}): nothing written")
    missed = sorted(set(in_lib) - set(names_now))
    if missed:
        fail(f"tests of lib.rs the written text does not have ({missed}): nothing written")

    n_moved = sum(len(t) for t in appended.values())
    n_replaced = sum(len(t) for t in replaced.values())
    n_removed = sum(len(t) for t in removed.values())
    summary = (
        f"{n_moved} moved, {n_replaced} replaced, {n_removed} removed, "
        f"{len(dropped)} dropped as unchanged, {len(stay)} stay"
    )
    lib_now = new_lib.count("\n") + 1
    if check:
        print(f"ok: {summary}; lib.rs {len(lines)} -> {lib_now} lines (nothing written)")
        return
    if not (n_moved or n_replaced or n_removed or dropped):
        print("ok: nothing to move")
        return
    TESTS.mkdir(exist_ok=True)
    LIB.write_text(new_lib)
    for path, text in files.items():
        path.write_text(text)
    if "--no-fmt" not in args:
        subprocess.run(["cargo", "fmt", "-p", "pm-api"], cwd=ROOT, check=True)
    print(f"ok: {summary}; lib.rs {len(lines)} -> {len(LIB.read_text().split(chr(10)))} lines")


if __name__ == "__main__":
    main()
