/* Shader modules compiled by uam in scripts/build-player.sh, paths relative to the
   repository root where the build runs. */
    .section .rodata.cmvs_shaders, "a"
    .balign 256
#define SHADER(name, file) \
    .global name; name: .incbin file; .global name##_end; name##_end:; .balign 256
SHADER(cmvs_scene_vsh, "platforms/switch/build/scene_vsh.dksh")
SHADER(cmvs_scene_fsh, "platforms/switch/build/scene_fsh.dksh")
SHADER(cmvs_present_vsh, "platforms/switch/build/present_vsh.dksh")
SHADER(cmvs_present_fsh, "platforms/switch/build/present_fsh.dksh")
