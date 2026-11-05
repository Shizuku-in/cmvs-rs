#!/bin/sh
set -eu
cd "$(dirname "$0")/../../.."
: "${DEVKITPRO:=/opt/devkitpro}"
: "${DEVKITA64:=$DEVKITPRO/devkitA64}"
sh platforms/switch/scripts/prepare-libc.sh
for shader in scene_vsh:vert scene_fsh:frag present_vsh:vert present_fsh:frag; do
    name=${shader%%:*}
    "$DEVKITPRO/tools/bin/uam" -s "${shader##*:}" -o "platforms/switch/build/$name.dksh" "platforms/switch/runtime/shaders/$name.glsl"
done
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/shaders.S -o platforms/switch/build/shaders.o
for unit in window gpu audio system libc keyboard; do
    "$DEVKITA64/bin/aarch64-none-elf-gcc" -c "platforms/switch/runtime/$unit.c" -o "platforms/switch/build/$unit.o" \
        -Wall -Wextra -Werror -O2 -march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE -I"$DEVKITPRO/libnx/include"
done
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/abi.c -o platforms/switch/build/abi.o
cargo +nightly-2026-01-20 rustc --config 'patch.crates-io.libc.path="platforms/switch/build/libc-0.2.178"' \
    -p cmvs-switch --release --target platforms/switch/toolchain/aarch64-switch.json -Zbuild-std=std,panic_abort -- \
    -C "linker=$PWD/platforms/switch/toolchain/linker.py" \
    -C "link-arg=-specs=$DEVKITPRO/libnx/switch.specs" \
    -C "link-arg=$PWD/platforms/switch/build/window.o" \
    -C "link-arg=$PWD/platforms/switch/build/gpu.o" \
    -C "link-arg=$PWD/platforms/switch/build/shaders.o" \
    -C "link-arg=$PWD/platforms/switch/build/audio.o" \
    -C "link-arg=$PWD/platforms/switch/build/system.o" \
    -C "link-arg=$PWD/platforms/switch/build/libc.o" \
    -C "link-arg=$PWD/platforms/switch/build/keyboard.o" \
    -C "link-arg=-L$DEVKITPRO/libnx/lib" -C link-arg=-ldeko3d -C link-arg=-lnx -C link-arg=-lm
"$DEVKITPRO/tools/bin/nacptool" --create 'CMVS Switch' 'cmvs-rs' '0.1.0' platforms/switch/build/cmvs-switch.nacp
"$DEVKITPRO/tools/bin/elf2nro" target/aarch64-switch/release/cmvs-switch platforms/switch/build/cmvs-switch.nro --nacp=platforms/switch/build/cmvs-switch.nacp
