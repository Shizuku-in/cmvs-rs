/* GXP programs compiled offline from platforms/psvita/shaders (see the README there),
   paths relative to the repository root where the build runs. */
    .section .rodata.cmvs_shaders, "a"
#define SHADER(name, file) \
    .balign 16; .global name; name: .incbin file
SHADER(cmvs_scene_v, "platforms/psvita/shaders/scene_v.gxp")
SHADER(cmvs_scene_f, "platforms/psvita/shaders/scene_f.gxp")
