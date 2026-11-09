#!/bin/bash
# -- WP REL013M -- macOS release of the Swift app: a universal (arm64 + x86_64)
# Release "DSPi uConsole.app" with the licence texts and the notice
# (Contents/Resources/NOTICE.txt and LICENSES/, from the Qt .dmg's
# packaging/licences/stage.sh), ad-hoc signed, and a .dmg of it:
# DSPi-uConsole-Swift-<version>-macos.dmg, apart from the Qt app's
# DSPi-uConsole-<version>-macos.dmg. Not notarised (no Apple ID).
#
#   apps/apple/package-macos.sh [output dir, default dist/]
#
# On the Mac (the macOS VM): Xcode and rustup with the Apple targets
# (build-xcframework.sh). DERIVED_DATA (default apps/apple/build/package-macos)
# holds Xcode's output. The version is the macOS app's MARKETING_VERSION.
set -euo pipefail
[ "$(uname -s)" = Darwin ] || { echo "package-macos: macOS only (the macOS VM)" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
out=${1:-$repo/dist}
dd=${DERIVED_DATA:-$here/build/package-macos}

"$here/build-xcframework.sh"

cd "$here"
project="DSPi Console.xcodeproj"
build=(-project "$project" -scheme "DSPi Console" -configuration Release
       -destination "generic/platform=macOS" -derivedDataPath "$dd"
       ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO)
settings=$(xcodebuild -showBuildSettings "${build[@]}" 2>/dev/null)
setting() { sed -n "s/^ *$1 = //p" <<<"$settings" | head -n 1; }
version=$(setting MARKETING_VERSION)
[ -n "$version" ] || { echo "package-macos: no MARKETING_VERSION" >&2; exit 1; }
name="DSPi-uConsole-Swift-$version-macos"

rm -rf "$dd/Build/Products/Release"
xcodebuild build -quiet "${build[@]}"

mkdir -p "$out"
app="$out/DSPi uConsole.app"
rm -rf "$app"; ditto "$dd/Build/Products/Release/DSPi uConsole.app" "$app"
exe="$app/Contents/MacOS/DSPi uConsole"
for a in arm64 x86_64; do
    lipo "$exe" -verify_arch "$a" || { echo "package-macos: the app has no $a slice" >&2; exit 1; }
done

# The licence texts and the notice (before signing: the signature covers them)
PACKAGE_RESOLVED="$here/$project/project.xcworkspace/xcshareddata/swiftpm/Package.resolved" \
SWIFTPM_CHECKOUTS="$dd/SourcePackages/checkouts" \
    "$repo/apps/desktop/packaging/licences/stage.sh" swift "$app" "$app/Contents/Resources" "$version"

# Ad hoc, as the Qt .dmg: without the hardened runtime, whose library
# validation could refuse the embedded Swift runtime library (Contents/Frameworks)
# when it is not signed by the same team
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

rm -f "$out/$name.dmg"
hdiutil create -quiet -volname "DSPi uConsole" -srcfolder "$app" -format UDZO "$out/$name.dmg"
lipo -info "$exe"
ls -la "$out/$name.dmg"
