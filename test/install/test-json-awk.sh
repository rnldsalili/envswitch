#!/bin/sh
set -eu
root=$1
shift
work=$(mktemp -d "$root/test/install/.awk-suite-XXXXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
sed -n '/^JSON_AWK=/,/^JSON_AWK_END$/p' "$root/scripts/install.sh" | sed '1d;$d' > "$work/parser.awk"
[ -s "$work/parser.awk" ] || exit 1
count=0
tab=$(printf '\t')
while IFS="$tab" read -r expected json; do
  json=${json#@}; json=${json%@}
  if printf '%s\n' "$json" | LC_ALL=C "$@" -f "$work/parser.awk" > "$work/output"; then actual=valid; else actual=invalid; fi
  if [ "$actual" != "$expected" ]; then printf 'JSON case failed: %s %s\n' "$expected" "$json" >&2; exit 1; fi
  count=$((count+1))
done < "$root/test/install/json-cases.tsv"
printf 'awk JSON grammar assertions: %s passed\n' "$count"
