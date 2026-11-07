#!/bin/sh
# Compiles platforms/psvita/shaders/*.cg to the committed .gxp binaries in Vita3K.
# Needs Vita3K, a decrypted libshacccg.suprx in its ur0:data/ and the VPK from
# build-shaderc.sh. VITA3K is the executable, VITA3K_DATA its data root.
set -eu
cd "$(dirname "$0")/../../.."
: "${VITA3K:=$HOME/Applications/Vita3K/Vita3K.AppImage}"
: "${VITA3K_DATA:=$HOME/.local/share/Vita3K/Vita3K}"
work="$VITA3K_DATA/ux0/data/cmvs-shaderc"
if [ ! -f "$VITA3K_DATA/ur0/data/libshacccg.suprx" ]; then
    echo "missing $VITA3K_DATA/ur0/data/libshacccg.suprx" >&2
    exit 1
fi
app="$VITA3K_DATA/ux0/app/CMVS00098"
rm -rf "$app" "$work"
mkdir -p "$app" "$work"
unzip -q platforms/psvita/build/cmvs-vita-shaderc.vpk -d "$app"
cp platforms/psvita/shaders/*.cg "$work/"
# Vita3K stays open after the program exits; its own process group is ended here.
setsid "$VITA3K" -w --frontend sdl -r CMVS00098 >"$work/vita3k.log" 2>&1 &
group=$!
tries=0
while [ ! -f "$work/done.txt" ] && [ $tries -lt 120 ]; do
    sleep 1
    tries=$((tries + 1))
done
sleep 1
kill -TERM -- "-$group" 2>/dev/null || true
cat "$work/log.txt" 2>/dev/null || true
if [ "$(cat "$work/done.txt" 2>/dev/null)" != ok ]; then
    echo "shader compilation failed; Vita3K log: $work/vita3k.log" >&2
    exit 1
fi
cp "$work"/*.gxp platforms/psvita/shaders/
