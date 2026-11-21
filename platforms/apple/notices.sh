#!/usr/bin/env sh
# Collect upstream notices from the locked libraries linked for each architecture.
set -eu
out=$1
shift
script_dir=$(CDPATH= cd -P -- "$(dirname -- "$0")" && pwd)
repo_dir=$(cd "$script_dir/../.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
for target do
    cargo tree --locked -p cmvs-apple --target "$target" -e normal --prefix none > "$stage/tree"
    awk '{print $1 " " $2}' "$stage/tree" | sort -u > "$stage/linked"
    cargo metadata --locked --format-version 1 --filter-platform "$target" > "$stage/meta"
    jq -r --rawfile linked "$stage/linked" '
        ($linked | split("\n") | INDEX(.)) as $linked
        | .packages[] | select(.source and $linked[.name + " v" + .version])
        | [.name, .version, (.license // "see license file"), .manifest_path, (.license_file // "-")]
        | @tsv' "$stage/meta" >> "$stage/packages"
done
sort -u "$stage/packages" > "$stage/sorted"
python3 - "$stage/sorted" "$script_dir/objc2-license.md" "$repo_dir/LICENSE-APACHE" > "$stage/output" << 'EOF'
import os
import re
import sys

objc2_text = open(sys.argv[2], encoding="utf-8").read().strip()
apache_text = open(sys.argv[3], encoding="utf-8").read().strip()

MIT_TEXT = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

ZLIB_TEXT = """This software is provided 'as-is', without any express or implied
warranty. In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution."""

APACHE_FULL_RE = re.compile(
    r"(?:^|\n)[ \t-]*Apache License[ \t]*\n[ \t]*Version 2\.0, January 2004[ \t]*\n"
    r".*?"
    r"(?:limitations under the License\.[ \t]*(?:\n[ \t-]*)?|END OF TERMS AND CONDITIONS[ \t]*(?:\n[ \t-]*)?(?!\s*APPENDIX:))",
    re.DOTALL,
)


def strip_block(lines, start_pat, end_substr):
    out = []
    used = False
    i = 0
    while i < len(lines):
        if re.match(start_pat, lines[i]):
            j = i
            buf = []
            while j < len(lines):
                buf.append(lines[j])
                if end_substr in " ".join(x.strip() for x in buf):
                    used = True
                    i = j + 1
                    break
                j += 1
            else:
                out.append(lines[i])
                i += 1
        else:
            out.append(lines[i])
            i += 1
    return out, used


def clean_file(raw):
    text = raw.replace("\r\n", "\n")
    refs = []
    if "Apache License" in text and "Version 2.0, January 2004" in text:
        match = APACHE_FULL_RE.search(text)
        if match:
            text = text[: match.start()] + "\n" + text[match.end() :]
            refs.append("Apache-2.0")
    lines = text.splitlines()
    lines, has_mit = strip_block(
        lines,
        r"^[ \t>*`-]*Permission is hereby granted,\s*free of charge",
        "IN THE SOFTWARE.",
    )
    if has_mit:
        refs.append("MIT")
    lines, has_zlib = strip_block(
        lines,
        r"^[ \t>*`-]*This software is provided\s+['\"]as-is['\"]",
        "from any source distribution.",
    )
    if has_zlib:
        refs.append("Zlib")
    text = "\n".join(lines)
    text = re.sub(r"(?m)^[ \t#*`=-]*(?:The )?MIT License(?: \(MIT\))?[ \t*`=-]*$", "", text)
    text = re.sub(r"(?m)^[ \t#*`=-]*The zlib/libpng License(?: \(Zlib\))?[ \t*`=-]*$", "", text)
    text = re.sub(r"(?m)^[ \t]*[-=]{10,}[ \t]*$", "", text)
    text = re.sub(r"\n{3,}", "\n\n", text).strip()
    return text, refs


def infer_shared_from_spdx(lic):
    return [key for key in ("Apache-2.0", "MIT", "Zlib") if key in lic]


used_shared = set()
entries = []

with open(sys.argv[1], encoding="utf-8") as f:
    for line in f:
        name, ver, lic, manifest, lic_file = line.rstrip("\n").split("\t")
        root = os.path.dirname(manifest)
        found = set()
        for entry in os.listdir(root):
            low = entry.lower()
            if low.startswith(("license", "licence", "copyright", "copying", "notice")):
                path = os.path.join(root, entry)
                if os.path.isfile(path):
                    found.add(path)
                elif os.path.isdir(path):
                    for dirpath, _, filenames in os.walk(path):
                        for filename in filenames:
                            found.add(os.path.join(dirpath, filename))
        if lic_file != "-":
            candidate = os.path.join(root, lic_file)
            if os.path.isfile(candidate):
                found.add(candidate)

        notes = []
        refs = set()
        for path in sorted(found):
            cleaned, file_refs = clean_file(open(path, encoding="utf-8", errors="replace").read())
            refs.update(file_refs)
            if cleaned:
                notes.append(cleaned)

        deduped = []
        seen_lines = set()
        for note in notes:
            note_lines = [ln.strip() for ln in note.splitlines() if ln.strip()]
            if note in deduped or (note_lines and all(ln in seen_lines for ln in note_lines)):
                continue
            deduped.append(note)
            seen_lines.update(note_lines)

        if not found:
            if name.startswith("objc2") or name in ("block2", "dispatch2"):
                refs.add("objc2")
            refs.update(infer_shared_from_spdx(lic))

        for note in deduped:
            for ln in note.splitlines():
                if ln.startswith(("=== ", "== ", "Shared: ")):
                    raise SystemExit(f"Delimiter collision in {name}: {ln}")

        ordered_refs = [k for k in ("Apache-2.0", "MIT", "Zlib", "objc2") if k in refs]
        used_shared.update(ordered_refs)
        entries.append((name, ver, lic, ordered_refs, deduped))

print("Dependency licenses for linked Apple target crates.")
print()
print("=== PACKAGES ===")
for name, ver, lic, refs, notes in entries:
    print()
    print(f"== {name} {ver} — {lic} ==")
    if refs:
        print("Shared: " + ", ".join(refs))
    if notes:
        print()
        print("\n\n".join(notes))

shared_texts = {
    "Apache-2.0": apache_text,
    "MIT": MIT_TEXT,
    "Zlib": ZLIB_TEXT,
    "objc2": objc2_text,
}
print()
print("=== SHARED LICENSES ===")
for key in ("Apache-2.0", "MIT", "Zlib", "objc2"):
    if key in used_shared:
        print()
        print(f"== [{key}] ==")
        print()
        print(shared_texts[key])
EOF
grep -Fq 'Copyright (c) 2014 Mathijs van de Nes' "$stage/output"
mv "$stage/output" "$out"
