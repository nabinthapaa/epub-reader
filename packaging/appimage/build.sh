#!/usr/bin/env bash
# Build folio-x86_64.AppImage from an existing release build.
#
# Needs appimagetool on PATH (https://github.com/AppImage/AppImageKit).
# Bundling is deliberately minimal: the GPU and font stacks are used from the
# host, which is what a reader app wants and what keeps the image small.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
appdir="$root/build/AppDir"
binary="$root/target/release/folio"

[ -x "$binary" ] || { echo "build first: cargo build --release" >&2; exit 1; }
command -v appimagetool >/dev/null || { echo "appimagetool not found on PATH" >&2; exit 1; }

rm -rf "$appdir"
install -Dm755 "$binary" "$appdir/usr/bin/folio"
install -Dm644 "$root/packaging/folio.desktop" "$appdir/usr/share/applications/folio.desktop"
install -Dm644 "$root/packaging/folio.svg" "$appdir/usr/share/icons/hicolor/scalable/apps/folio.svg"
install -Dm644 "$root/packaging/folio.svg" "$appdir/folio.svg"
cp "$root/packaging/folio.desktop" "$appdir/folio.desktop"

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/folio" "$@"
RUN
chmod +x "$appdir/AppRun"

ARCH=x86_64 appimagetool "$appdir" "$root/build/folio-x86_64.AppImage"
echo "wrote build/folio-x86_64.AppImage"
