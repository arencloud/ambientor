#!/usr/bin/env bash
# Regenerate logo variants from docs/images/logo/ambientor.png (requires ImageMagick).
set -euo pipefail
cd "$(dirname "$0")/.."
SRC="docs/images/logo/ambientor.png"
OUT="docs/images/logo"
WEB="crates/ambientor-web/assets/logo"

if ! command -v magick >/dev/null 2>&1 && ! command -v convert >/dev/null 2>&1; then
  echo "ImageMagick (magick or convert) is required." >&2
  exit 1
fi

run_magick() {
  if command -v magick >/dev/null 2>&1; then
    magick "$@"
  else
    convert "$@"
  fi
}

mkdir -p "${OUT}" "${WEB}"
# Emblem crop (top portion without wordmark text).
run_magick "${SRC}" -crop 1254x820+0+0 +repage -resize 256x256 "${OUT}/ambientor-icon-256.png"
run_magick "${OUT}/ambientor-icon-256.png" -resize 64x64 "${OUT}/ambientor-icon-64.png"
run_magick "${OUT}/ambientor-icon-256.png" -resize 32x32 "${OUT}/ambientor-icon-32.png"
cp "${OUT}/ambientor-icon-64.png" "${OUT}/ambientor-icon.png"

cp "${OUT}/ambientor-icon-64.png" "${WEB}/icon-64.png"
cp "${OUT}/ambientor-icon-256.png" "${WEB}/icon-256.png"
run_magick "${OUT}/ambientor-icon-32.png" "${OUT}/ambientor-icon-64.png" "${OUT}/ambientor-icon-256.png" -colors 256 "${WEB}/favicon.ico"

echo "Generated icons in ${OUT} and ${WEB}"
