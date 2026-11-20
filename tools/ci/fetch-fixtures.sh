#!/usr/bin/env bash
# Fetch the public test inputs and verify each against its pin.
#
#   tools/ci/fetch-fixtures.sh TARGET_DIR
#
# TARGET_DIR/skp/<path>   skpwright's corpus files (GPL-3.0-only, third-party textures aside)
#                         at the pinned commit, listed with their sha256 in skpwright-corpus.sha256.
# TARGET_DIR/rvt/         rvt-open's synthetic project files at the pinned commit, and its tiny
#                         reference models (release asset fixtures-tiny-1, MIT).
# Nothing is committed to this repository. Exits 1 on a mismatch and 2 when a download fails.
set -euo pipefail

TARGET="${1:?usage: fetch-fixtures.sh TARGET_DIR}"
HERE="$(cd "$(dirname "$0")" && pwd)"
SKPWRIGHT_COMMIT=491e1fc7bb8ba135f7e3863d38abda29004c68bf
RVT_OPEN_COMMIT=c707ff98cdb14e68e28adc9edb5c2d3d1cb7945c
TINY_SHA=7f22fcc0a1d4753e18245577755c1072830e87df0b8953f7e5f254b2e46de0ab
RVT_SYNTHETIC=(
  "corpus/tier1/architectural-2024/architectural-2024.rvt d7ee4f7f5918600e73c0e22292682ad4f86ffd9d03b31ee7e3c50f6307a8ef77"
)

mkdir -p "$TARGET/skp" "$TARGET/rvt"
get() { curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 600 -o "$2" "$1" || { echo "could not download $1"; exit 2; }; }

# skpwright corpus: one file per line of the pin list, eight at a time.
export TARGET SKPWRIGHT_COMMIT
fetch_one() {
  sha="$1"; path="$2"; dest="$TARGET/skp/${path#corpus/}"
  mkdir -p "$(dirname "$dest")"
  if [ -f "$dest" ] && echo "$sha  $dest" | sha256sum -c --quiet - 2>/dev/null; then return 0; fi
  curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 --max-time 600 -o "$dest" \
    "https://raw.githubusercontent.com/hew3d/skpwright/$SKPWRIGHT_COMMIT/$path" || { echo "could not download $path"; exit 2; }
  echo "$sha  $dest" | sha256sum -c --quiet - || { echo "$path: sha256 does not match the pin"; exit 1; }
}
export -f fetch_one
xargs -P 8 -L 1 bash -c 'fetch_one "$0" "$1"' < "$HERE/skpwright-corpus.sha256"
echo "$TARGET/skp: $(find "$TARGET/skp" -name '*.skp' | wc -l) files verified"

# rvt-open's synthetic project files.
for entry in "${RVT_SYNTHETIC[@]}"; do
  read -r path sha <<< "$entry"
  dest="$TARGET/rvt/$(basename "$path")"
  if ! { [ -f "$dest" ] && echo "$sha  $dest" | sha256sum -c --quiet - 2>/dev/null; }; then
    get "https://raw.githubusercontent.com/helenkwok/rvt-open/$RVT_OPEN_COMMIT/$path" "$dest"
    echo "$sha  $dest" | sha256sum -c --quiet - || { echo "$dest: sha256 does not match the pin"; exit 1; }
  fi
done

# rvt-open's tiny reference models.
if [ ! -f "$TARGET/rvt/rvt-open-tiny-models-1/SHA256SUMS" ] || ! (cd "$TARGET/rvt/rvt-open-tiny-models-1" && sha256sum --quiet -c SHA256SUMS); then
  tgz="$TARGET/rvt/tiny-1.tar.gz"
  get "https://github.com/helenkwok/rvt-open/releases/download/fixtures-tiny-1/rvt-open-tiny-models-1.tar.gz" "$tgz"
  echo "$TINY_SHA  $tgz" | sha256sum -c --quiet - || { echo "$tgz: sha256 does not match the pin"; exit 1; }
  rm -rf "$TARGET/rvt/rvt-open-tiny-models-1"
  tar -xzf "$tgz" -C "$TARGET/rvt" --no-same-owner
  rm -f "$tgz"
  (cd "$TARGET/rvt/rvt-open-tiny-models-1" && sha256sum --quiet -c SHA256SUMS) || { echo "tiny models: SHA256SUMS check failed"; exit 1; }
fi
echo "$TARGET/rvt: $(find "$TARGET/rvt" -name '*.rvt' | wc -l) models verified"
