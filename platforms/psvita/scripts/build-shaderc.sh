#!/bin/sh
# Shader compiler VPK (CMVS00098) for platforms/psvita/shaders; see the README there.
set -eu
cd "$(dirname "$0")/../../.."
out=platforms/psvita/build/shaderc
mkdir -p "$out"
arm-vita-eabi-gcc platforms/psvita/shaderc/main.c -o "$out/shaderc.elf" -Wall -Wextra -Werror -O2 \
    -Wl,-q -lvitashark -lSceShaccCgExt -lSceShaccCg_stub -ltaihen_stub -lSceKernelModulemgr_stub \
    -lSceSblSsMgr_stub
vita-elf-create "$out/shaderc.elf" "$out/shaderc.velf"
vita-make-fself -c "$out/shaderc.velf" "$out/eboot.bin"
vita-mksfoex -s TITLE_ID=CMVS00098 'CMVS shaderc' "$out/param.sfo"
vita-pack-vpk -s "$out/param.sfo" -b "$out/eboot.bin" platforms/psvita/build/cmvs-vita-shaderc.vpk
