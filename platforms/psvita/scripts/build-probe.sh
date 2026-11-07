#!/bin/sh
# Toolchain probe VPK: std and core on the device, results in ux0:data/cmvs-rs/probe.log.
set -eu
cd "$(dirname "$0")/../../.."
out=platforms/psvita/build/probe
mkdir -p "$out"
arm-vita-eabi-gcc -c platforms/psvita/runtime/system.c -o "$out/system.o" -Wall -Wextra -Werror -O2
cargo +nightly-2026-01-20 rustc --config 'profile.release.panic="abort"' -p cmvs-vita-probe --release --target armv7-sony-vita-newlibeabihf -Zbuild-std=std,panic_abort -- \
    -C "link-arg=$PWD/$out/system.o" -C link-arg=-lScePower_stub
elf=target/armv7-sony-vita-newlibeabihf/release/cmvs-vita-probe.elf
vita-elf-create "$elf" "$out/probe.velf"
vita-make-fself -c "$out/probe.velf" "$out/eboot.bin"
vita-mksfoex -s TITLE_ID=CMVS00099 -d ATTRIBUTE2=12 'CMVS probe' "$out/param.sfo"
vita-pack-vpk -s "$out/param.sfo" -b "$out/eboot.bin" platforms/psvita/build/cmvs-vita-probe.vpk
