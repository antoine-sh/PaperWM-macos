#!/usr/bin/env bash
# Packages build/PaperWM.app (from scripts/build-app.sh) as
# build/PaperWM-<version>.dmg, with an Applications shortcut to drag it onto.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

app="build/PaperWM.app"
if [[ ! -d "$app" ]]; then
    echo "$app not found; run scripts/build-app.sh first" >&2
    exit 1
fi

version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
dmg="build/PaperWM-$version.dmg"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"

echo "==> Creating $dmg"
rm -f "$dmg"
hdiutil create -volname "PaperWM" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg"
echo "==> Built $dmg"
