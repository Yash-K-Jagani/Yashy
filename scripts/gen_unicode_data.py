#!/usr/bin/env python3
"""Generate crates/yashy-runtime/src/unicode_data.h from vendored UCD files.

Offline: reads only the checked-in .txt files under
crates/yashy-runtime/data/unicode/. No network at build time. Run with:

    python3 scripts/gen_unicode_data.py

The header records the SHA-256 of each input so the Rust conformance test can
verify the header is current without invoking Python.

Tables emitted:

  UCD_CASE_FOLD      full case folding (CaseFolding.txt)
  UCD_EAW_WIDE       East Asian Wide/Full ranges (EastAsianWidth.txt)
  UCD_CCC            canonical combining class (UnicodeData.txt field 3)
  UCD_CANON_DECOMP   fully recursive canonical decompositions
  UCD_COMPAT_DECOMP  fully recursive compatibility decompositions
  UCD_COMPOSE        primary composites for NFC/NFKC

Hangul syllables are handled algorithmically in C, so they are omitted from the
decomposition and composition tables.
"""

import hashlib
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "crates" / "yashy-runtime" / "data" / "unicode"
OUT = ROOT / "crates" / "yashy-runtime" / "src" / "unicode_data.h"

SOURCE_FILES = [
    "UnicodeData.txt",
    "CaseFolding.txt",
    "CompositionExclusions.txt",
    "EastAsianWidth.txt",
    "NormalizationTest.txt",
    "ReadMe.txt",
]

HANGUL_FIRST = 0xAC00
HANGUL_LAST = 0xD7A3


def sha256(name):
    return hashlib.sha256((DATA / name).read_bytes()).hexdigest()


def parse_casefolding():
    """Full case folding: prefer F (full) mappings, else C (common).

    Returns a dict codepoint -> tuple of target codepoints. S (simple) is a
    subset of C/F and T (Turkish) is locale-specific, so both are skipped.
    """
    folds = {}
    for line in (DATA / "CaseFolding.txt").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [p.strip() for p in line.split(";")]
        if len(parts) < 3:
            continue
        source = int(parts[0], 16)
        status = parts[1]
        targets = tuple(int(x, 16) for x in parts[2].split())
        if status == "F":
            folds[source] = targets
        elif status == "C" and source not in folds:
            folds[source] = targets
    return dict(sorted(folds.items()))


def parse_east_asian_width():
    """Return sorted (lo, hi) ranges whose class is W or F (wide or full)."""
    ranges = []
    for line in (DATA / "EastAsianWidth.txt").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        spec, _, cls = line.partition(";")
        spec = spec.strip()
        cls = cls.strip()
        if cls not in ("W", "F"):
            continue
        if ".." in spec:
            lo, hi = spec.split("..", 1)
        else:
            lo = hi = spec
        ranges.append((int(lo, 16), int(hi, 16)))
    ranges.sort()
    return ranges


def parse_unicode_data():
    """Return (ccc, canonical, compatibility) direct mappings.

    `canonical` holds only untagged decompositions; `compatibility` also holds
    the tagged ones. Hangul syllables are omitted: they are algorithmic.
    """
    ccc = {}
    canonical = {}
    compatibility = {}
    for line in (DATA / "UnicodeData.txt").read_text(encoding="utf-8").splitlines():
        if not line:
            continue
        fields = line.split(";")
        if len(fields) < 6:
            continue
        cp = int(fields[0], 16)
        combining = int(fields[3]) if fields[3] else 0
        if combining:
            ccc[cp] = combining
        decomposition = fields[5].strip()
        if not decomposition:
            continue
        if HANGUL_FIRST <= cp <= HANGUL_LAST:
            continue
        parts = decomposition.split()
        if parts[0].startswith("<"):
            compatibility[cp] = tuple(int(x, 16) for x in parts[1:])
        else:
            mapping = tuple(int(x, 16) for x in parts)
            canonical[cp] = mapping
            compatibility[cp] = mapping
    return ccc, canonical, compatibility


def full_decomposition(mapping):
    """Expand every mapping recursively so C needs no recursion."""
    memo = {}

    def expand(cp):
        if HANGUL_FIRST <= cp <= HANGUL_LAST:
            return (cp,)
        if cp in memo:
            return memo[cp]
        direct = mapping.get(cp)
        if direct is None:
            result = (cp,)
        else:
            out = []
            for piece in direct:
                out.extend(expand(piece))
            result = tuple(out)
        memo[cp] = result
        return result

    return {cp: expand(cp) for cp in sorted(mapping)}


def parse_composition_exclusions():
    excluded = set()
    for line in (DATA / "CompositionExclusions.txt").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        excluded.add(int(line, 16))
    return excluded


def primary_composites(canonical, ccc, excluded):
    """Pairs (starter, combining) -> composite for NFC/NFKC.

    Only length-2 canonical decompositions are primary composites. A
    decomposition whose first code point is a non-starter is a full-composition
    exclusion even when it is not listed in CompositionExclusions.txt, so it is
    dropped here (UAX #15, Full_Composition_Exclusion).
    """
    pairs = {}
    for cp, mapping in canonical.items():
        if len(mapping) != 2:
            continue
        if cp in excluded:
            continue
        starter, combining = mapping
        if ccc.get(starter, 0) != 0:
            continue
        pairs[(starter, combining)] = cp
    return dict(sorted(pairs.items()))


def emit_mapping(lines, struct_name, table_name, mapping, emit_struct=True):
    pool = []
    rows = []
    for cp, seq in mapping.items():
        rows.append((cp, len(pool), len(seq)))
        pool.extend(seq)
    if emit_struct:
        lines.append("struct %s { unsigned cp; int offset; int length; };" % struct_name)
    lines.append("static const struct %s %s[] = {" % (struct_name, table_name))
    for cp, offset, length in rows:
        lines.append("    {0x%Xu, %d, %d}," % (cp, offset, length))
    lines.append("};")
    lines.append("static const int %s_count = %d;" % (table_name, len(rows)))
    lines.append("")
    pool_name = table_name + "_POOL"
    lines.append("static const unsigned %s[] = {" % pool_name)
    for index in range(0, len(pool), 12):
        chunk = pool[index:index + 12]
        lines.append("    " + " ".join("0x%Xu," % cp for cp in chunk))
    lines.append("};")
    lines.append("static const int %s_count = %d;" % (pool_name, len(pool)))
    lines.append("")


def main():
    folds = parse_casefolding()
    wide = parse_east_asian_width()
    ccc, canonical, compatibility = parse_unicode_data()
    excluded = parse_composition_exclusions()
    canon_full = full_decomposition(canonical)
    compat_full = full_decomposition(compatibility)
    composites = primary_composites(canonical, ccc, excluded)

    lines = []
    lines.append("/* Case folding, display width and normalization tables generated")
    lines.append(" * from UnicodeData.txt, CaseFolding.txt, CompositionExclusions.txt and")
    lines.append(" * EastAsianWidth.txt (Unicode 18.0.0).")
    lines.append(" * Generated by scripts/gen_unicode_data.py. Do not edit by hand. */")
    lines.append("#pragma once")
    lines.append("")
    lines.append("/* Input hashes, so a test can verify this header is current. */")
    for name in SOURCE_FILES:
        lines.append("/* %s sha256:%s */" % (name, sha256(name)))
    lines.append("")
    lines.append("struct ucd_fold { unsigned from; unsigned to[3]; unsigned count; };")
    lines.append("static const struct ucd_fold UCD_CASE_FOLD[] = {")
    for source, targets in folds.items():
        padded = list(targets) + [0] * (3 - len(targets))
        lines.append(
            "    {0x%Xu, {0x%Xu, 0x%Xu, 0x%Xu}, %du},"
            % (source, padded[0], padded[1], padded[2], len(targets))
        )
    lines.append("};")
    lines.append("static const int UCD_CASE_FOLD_count = %d;" % len(folds))
    lines.append("")
    lines.append("struct ucd_range { unsigned lo, hi; };")
    lines.append("static const struct ucd_range UCD_EAW_WIDE[] = {")
    for lo, hi in wide:
        lines.append("    {0x%Xu, 0x%Xu}," % (lo, hi))
    lines.append("};")
    lines.append("static const int UCD_EAW_WIDE_count = %d;" % len(wide))
    lines.append("")
    lines.append("struct ucd_ccc { unsigned cp; int ccc; };")
    lines.append("static const struct ucd_ccc UCD_CCC[] = {")
    for cp, value in sorted(ccc.items()):
        lines.append("    {0x%Xu, %d}," % (cp, value))
    lines.append("};")
    lines.append("static const int UCD_CCC_count = %d;" % len(ccc))
    lines.append("")
    emit_mapping(lines, "ucd_decomp", "UCD_CANON_DECOMP", canon_full)
    emit_mapping(lines, "ucd_decomp", "UCD_COMPAT_DECOMP", compat_full, emit_struct=False)
    lines.append("struct ucd_pair { unsigned starter; unsigned combining; unsigned composite; };")
    lines.append("static const struct ucd_pair UCD_COMPOSE[] = {")
    for (starter, combining), composite in composites.items():
        lines.append("    {0x%Xu, 0x%Xu, 0x%Xu}," % (starter, combining, composite))
    lines.append("};")
    lines.append("static const int UCD_COMPOSE_count = %d;" % len(composites))
    lines.append("")

    OUT.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(
        "wrote %s (%d folds, %d wide ranges, %d ccc, %d canonical, %d compat, %d composites)"
        % (
            OUT,
            len(folds),
            len(wide),
            len(ccc),
            len(canon_full),
            len(compat_full),
            len(composites),
        )
    )


if __name__ == "__main__":
    main()
