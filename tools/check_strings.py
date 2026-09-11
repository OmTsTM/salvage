"""Checks that everything the backend can say has wording in the window.

The window falls back to showing a key when it has no text for one. That is
deliberate — a rare technical string from a lower layer is more useful shown
raw than replaced by a placeholder — but it means a missing translation is
invisible until a user hits it.

Version 0.6.0 shipped exactly that: `release_card` added an `ApplyStep` variant
and no wording for it, so the first person to release a card was told
`table_restored`. Nothing failed; there was simply nothing to fail. This is the
test that would have caught it.

Checked here rather than in Rust or in JavaScript because the mismatch lives
between them: the variants are declared in one language and worded in another,
and neither compiler can see both.

Usage:
    python tools/check_strings.py
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def snake(name):
    """`TableRestored` to `table_restored`, matching serde's rename_all."""
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def enum_variants(source, name):
    """Variant names of one enum, ignoring its fields and attributes."""
    start = source.index(f"pub enum {name}")
    body = source[start : source.index("\n}", start)]
    # Variants sit at one level of indentation; fields sit deeper.
    return re.findall(r"^\s{4}([A-Z]\w+)", body, re.M)


def dictionary_keys(source, start):
    """Keys defined in the language block beginning at `start`."""
    end = source.find(chr(10) + "    }," , start)
    return set(re.findall(r'"([\w.]+)":', source[start:end]))


def main():
    problems = []

    # Every ApplyStep the backend can emit must have a case in the window.
    apply_rs = (ROOT / "crates/salvage-win32/src/apply.rs").read_text(encoding="utf-8")
    app_js = (ROOT / "ui/app.js").read_text(encoding="utf-8")
    for variant in enum_variants(apply_rs, "ApplyStep"):
        if f'case "{snake(variant)}"' not in app_js:
            problems.append(f"ApplyStep::{variant} has no case in stepMessage()")

    # Every key one language defines, the others must define too. A key present
    # in only one leaves the rest showing its identifier.
    i18n = (ROOT / "ui/i18n.js").read_text(encoding="utf-8")
    # Anchored to the start of a line at the dictionary's own indentation. A
    # bare substring search finds `es: {` inside the English block and reads the
    # wrong half of the file — which it did, and reported a hundred keys missing
    # from a language that had them all.
    languages = {}
    for code in ("pt-BR", "en", "es", "zh"):
        m = re.search(rf'^ {{4}}"?{re.escape(code)}"?: {{', i18n, re.M)
        languages[code] = m.start() if m else None
    keys = {}
    for code, start in languages.items():
        if start is None:
            problems.append(f"no dictionary found for {code}")
            continue
        keys[code] = dictionary_keys(i18n, start)

    if len(keys) == len(languages):
        every = set().union(*keys.values())
        for code, defined in keys.items():
            for missing in sorted(every - defined):
                problems.append(f"{code} is missing the key {missing}")

    if problems:
        print(f"{len(problems)} problem(s):", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        return 1

    total = len(next(iter(keys.values()))) if keys else 0
    print(f"every ApplyStep has wording; {total} keys present in all {len(keys)} languages")
    return 0


if __name__ == "__main__":
    sys.exit(main())
