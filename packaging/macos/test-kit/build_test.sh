set -euo pipefail

kit_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
verify=0
for argument in "$@"; do
    case "$argument" in
        --verify) verify=1 ;;
        *) printf 'Unknown option: %s\n' "$argument" >&2; exit 2 ;;
    esac
done

if [ "$(uname -s)" != "Darwin" ]; then
    printf 'Run this kit on your Mac.\n' >&2
    exit 2
fi

commit="55dda6b6541efe16e85ffc8839c96fe650df746e"
mkdir -p "$kit_dir/logs"
build_log="$kit_dir/logs/build-$(date +%Y%m%d-%H%M%S).log"
exec > >(tee -a "$build_log") 2>&1
trap 'printf "\nStopped at line %s. Build log: %s\n" "$LINENO" "$build_log" >&2' ERR

printf 'GDAW test revision: %s\nBuild log: %s\n' "$commit" "$build_log"
if ! /usr/bin/xcode-select -p >/dev/null 2>&1 || ! /usr/bin/xcrun --find clang >/dev/null 2>&1; then
    /usr/bin/xcode-select --install || true
    printf 'Finish installing Apple Command Line Tools, then run this command again.\n'
    exit 2
fi

if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
if ! command -v rustup >/dev/null 2>&1; then
    curl --fail --location --retry 3 --proto '=https' --tlsv1.2 https://sh.rustup.rs -o "$kit_dir/rustup-init.sh"
    sh "$kit_dir/rustup-init.sh" -y --profile minimal --default-toolchain stable
    . "$HOME/.cargo/env"
fi
rustup toolchain install stable --profile minimal

if [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = "1" ]; then
    target="aarch64-apple-darwin"
else
    target="x86_64-apple-darwin"
fi
rustup target add "$target" --toolchain stable
printf 'Target: %s\n' "$target"
cargo +stable --version
rustc +stable --version

source_dir="$kit_dir/source/GDAW-$commit"
if [ ! -f "$source_dir/Cargo.toml" ]; then
    mkdir -p "$kit_dir/source"
    archive="$kit_dir/source/GDAW-$commit.zip"
    curl --fail --location --retry 3 --proto '=https' --tlsv1.2 "https://github.com/GeraldCerda/GDAW/archive/$commit.zip" -o "$archive.partial"
    mv "$archive.partial" "$archive"
    /usr/bin/ditto -x -k "$archive" "$kit_dir/source"
fi
if [ ! -f "$source_dir/Cargo.lock" ] || [ ! -f "$source_dir/apps/soundcraft/Cargo.toml" ]; then
    printf 'The pinned source archive is incomplete.\n' >&2
    exit 1
fi

export CARGO_TARGET_DIR="$kit_dir/target"
export MACOSX_DEPLOYMENT_TARGET=11.0
cd "$source_dir"

if [ "$verify" = "1" ]; then
    rustup component add rustfmt --toolchain stable
    if cargo +stable fmt --all -- --check; then
        printf 'Formatting check passed.\n'
    else
        printf 'Formatting check failed; continuing to compile and run the regression suites.\n'
    fi
    cargo +stable test --locked --release --target "$target" -p soundcraft-dsp -p soundcraft-model -p soundcraft-engine -p soundcraft-mix -p soundcraft-playback -p soundcraft-ui-egui
fi

cargo +stable build --locked --release --target "$target" -p soundcraft
binary="$CARGO_TARGET_DIR/$target/release/soundcraft"
test -x "$binary"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$source_dir/Cargo.toml" | head -n 1)"
test -n "$version"
stage="$(mktemp -d "$kit_dir/app-stage.XXXXXX")"
app="$stage/GDAW Test.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/GDAW-Test"
bash "$source_dir/packaging/macos/make-g-icon.sh" "$source_dir/Purple G icon with transparent background.png" "$app/Contents/Resources/GDAW-Test.icns"
cp "$source_dir/LICENSE-MIT" "$source_dir/LICENSE-APACHE" "$source_dir/NOTICE" "$source_dir/ATTRIBUTION.md" "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleName</key><string>GDAW Test</string>
<key>CFBundleDisplayName</key><string>GDAW Test</string>
<key>CFBundleExecutable</key><string>GDAW-Test</string>
<key>CFBundleIdentifier</key><string>com.geraldcerda.gdaw.test</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleIconFile</key><string>GDAW-Test</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.music</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
<key>NSRequiresAquaSystemAppearance</key><false/>
<key>NSMicrophoneUsageDescription</key><string>GDAW Test records audio from the input you choose.</string>
<key>GDAWBuildCommit</key><string>$commit</string>
</dict>
</plist>
PLIST
printf 'APPL????' > "$app/Contents/PkgInfo"
/usr/bin/plutil -lint "$app/Contents/Info.plist"
/usr/bin/xattr -cr "$app"
/usr/bin/codesign --force --sign - --timestamp=none --entitlements "$source_dir/packaging/macos/entitlements.plist" "$app/Contents/MacOS/GDAW-Test"
/usr/bin/codesign --force --sign - --timestamp=none --entitlements "$source_dir/packaging/macos/entitlements.plist" "$app"
/usr/bin/codesign --verify --strict --deep "$app"

destination="$HOME/Applications/GDAW Test.app"
mkdir -p "$HOME/Applications"
if [ -e "$destination" ]; then
    backup="$HOME/Applications/GDAW Test $(date +%Y%m%d-%H%M%S).app"
    mv "$destination" "$backup"
    printf 'Previous tester kept at: %s\n' "$backup"
fi
mv "$app" "$destination"
rmdir "$stage"
printf '\nInstalled tester: %s\n' "$destination"
bash "$kit_dir/launch_test.sh"
