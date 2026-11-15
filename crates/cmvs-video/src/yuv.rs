//! BT.601 limited-range YUV420-to-RGBA row conversion kernels.

/// Converts one cropped YUV420 scanline into RGBA pixels.
///
/// When `odd_left` is `true`, the row starts at the second luma pixel of a
/// horizontally subsampled chroma pair, so pixel 0 uses `cb[0]`/`cr[0]` and
/// subsequent pairs advance from `cb[1..]`/`cr[1..]`.
pub(crate) fn convert_row(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8], odd_left: bool) {
    assert_eq!(out.len(), luma.len() * 4);
    let (head, luma, out) = if odd_left {
        let Some((&y0, rest_luma)) = luma.split_first() else {
            return;
        };
        let (first, rest_out) = out.split_at_mut(4);
        first.copy_from_slice(&ycbcr(y0, cb[0], cr[0]));
        (1, rest_luma, rest_out)
    } else {
        (0, luma, out)
    };
    let (cb, cr) = (&cb[head..], &cr[head..]);
    convert_pairs(luma, cb, cr, out);
}

fn convert_pairs(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8]) {
    let pairs = luma.len() / 2;
    let tail_pixel = luma.len() % 2;
    assert!(out.len() >= luma.len() * 4);
    assert!(cb.len() >= pairs + tail_pixel);
    assert!(cr.len() >= pairs + tail_pixel);

    let chunks = pairs / 8;
    if chunks > 0 {
        let simd_pairs = chunks * 8;
        let simd_luma = &luma[..simd_pairs * 2];
        let simd_cb = &cb[..simd_pairs];
        let simd_cr = &cr[..simd_pairs];
        let simd_out = &mut out[..simd_pairs * 8];
        #[cfg(target_arch = "x86_64")]
        // SAFETY: SSE2 is baseline on x86-64. The checked slices cover
        // `chunks * 16` luma bytes, `chunks * 8` chroma bytes per plane, and
        // `chunks * 64` exclusive output bytes.
        #[allow(unsafe_code)]
        unsafe {
            x86::convert_chunks(
                simd_luma.as_ptr(),
                simd_cb.as_ptr(),
                simd_cr.as_ptr(),
                simd_out.as_mut_ptr(),
                chunks,
            );
        }
        #[cfg(target_arch = "aarch64")]
        // SAFETY: Advanced SIMD is enabled by supported AArch64 targets. The
        // checked slices cover `chunks * 16` luma bytes, `chunks * 8` chroma
        // bytes per plane, and `chunks * 64` exclusive output bytes.
        #[allow(unsafe_code)]
        unsafe {
            neon::convert_chunks(
                simd_luma.as_ptr(),
                simd_cb.as_ptr(),
                simd_cr.as_ptr(),
                simd_out.as_mut_ptr(),
                chunks,
            );
        }
        #[cfg(target_os = "vita")]
        // SAFETY: PS Vita links `platforms/psvita/runtime/yuv.c` (Cortex-A9
        // NEON). The checked slices cover `chunks * 16` luma bytes,
        // `chunks * 8` chroma bytes per plane, and `chunks * 64` exclusive
        // output bytes.
        #[allow(unsafe_code)]
        unsafe {
            vita_neon::convert_chunks(
                simd_luma.as_ptr(),
                simd_cb.as_ptr(),
                simd_cr.as_ptr(),
                simd_out.as_mut_ptr(),
                chunks,
            );
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_os = "vita")))]
        convert_pairs_scalar(simd_luma, simd_cb, simd_cr, simd_out);
    }

    let done_pairs = chunks * 8;
    convert_pairs_scalar(
        &luma[done_pairs * 2..],
        &cb[done_pairs..],
        &cr[done_pairs..],
        &mut out[done_pairs * 8..],
    );
}

fn convert_pairs_scalar(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8]) {
    let pairs = luma.len() / 2;
    for (((y2, rgba), &u), &v) in luma
        .as_chunks::<2>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<8>().0)
        .zip(cb)
        .zip(cr)
    {
        rgba[..4].copy_from_slice(&ycbcr(y2[0], u, v));
        rgba[4..].copy_from_slice(&ycbcr(y2[1], u, v));
    }
    if let Some(&last) = luma.get(pairs * 2) {
        out[pairs * 8..pairs * 8 + 4].copy_from_slice(&ycbcr(last, cb[pairs], cr[pairs]));
    }
}

/// BT.601 limited-range conversion; the decoded Y/Cb/Cr samples are retained exactly.
#[inline(always)]
fn ycbcr(y: u8, cb: u8, cr: u8) -> [u8; 4] {
    let luma = 298 * (i32::from(y) - 16);
    let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
    let clip = |v: i32| ((v + 128) >> 8).clamp(0, 255) as u8;
    [
        clip(luma + 409 * cr),
        clip(luma - 100 * cb - 208 * cr),
        clip(luma + 516 * cb),
        255,
    ]
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod x86 {
    use std::arch::x86_64::*;

    const fn pair_i16(even: i16, odd: i16) -> i32 {
        ((even as u16 as u32) | ((odd as u16 as u32) << 16)) as i32
    }

    /// # Safety
    /// Caller guarantees SSE2 and that `luma` covers `chunks * 16` bytes, `cb`
    /// and `cr` each cover `chunks * 8` bytes, and `out` covers `chunks * 64`
    /// writable non-aliasing bytes.
    pub(super) unsafe fn convert_chunks(
        mut luma: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: caller guarantees valid non-aliasing spans for `chunks` iterations.
        unsafe {
            let zero = _mm_setzero_si128();
            let ones = _mm_set1_epi16(1);
            let c128 = _mm_set1_epi16(128);
            let alpha = _mm_set1_epi8(-1);
            // Fold 298 * (-16) + 128 = -4640 into each pixel's luma dot product.
            let k_y = _mm_set1_epi32(pair_i16(298, -4640));
            let k_r = _mm_set1_epi32(pair_i16(0, 409));
            let k_g = _mm_set1_epi32(pair_i16(-100, -208));
            let k_b = _mm_set1_epi32(pair_i16(516, 0));

            for _ in 0..chunks {
                let vy = _mm_loadu_si128(luma.cast());
                let y_lo = _mm_unpacklo_epi8(vy, zero);
                let y_hi = _mm_unpackhi_epi8(vy, zero);
                let l0 = _mm_madd_epi16(_mm_unpacklo_epi16(y_lo, ones), k_y);
                let l1 = _mm_madd_epi16(_mm_unpackhi_epi16(y_lo, ones), k_y);
                let l2 = _mm_madd_epi16(_mm_unpacklo_epi16(y_hi, ones), k_y);
                let l3 = _mm_madd_epi16(_mm_unpackhi_epi16(y_hi, ones), k_y);

                let cb16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cb.cast()), zero), c128);
                let cr16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cr.cast()), zero), c128);
                let uv_lo = _mm_unpacklo_epi16(cb16, cr16);
                let uv_hi = _mm_unpackhi_epi16(cb16, cr16);

                let r_ch_03 = _mm_madd_epi16(uv_lo, k_r);
                let g_ch_03 = _mm_madd_epi16(uv_lo, k_g);
                let b_ch_03 = _mm_madd_epi16(uv_lo, k_b);
                let r_ch_47 = _mm_madd_epi16(uv_hi, k_r);
                let g_ch_47 = _mm_madd_epi16(uv_hi, k_g);
                let b_ch_47 = _mm_madd_epi16(uv_hi, k_b);

                let channel = |ch_03: __m128i, ch_47: __m128i| {
                    let s0 = _mm_srai_epi32(_mm_add_epi32(l0, _mm_unpacklo_epi32(ch_03, ch_03)), 8);
                    let s1 = _mm_srai_epi32(_mm_add_epi32(l1, _mm_unpackhi_epi32(ch_03, ch_03)), 8);
                    let s2 = _mm_srai_epi32(_mm_add_epi32(l2, _mm_unpacklo_epi32(ch_47, ch_47)), 8);
                    let s3 = _mm_srai_epi32(_mm_add_epi32(l3, _mm_unpackhi_epi32(ch_47, ch_47)), 8);
                    _mm_packus_epi16(_mm_packs_epi32(s0, s1), _mm_packs_epi32(s2, s3))
                };

                let r8 = channel(r_ch_03, r_ch_47);
                let g8 = channel(g_ch_03, g_ch_47);
                let b8 = channel(b_ch_03, b_ch_47);

                let rg_lo = _mm_unpacklo_epi8(r8, g8);
                let rg_hi = _mm_unpackhi_epi8(r8, g8);
                let ba_lo = _mm_unpacklo_epi8(b8, alpha);
                let ba_hi = _mm_unpackhi_epi8(b8, alpha);

                _mm_storeu_si128(out.cast(), _mm_unpacklo_epi16(rg_lo, ba_lo));
                _mm_storeu_si128(out.add(16).cast(), _mm_unpackhi_epi16(rg_lo, ba_lo));
                _mm_storeu_si128(out.add(32).cast(), _mm_unpacklo_epi16(rg_hi, ba_hi));
                _mm_storeu_si128(out.add(48).cast(), _mm_unpackhi_epi16(rg_hi, ba_hi));

                luma = luma.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out = out.add(64);
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon {
    use std::arch::aarch64::*;

    /// # Safety
    /// Caller guarantees NEON and that `luma` covers `chunks * 16` bytes, `cb`
    /// and `cr` each cover `chunks * 8` bytes, and `out` covers `chunks * 64`
    /// writable non-aliasing bytes.
    pub(super) unsafe fn convert_chunks(
        mut luma: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: caller guarantees valid non-aliasing spans for `chunks` iterations.
        unsafe {
            let c298 = vdupq_n_s16(298);
            let bias = vdupq_n_s32(-4640);
            let c128 = vdupq_n_s16(128);
            let c409 = vdupq_n_s16(409);
            let c_neg100 = vdupq_n_s16(-100);
            let c208 = vdupq_n_s16(208);
            let c516 = vdupq_n_s16(516);
            let alpha = vdupq_n_u8(255);

            for _ in 0..chunks {
                let vy = vld1q_u8(luma);
                let y_lo = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(vy)));
                let y_hi = vreinterpretq_s16_u16(vmovl_high_u8(vy));
                let l0 = vaddq_s32(vmull_s16(vget_low_s16(y_lo), vget_low_s16(c298)), bias);
                let l1 = vaddq_s32(vmull_high_s16(y_lo, c298), bias);
                let l2 = vaddq_s32(vmull_s16(vget_low_s16(y_hi), vget_low_s16(c298)), bias);
                let l3 = vaddq_s32(vmull_high_s16(y_hi, c298), bias);

                let cb16 = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vld1_u8(cb))), c128);
                let cr16 = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vld1_u8(cr))), c128);

                let cb_lo = vget_low_s16(cb16);
                let cr_lo = vget_low_s16(cr16);
                let r_ch_03 = vmull_s16(cr_lo, vget_low_s16(c409));
                let g_ch_03 = vmlsl_s16(
                    vmull_s16(cb_lo, vget_low_s16(c_neg100)),
                    cr_lo,
                    vget_low_s16(c208),
                );
                let b_ch_03 = vmull_s16(cb_lo, vget_low_s16(c516));

                let r_ch_47 = vmull_high_s16(cr16, c409);
                let g_ch_47 = vmlsl_high_s16(vmull_high_s16(cb16, c_neg100), cr16, c208);
                let b_ch_47 = vmull_high_s16(cb16, c516);

                let channel = |ch_03: int32x4_t, ch_47: int32x4_t| {
                    let s0 = vshrq_n_s32::<8>(vaddq_s32(l0, vzip1q_s32(ch_03, ch_03)));
                    let s1 = vshrq_n_s32::<8>(vaddq_s32(l1, vzip2q_s32(ch_03, ch_03)));
                    let s2 = vshrq_n_s32::<8>(vaddq_s32(l2, vzip1q_s32(ch_47, ch_47)));
                    let s3 = vshrq_n_s32::<8>(vaddq_s32(l3, vzip2q_s32(ch_47, ch_47)));
                    let n_lo = vcombine_s16(vmovn_s32(s0), vmovn_s32(s1));
                    let n_hi = vcombine_s16(vmovn_s32(s2), vmovn_s32(s3));
                    vcombine_u8(vqmovun_s16(n_lo), vqmovun_s16(n_hi))
                };

                let r = channel(r_ch_03, r_ch_47);
                let g = channel(g_ch_03, g_ch_47);
                let b = channel(b_ch_03, b_ch_47);
                vst4q_u8(out, uint8x16x4_t(r, g, b, alpha));

                luma = luma.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out = out.add(64);
            }
        }
    }
}

#[cfg(target_os = "vita")]
#[allow(unsafe_code)]
mod vita_neon {
    unsafe extern "C" {
        fn cmvs_vita_yuv420_to_rgba_chunks(
            luma: *const u8,
            cb: *const u8,
            cr: *const u8,
            out: *mut u8,
            chunks: usize,
        );
    }

    /// # Safety
    /// Caller guarantees `luma` covers `chunks * 16` bytes, `cb` and `cr` each
    /// cover `chunks * 8` bytes, and `out` covers `chunks * 64` writable
    /// non-aliasing bytes.
    pub(super) unsafe fn convert_chunks(
        luma: *const u8,
        cb: *const u8,
        cr: *const u8,
        out: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: forwarded to `platforms/psvita/runtime/yuv.c` with checked
        // non-aliasing spans.
        unsafe {
            cmvs_vita_yuv420_to_rgba_chunks(luma, cb, cr, out, chunks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_row_scalar(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8], odd_left: bool) {
        let (head, luma, out) = if odd_left {
            let Some((&y0, rest_luma)) = luma.split_first() else {
                return;
            };
            let (first, rest_out) = out.split_at_mut(4);
            first.copy_from_slice(&ycbcr(y0, cb[0], cr[0]));
            (1, rest_luma, rest_out)
        } else {
            (0, luma, out)
        };
        convert_pairs_scalar(luma, &cb[head..], &cr[head..], out);
    }

    #[test]
    fn row_conversion_matches_scalar_across_widths_and_alignments() {
        let mut seed = 0x51a7_9b03u32;
        let mut next_u8 = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        };

        for odd_left in [false, true] {
            for width in 0usize..=40 {
                let chroma_len = if odd_left {
                    usize::from(width > 0) + width.saturating_sub(1).div_ceil(2)
                } else {
                    width.div_ceil(2)
                };
                for offset in 0..16 {
                    let luma: Vec<u8> = (0..offset + width).map(|_| next_u8()).collect();
                    let cb: Vec<u8> = (0..offset + chroma_len).map(|_| next_u8()).collect();
                    let cr: Vec<u8> = (0..offset + chroma_len).map(|_| next_u8()).collect();

                    let mut got = vec![0x5a; offset + width * 4];
                    let mut want = got.clone();
                    convert_row(
                        &luma[offset..],
                        &cb[offset..],
                        &cr[offset..],
                        &mut got[offset..],
                        odd_left,
                    );
                    convert_row_scalar(
                        &luma[offset..],
                        &cb[offset..],
                        &cr[offset..],
                        &mut want[offset..],
                        odd_left,
                    );
                    assert_eq!(
                        got, want,
                        "odd_left={odd_left} width={width} offset={offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn simd_matches_scalar_at_saturation_and_rounding_boundaries() {
        let ys = [0, 1, 15, 16, 17, 127, 128, 235, 254, 255];
        let chromas = [0, 1, 64, 127, 128, 129, 192, 254, 255];
        let mut luma = [0u8; 16];
        let mut cb = [0u8; 8];
        let mut cr = [0u8; 8];

        let mut count = 0usize;
        for &y in &ys {
            for &u in &chromas {
                for &v in &chromas {
                    let slot = count % 8;
                    count += 1;
                    luma[slot * 2] = y;
                    luma[slot * 2 + 1] = y.wrapping_add(13);
                    cb[slot] = u;
                    cr[slot] = v;
                    if slot == 7 {
                        let mut got = [0u8; 64];
                        let mut want = [0u8; 64];
                        convert_pairs(&luma, &cb, &cr, &mut got);
                        convert_pairs_scalar(&luma, &cb, &cr, &mut want);
                        assert_eq!(got, want);
                    }
                }
            }
        }
        if !count.is_multiple_of(8) {
            let mut got = [0u8; 64];
            let mut want = [0u8; 64];
            convert_pairs(&luma, &cb, &cr, &mut got);
            convert_pairs_scalar(&luma, &cb, &cr, &mut want);
            assert_eq!(got, want);
        }

        let mut got = [0u8; 64];
        let mut want = [0u8; 64];
        for (i, pair) in luma.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            pair[0] = ys[i % ys.len()];
            pair[1] = 255 - ys[i % ys.len()];
            cb[i] = chromas[i % chromas.len()];
            cr[i] = chromas[(i * 3) % chromas.len()];
        }
        convert_pairs(&luma, &cb, &cr, &mut got);
        convert_pairs_scalar(&luma, &cb, &cr, &mut want);
        assert_eq!(got, want);
    }

    #[test]
    #[should_panic]
    fn convert_pairs_rejects_short_chroma_before_simd() {
        let mut out = [0u8; 64];
        convert_pairs(&[0; 16], &[0; 7], &[0; 8], &mut out);
    }

    #[test]
    #[should_panic]
    fn convert_pairs_rejects_short_output_before_simd() {
        let mut out = [0u8; 63];
        convert_pairs(&[0; 16], &[0; 8], &[0; 8], &mut out);
    }
}
