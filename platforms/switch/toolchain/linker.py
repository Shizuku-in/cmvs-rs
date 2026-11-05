#!/usr/bin/env python3
"""Let libnx resolve OS symbols before newlib, matching devkitPro's C link."""
import os
import sys
args = [a for a in sys.argv[1:] if a not in ('-lc', '-lm', '-lnx', '-nodefaultlibs', '-no-pie', '-Wl,--eh-frame-hdr')]
devkit = os.environ.get('DEVKITA64', '/opt/devkitpro/devkitA64')
os.execv(devkit + '/bin/aarch64-none-elf-gcc', ['aarch64-none-elf-gcc', '-fPIE', *args, '-Wl,--start-group', '-lnx', '-lc', '-lm', '-lgcc', '-Wl,--end-group'])
