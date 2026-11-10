#!/usr/bin/env bash
# Run from a clean app checkout after npm installation and SDK staging.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
app="$root/applications/prns/app"
logs="$root/validation-artifacts/results/app-ios-release-notices"
mkdir -p "$logs"
build_dir="$(mktemp -d "${TMPDIR:-/tmp}/prns-ios-release.XXXXXXXX")"
trap 'rm -rf "$build_dir"' EXIT
export PRNS_APP_VARIANT=production CI=1 EXPO_NO_TELEMETRY=1 COCOAPODS_DISABLE_STATS=true
python3 "$root/applications/tools/native-bindings/generate.py" ios --release --targets aarch64-apple-ios
cd "$app"
node ../../node_modules/expo/bin/cli prebuild --platform ios --no-install --skip-dependency-update react,react-native,expo
cd ios
pod _1.16.2_ install
target=PRNSChat
map="$build_dir/main.jsbundle.map"
xcodebuild -workspace "$target.xcworkspace" -scheme "$target" -configuration Release \
  -destination 'generic/platform=iOS' -derivedDataPath "$build_dir/DerivedData" -jobs 4 -quiet \
  "SOURCEMAP_FILE=$map" CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO build
product="$build_dir/DerivedData/Build/Products/Release-iphoneos/$target.app"
mkdir "$build_dir/Payload"
mv "$product" "$build_dir/Payload/$target.app"
product="$build_dir/Payload/$target.app"
(cd "$build_dir" && /usr/bin/zip -qr release.ipa Payload)
python3 "$root/applications/tools/native-bindings/check_release_notices.py" \
  "$build_dir/release.ipa" --bundle "$product/main.jsbundle" --source-map "$map" \
  --pods-acknowledgements "Pods/Target Support Files/Pods-$target/Pods-$target-acknowledgements.markdown" \
  --output "$logs/receipt.json"
