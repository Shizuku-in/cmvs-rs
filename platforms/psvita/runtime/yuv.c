#include <arm_neon.h>
#include <stddef.h>
#include <stdint.h>

static inline void convert_half(uint8x8_t vy, int16x4_t cb4, int16x4_t cr4, int16x4_t c298,
                                int32x4_t bias, int16x4_t c409, int16x4_t c_neg100, int16x4_t c208,
                                int16x4_t c516, uint8x8_t alpha, uint8_t *out) {
    int16x8_t y16 = vreinterpretq_s16_u16(vmovl_u8(vy));
    int32x4_t l0 = vmlal_s16(bias, vget_low_s16(y16), c298);
    int32x4_t l1 = vmlal_s16(bias, vget_high_s16(y16), c298);

    int32x4_t r_ch = vmull_s16(cr4, c409);
    int32x4_t g_ch = vmlsl_s16(vmull_s16(cb4, c_neg100), cr4, c208);
    int32x4_t b_ch = vmull_s16(cb4, c516);

    int32x4x2_t r_dup = vzipq_s32(r_ch, r_ch);
    int32x4x2_t g_dup = vzipq_s32(g_ch, g_ch);
    int32x4x2_t b_dup = vzipq_s32(b_ch, b_ch);

    uint8x8x4_t rgba;
    rgba.val[0] = vqmovun_s16(vcombine_s16(vshrn_n_s32(vaddq_s32(l0, r_dup.val[0]), 8),
                                           vshrn_n_s32(vaddq_s32(l1, r_dup.val[1]), 8)));
    rgba.val[1] = vqmovun_s16(vcombine_s16(vshrn_n_s32(vaddq_s32(l0, g_dup.val[0]), 8),
                                           vshrn_n_s32(vaddq_s32(l1, g_dup.val[1]), 8)));
    rgba.val[2] = vqmovun_s16(vcombine_s16(vshrn_n_s32(vaddq_s32(l0, b_dup.val[0]), 8),
                                           vshrn_n_s32(vaddq_s32(l1, b_dup.val[1]), 8)));
    rgba.val[3] = alpha;
    vst4_u8(out, rgba);
}

/* Converts `chunks * 8` chroma-shared luma pairs (`chunks * 16` luma bytes,
   `chunks * 8` cb/cr bytes) into `chunks * 64` BT.601 limited-range RGBA bytes.
   Caller guarantees non-aliasing valid spans for all `chunks` iterations. */
void cmvs_vita_yuv420_to_rgba_chunks(const uint8_t *luma, const uint8_t *cb, const uint8_t *cr,
                                     uint8_t *out, size_t chunks) {
    const int16x4_t c298 = vdup_n_s16(298);
    /* Fold 298 * (-16) + 128 = -4640 into each pixel's luma accumulator. */
    const int32x4_t bias = vdupq_n_s32(-4640);
    const int16x8_t c128 = vdupq_n_s16(128);
    const int16x4_t c409 = vdup_n_s16(409);
    const int16x4_t c_neg100 = vdup_n_s16(-100);
    const int16x4_t c208 = vdup_n_s16(208);
    const int16x4_t c516 = vdup_n_s16(516);
    const uint8x8_t alpha = vdup_n_u8(255);

    for (size_t i = 0; i < chunks; ++i) {
        uint8x16_t vy = vld1q_u8(luma);
        int16x8_t cb16 = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vld1_u8(cb))), c128);
        int16x8_t cr16 = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vld1_u8(cr))), c128);

        convert_half(vget_low_u8(vy), vget_low_s16(cb16), vget_low_s16(cr16), c298, bias, c409,
                     c_neg100, c208, c516, alpha, out);
        convert_half(vget_high_u8(vy), vget_high_s16(cb16), vget_high_s16(cr16), c298, bias, c409,
                     c_neg100, c208, c516, alpha, out + 32);

        luma += 16;
        cb += 8;
        cr += 8;
        out += 64;
    }
}
