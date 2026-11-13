#!/bin/sh
# Builds platforms/psvita/build/cmvs-vita.vpk inside the cmvs-vita-build image.
set -eu
cd "$(dirname "$0")/../../.."
out=platforms/psvita/build
mkdir -p "$out/obj"
for unit in window gpu audio system; do
    arm-vita-eabi-gcc -c "platforms/psvita/runtime/$unit.c" -o "$out/obj/$unit.o" \
        -Wall -Wextra -Werror -O2 -mcpu=cortex-a9 -mfpu=neon -mfloat-abi=hard
done
arm-vita-eabi-gcc -c platforms/psvita/runtime/shaders.S -o "$out/obj/shaders.o"
link=""
for unit in window gpu shaders audio system; do
    link="$link -C link-arg=$PWD/$out/obj/$unit.o"
done
for lib in vita2d SceGxm_stub SceDisplay_stub SceCtrl_stub SceTouch_stub SceAudio_stub \
    SceAppMgr_stub SceAppUtil_stub SceCommonDialog_stub ScePower_stub SceSysmodule_stub m; do
    link="$link -C link-arg=-l$lib"
done
# vita-elf-create appends SCE metadata to the text segment; without headroom it fails
# whenever the text happens to end just below the 64 KiB-aligned data segment.
link="$link -C link-arg=-Wl,--defsym=__sce_headroom=0x4000"
# shellcheck disable=SC2086 # one word per linker argument
# Whole-program optimization: slower to build, a few percent faster in the VM and decoders.
cargo +nightly-2026-01-20 rustc --config 'profile.release.panic="abort"' \
    --config 'profile.release.lto="fat"' --config 'profile.release.codegen-units=1' -p cmvs-vita --release \
    --target armv7-sony-vita-newlibeabihf -Zbuild-std=std,panic_abort -- $link
elf=target/armv7-sony-vita-newlibeabihf/release/cmvs-vita.elf
vita-elf-create "$elf" "$out/obj/cmvs-vita.velf"
vita-make-fself -c "$out/obj/cmvs-vita.velf" "$out/obj/eboot.bin"
# ATTRIBUTE2=12 grants the extended application memory budget the heap relies on.
vita-mksfoex -s TITLE_ID=CMVS00001 -s APP_VER="${CMVS_APP_VER:-00.10}" -d ATTRIBUTE2=12 'CMVS' "$out/obj/param.sfo"
python3 platforms/psvita/scripts/livearea.py "$out/obj/sce_sys"
vita-pack-vpk -s "$out/obj/param.sfo" -b "$out/obj/eboot.bin" \
    -a "$out/obj/sce_sys/icon0.png=sce_sys/icon0.png" \
    -a "$out/obj/sce_sys/livearea/contents/bg.png=sce_sys/livearea/contents/bg.png" \
    -a "$out/obj/sce_sys/livearea/contents/startup.png=sce_sys/livearea/contents/startup.png" \
    -a "$out/obj/sce_sys/livearea/contents/template.xml=sce_sys/livearea/contents/template.xml" \
    "$out/cmvs-vita.vpk"
