#!/usr/bin/env bash
# Builds the Rust engine and the Swift app, and bundles them as
# build/PaperWM.app.
#
#   scripts/build-app.sh              release build for this Mac's architecture
#   scripts/build-app.sh --universal  release build for Apple Silicon and Intel
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

case "${1:-}" in
    --universal) archs=(arm64 x86_64) ;;
    "") archs=("$(uname -m)") ;;
    *) echo "usage: $0 [--universal]" >&2; exit 64 ;;
esac

version="$(sed -n 's/^version = "\(.*\)"/\1/p' engine/Cargo.toml | head -n1)"
binaries=()

for arch in "${archs[@]}"; do
    case "$arch" in
        arm64) triple=aarch64-apple-darwin ;;
        x86_64) triple=x86_64-apple-darwin ;;
        *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
    esac

    echo "==> Building engine for $triple"
    cargo build --release --locked -p paperwm-engine --target "$triple"

    echo "==> Building app for $arch"
    swift build -c release --package-path app --arch "$arch" --scratch-path "app/.build/$arch" \
        -Xlinker -L"$root/target/$triple/release"
    binaries+=("app/.build/$arch/release/PaperWM")
done

bundle="build/PaperWM.app"
echo "==> Bundling $bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
lipo -create "${binaries[@]}" -output "$bundle/Contents/MacOS/PaperWM"
sed "s/__VERSION__/$version/g" app/Resources/Info.plist > "$bundle/Contents/Info.plist"

# An ad-hoc signature gives the app a stable identity for the Accessibility
# permission. Release signing and notarization come later.
codesign --force --sign - "$bundle"

echo "==> Built $bundle"
