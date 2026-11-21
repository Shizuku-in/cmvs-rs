#!/usr/bin/env sh
# Collect upstream notices from the locked libraries linked for each architecture.
set -eu
out=$1
shift
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
while IFS="$(printf '\t')" read -r name version license manifest license_file; do
    root=${manifest%/*}
    printf '\n%s %s — %s\n\n' "$name" "$version" "$license"
    # Some upstream packages keep license collections in subdirectories.
    find "$root" -maxdepth 1 \( -iname 'licen[cs]e*' -o -iname 'copyright*' -o -iname 'copying*' -o -iname 'notice*' \) \
        -exec find {} -type f \; > "$stage/files"
    if [ "$license_file" != "-" ]; then printf '%s\n' "$root/$license_file" >> "$stage/files"; fi
    if [ ! -s "$stage/files" ]; then
        case "$name" in objc2*|block2|dispatch2) cat "$(dirname "$0")/objc2-license.md" ;; esac
    fi
    sort -u "$stage/files" > "$stage/notices"
    while IFS= read -r notice; do cat "$notice"; printf '\n'; done < "$stage/notices"
done < "$stage/sorted" > "$stage/output"
grep -Fq 'Copyright (c) 2014 Mathijs van de Nes' "$stage/output"
mv "$stage/output" "$out"
