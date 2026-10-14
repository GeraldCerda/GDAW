set -euo pipefail

input="$1"
output="$2"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
iconset="$work/G.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    retina=$((size * 2))
    /usr/bin/sips -s format png -z "$size" "$size" "$input" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    /usr/bin/sips -s format png -z "$retina" "$retina" "$input" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
/usr/bin/iconutil -c icns "$iconset" -o "$output"
