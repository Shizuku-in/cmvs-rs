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

/// Converts two luma scanlines sharing a single chroma line into two RGBA rows.
pub(crate) fn convert_row_pair(
    luma0: &[u8],
    luma1: &[u8],
    cb: &[u8],
    cr: &[u8],
    out0: &mut [u8],
    out1: &mut [u8],
    odd_left: bool,
) {
    let width = luma0.len();
    assert_eq!(luma1.len(), width);
    assert_eq!(out0.len(), width * 4);
    assert_eq!(out1.len(), width * 4);
    let (head, luma0, luma1, out0, out1) = if odd_left {
        let Some(((&y0, rest_l0), (&y1, rest_l1))) = luma0.split_first().zip(luma1.split_first())
        else {
            return;
        };
        let (first0, rest_o0) = out0.split_at_mut(4);
        let (first1, rest_o1) = out1.split_at_mut(4);
        let chroma = chroma_terms(cb[0], cr[0]);
        first0.copy_from_slice(&luma_rgba(y0, chroma));
        first1.copy_from_slice(&luma_rgba(y1, chroma));
        (1, rest_l0, rest_l1, rest_o0, rest_o1)
    } else {
        (0, luma0, luma1, out0, out1)
    };
    let (cb, cr) = (&cb[head..], &cr[head..]);
    let pairs = luma0.len() / 2;
    let tail_pixel = luma0.len() % 2;
    assert!(cb.len() >= pairs + tail_pixel);
    assert!(cr.len() >= pairs + tail_pixel);

    #[cfg(target_arch = "x86_64")]
    {
        let mut done_pairs = 0;
        if is_x86_feature_detected!("avx2") {
            let avx_chunks = pairs / 16;
            if avx_chunks > 0 {
                let avx_pairs = avx_chunks * 16;
                // SAFETY: `is_x86_feature_detected!("avx2")` verified AVX2 support.
                // Checked slices cover `avx_chunks * 32` luma bytes per row,
                // `avx_chunks * 16` chroma bytes per plane, and `avx_chunks * 128`
                // writable non-aliasing output bytes per row.
                #[allow(unsafe_code)]
                unsafe {
                    x86::convert_pair_chunks_avx2(
                        luma0[..avx_pairs * 2].as_ptr(),
                        luma1[..avx_pairs * 2].as_ptr(),
                        cb[..avx_pairs].as_ptr(),
                        cr[..avx_pairs].as_ptr(),
                        out0[..avx_pairs * 8].as_mut_ptr(),
                        out1[..avx_pairs * 8].as_mut_ptr(),
                        avx_chunks,
                    );
                }
                done_pairs = avx_pairs;
            }
        }
        let sse_chunks = (pairs - done_pairs) / 8;
        if sse_chunks > 0 {
            let sse_pairs = sse_chunks * 8;
            // SAFETY: SSE2 is baseline on x86-64. Checked slices cover
            // `sse_chunks * 16` luma bytes per row, `sse_chunks * 8` chroma
            // bytes per plane, and `sse_chunks * 64` writable non-aliasing
            // output bytes per row.
            #[allow(unsafe_code)]
            unsafe {
                x86::convert_pair_chunks(
                    luma0[done_pairs * 2..][..sse_pairs * 2].as_ptr(),
                    luma1[done_pairs * 2..][..sse_pairs * 2].as_ptr(),
                    cb[done_pairs..][..sse_pairs].as_ptr(),
                    cr[done_pairs..][..sse_pairs].as_ptr(),
                    out0[done_pairs * 8..][..sse_pairs * 8].as_mut_ptr(),
                    out1[done_pairs * 8..][..sse_pairs * 8].as_mut_ptr(),
                    sse_chunks,
                );
            }
            done_pairs += sse_pairs;
        }
        if done_pairs * 2 < luma0.len() {
            convert_pair_scalar(
                &luma0[done_pairs * 2..],
                &luma1[done_pairs * 2..],
                &cb[done_pairs..],
                &cr[done_pairs..],
                &mut out0[done_pairs * 8..],
                &mut out1[done_pairs * 8..],
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        let chunks = pairs / 8;
        if chunks > 0 {
            let simd_pairs = chunks * 8;
            // SAFETY: Advanced SIMD is enabled by supported AArch64 targets.
            // Checked slices cover `chunks * 16` luma bytes per row,
            // `chunks * 8` chroma bytes per plane, and `chunks * 64`
            // writable non-aliasing output bytes per row.
            #[allow(unsafe_code)]
            unsafe {
                neon::convert_pair_chunks(
                    luma0[..simd_pairs * 2].as_ptr(),
                    luma1[..simd_pairs * 2].as_ptr(),
                    cb[..simd_pairs].as_ptr(),
                    cr[..simd_pairs].as_ptr(),
                    out0[..simd_pairs * 8].as_mut_ptr(),
                    out1[..simd_pairs * 8].as_mut_ptr(),
                    chunks,
                );
            }
        }
        let done_pairs = chunks * 8;
        if done_pairs * 2 < luma0.len() {
            convert_pair_scalar(
                &luma0[done_pairs * 2..],
                &luma1[done_pairs * 2..],
                &cb[done_pairs..],
                &cr[done_pairs..],
                &mut out0[done_pairs * 8..],
                &mut out1[done_pairs * 8..],
            );
        }
    }

    #[cfg(target_os = "vita")]
    {
        let chunks = pairs / 8;
        if chunks > 0 {
            let simd_pairs = chunks * 8;
            // SAFETY: PS Vita links `platforms/psvita/runtime/yuv.c` (Cortex-A9
            // NEON). Checked slices cover `chunks * 16` luma bytes per row,
            // `chunks * 8` chroma bytes per plane, and `chunks * 64` exclusive
            // output bytes per row.
            #[allow(unsafe_code)]
            unsafe {
                vita_neon::convert_chunks(
                    luma0[..simd_pairs * 2].as_ptr(),
                    cb[..simd_pairs].as_ptr(),
                    cr[..simd_pairs].as_ptr(),
                    out0[..simd_pairs * 8].as_mut_ptr(),
                    chunks,
                );
                vita_neon::convert_chunks(
                    luma1[..simd_pairs * 2].as_ptr(),
                    cb[..simd_pairs].as_ptr(),
                    cr[..simd_pairs].as_ptr(),
                    out1[..simd_pairs * 8].as_mut_ptr(),
                    chunks,
                );
            }
        }
        let done_pairs = chunks * 8;
        if done_pairs * 2 < luma0.len() {
            convert_pair_scalar(
                &luma0[done_pairs * 2..],
                &luma1[done_pairs * 2..],
                &cb[done_pairs..],
                &cr[done_pairs..],
                &mut out0[done_pairs * 8..],
                &mut out1[done_pairs * 8..],
            );
        }
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_os = "vita")))]
    convert_pair_scalar(luma0, luma1, cb, cr, out0, out1);
}

fn convert_pairs(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8]) {
    let pairs = luma.len() / 2;
    let tail_pixel = luma.len() % 2;
    assert!(out.len() >= luma.len() * 4);
    assert!(cb.len() >= pairs + tail_pixel);
    assert!(cr.len() >= pairs + tail_pixel);

    #[cfg(target_arch = "x86_64")]
    {
        let mut done_pairs = 0;
        if is_x86_feature_detected!("avx2") {
            let avx_chunks = pairs / 16;
            if avx_chunks > 0 {
                let avx_pairs = avx_chunks * 16;
                // SAFETY: `is_x86_feature_detected!("avx2")` verified AVX2 support.
                // Checked slices cover `avx_chunks * 32` luma bytes, `avx_chunks * 16`
                // chroma bytes per plane, and `avx_chunks * 128` exclusive output bytes.
                #[allow(unsafe_code)]
                unsafe {
                    x86::convert_chunks_avx2(
                        luma[..avx_pairs * 2].as_ptr(),
                        cb[..avx_pairs].as_ptr(),
                        cr[..avx_pairs].as_ptr(),
                        out[..avx_pairs * 8].as_mut_ptr(),
                        avx_chunks,
                    );
                }
                done_pairs = avx_pairs;
            }
        }
        let sse_chunks = (pairs - done_pairs) / 8;
        if sse_chunks > 0 {
            let sse_pairs = sse_chunks * 8;
            // SAFETY: SSE2 is baseline on x86-64. Checked slices cover
            // `sse_chunks * 16` luma bytes, `sse_chunks * 8` chroma bytes per
            // plane, and `sse_chunks * 64` exclusive output bytes.
            #[allow(unsafe_code)]
            unsafe {
                x86::convert_chunks(
                    luma[done_pairs * 2..][..sse_pairs * 2].as_ptr(),
                    cb[done_pairs..][..sse_pairs].as_ptr(),
                    cr[done_pairs..][..sse_pairs].as_ptr(),
                    out[done_pairs * 8..][..sse_pairs * 8].as_mut_ptr(),
                    sse_chunks,
                );
            }
            done_pairs += sse_pairs;
        }
        convert_pairs_scalar(
            &luma[done_pairs * 2..],
            &cb[done_pairs..],
            &cr[done_pairs..],
            &mut out[done_pairs * 8..],
        );
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let chunks = pairs / 8;
        if chunks > 0 {
            let simd_pairs = chunks * 8;
            let simd_luma = &luma[..simd_pairs * 2];
            let simd_cb = &cb[..simd_pairs];
            let simd_cr = &cr[..simd_pairs];
            let simd_out = &mut out[..simd_pairs * 8];
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
            #[cfg(not(any(target_arch = "aarch64", target_os = "vita")))]
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
}

fn convert_pairs_scalar(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8]) {
    let pairs = luma.len() / 2;
    let (l_pairs, l_rem) = luma.as_chunks::<2>();
    let (o_pairs, o_rem) = out.as_chunks_mut::<8>();
    for (((y2, rgba), &u), &v) in l_pairs
        .iter()
        .zip(o_pairs)
        .zip(&cb[..pairs])
        .zip(&cr[..pairs])
    {
        let chroma = chroma_terms(u, v);
        rgba[..4].copy_from_slice(&luma_rgba(y2[0], chroma));
        rgba[4..].copy_from_slice(&luma_rgba(y2[1], chroma));
    }
    if let Some(&last) = l_rem.first() {
        o_rem[..4].copy_from_slice(&luma_rgba(last, chroma_terms(cb[pairs], cr[pairs])));
    }
}

/// Converts two luma rows sharing one chroma row in 2x2 pixel blocks so each
/// `(cb, cr)` pair is decoded once across all four luma samples.
fn convert_pair_scalar(
    luma0: &[u8],
    luma1: &[u8],
    cb: &[u8],
    cr: &[u8],
    out0: &mut [u8],
    out1: &mut [u8],
) {
    let pairs = luma0.len() / 2;
    let (l0_pairs, l0_rem) = luma0.as_chunks::<2>();
    let (l1_pairs, l1_rem) = luma1.as_chunks::<2>();
    let (o0_pairs, o0_rem) = out0.as_chunks_mut::<8>();
    let (o1_pairs, o1_rem) = out1.as_chunks_mut::<8>();
    for (((((y0, y1), r0), r1), &u), &v) in l0_pairs
        .iter()
        .zip(l1_pairs)
        .zip(o0_pairs)
        .zip(o1_pairs)
        .zip(&cb[..pairs])
        .zip(&cr[..pairs])
    {
        let chroma = chroma_terms(u, v);
        r0[..4].copy_from_slice(&luma_rgba(y0[0], chroma));
        r0[4..].copy_from_slice(&luma_rgba(y0[1], chroma));
        r1[..4].copy_from_slice(&luma_rgba(y1[0], chroma));
        r1[4..].copy_from_slice(&luma_rgba(y1[1], chroma));
    }
    if let (Some(&last0), Some(&last1)) = (l0_rem.first(), l1_rem.first()) {
        let chroma = chroma_terms(cb[pairs], cr[pairs]);
        o0_rem[..4].copy_from_slice(&luma_rgba(last0, chroma));
        o1_rem[..4].copy_from_slice(&luma_rgba(last1, chroma));
    }
}

#[inline(always)]
fn chroma_terms(cb: u8, cr: u8) -> (i32, i32, i32) {
    let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
    (409 * cr, -100 * cb - 208 * cr, 516 * cb)
}

#[inline(always)]
fn luma_rgba(y: u8, (rc, gc, bc): (i32, i32, i32)) -> [u8; 4] {
    // 298 * (y - 16) + 128 == 298 * y - 4640.
    let luma = 298 * i32::from(y) - 4640;
    let clip = |v: i32| (v >> 8).clamp(0, 255) as u8;
    [clip(luma + rc), clip(luma + gc), clip(luma + bc), 255]
}

/// BT.601 limited-range conversion; the decoded Y/Cb/Cr samples are retained exactly.
#[inline(always)]
fn ycbcr(y: u8, cb: u8, cr: u8) -> [u8; 4] {
    luma_rgba(y, chroma_terms(cb, cr))
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod x86 {
    use std::arch::x86_64::*;

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    unsafe fn sse2_finish_row(
        luma: *const u8,
        out: *mut u8,
        cb_lo: __m128i,
        cb_hi: __m128i,
        cr_lo: __m128i,
        cr_hi: __m128i,
        r_c_lo: __m128i,
        r_c_hi: __m128i,
        g_c_lo: __m128i,
        g_c_hi: __m128i,
        zero: __m128i,
        k42: __m128i,
        k4640: __m128i,
        alpha: __m128i,
    ) {
        // SAFETY: caller guarantees SSE2 and 16-byte luma / 64-byte output validity.
        unsafe {
            let vy = _mm_loadu_si128(luma.cast());
            let y_lo = _mm_unpacklo_epi8(vy, zero);
            let y_hi = _mm_unpackhi_epi8(vy, zero);
            let yc_lo = _mm_sub_epi16(_mm_mullo_epi16(y_lo, k42), k4640);
            let yc_hi = _mm_sub_epi16(_mm_mullo_epi16(y_hi, k42), k4640);

            let r_lo = _mm_add_epi16(
                _mm_add_epi16(y_lo, cr_lo),
                _mm_srai_epi16(_mm_add_epi16(yc_lo, r_c_lo), 8),
            );
            let r_hi = _mm_add_epi16(
                _mm_add_epi16(y_hi, cr_hi),
                _mm_srai_epi16(_mm_add_epi16(yc_hi, r_c_hi), 8),
            );

            let g_lo = _mm_add_epi16(
                _mm_sub_epi16(y_lo, cr_lo),
                _mm_srai_epi16(_mm_add_epi16(yc_lo, g_c_lo), 8),
            );
            let g_hi = _mm_add_epi16(
                _mm_sub_epi16(y_hi, cr_hi),
                _mm_srai_epi16(_mm_add_epi16(yc_hi, g_c_hi), 8),
            );

            let b_lo = _mm_add_epi16(
                _mm_add_epi16(y_lo, _mm_add_epi16(cb_lo, cb_lo)),
                _mm_srai_epi16(_mm_add_epi16(yc_lo, _mm_slli_epi16(cb_lo, 2)), 8),
            );
            let b_hi = _mm_add_epi16(
                _mm_add_epi16(y_hi, _mm_add_epi16(cb_hi, cb_hi)),
                _mm_srai_epi16(_mm_add_epi16(yc_hi, _mm_slli_epi16(cb_hi, 2)), 8),
            );

            let r8 = _mm_packus_epi16(r_lo, r_hi);
            let g8 = _mm_packus_epi16(g_lo, g_hi);
            let b8 = _mm_packus_epi16(b_lo, b_hi);

            let rg_lo = _mm_unpacklo_epi8(r8, g8);
            let rg_hi = _mm_unpackhi_epi8(r8, g8);
            let ba_lo = _mm_unpacklo_epi8(b8, alpha);
            let ba_hi = _mm_unpackhi_epi8(b8, alpha);

            _mm_storeu_si128(out.cast(), _mm_unpacklo_epi16(rg_lo, ba_lo));
            _mm_storeu_si128(out.add(16).cast(), _mm_unpackhi_epi16(rg_lo, ba_lo));
            _mm_storeu_si128(out.add(32).cast(), _mm_unpacklo_epi16(rg_hi, ba_hi));
            _mm_storeu_si128(out.add(48).cast(), _mm_unpackhi_epi16(rg_hi, ba_hi));
        }
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
            let c128 = _mm_set1_epi16(128);
            let alpha = _mm_set1_epi8(-1);
            // Exact 16-bit algebraic split matching `convert_chunks_avx2`.
            let k42 = _mm_set1_epi16(42);
            let k4640 = _mm_set1_epi16(4640);
            let k153 = _mm_set1_epi16(153);
            let k100 = _mm_set1_epi16(100);
            let k48 = _mm_set1_epi16(48);

            for _ in 0..chunks {
                let cb16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cb.cast()), zero), c128);
                let cr16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cr.cast()), zero), c128);
                let r_c = _mm_mullo_epi16(cr16, k153);
                let g_c = _mm_sub_epi16(_mm_mullo_epi16(cr16, k48), _mm_mullo_epi16(cb16, k100));

                let cb_lo = _mm_unpacklo_epi16(cb16, cb16);
                let cb_hi = _mm_unpackhi_epi16(cb16, cb16);
                let cr_lo = _mm_unpacklo_epi16(cr16, cr16);
                let cr_hi = _mm_unpackhi_epi16(cr16, cr16);
                let r_c_lo = _mm_unpacklo_epi16(r_c, r_c);
                let r_c_hi = _mm_unpackhi_epi16(r_c, r_c);
                let g_c_lo = _mm_unpacklo_epi16(g_c, g_c);
                let g_c_hi = _mm_unpackhi_epi16(g_c, g_c);

                sse2_finish_row(
                    luma, out, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );

                luma = luma.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out = out.add(64);
            }
        }
    }

    /// # Safety
    /// Caller guarantees SSE2 and valid non-aliasing spans for `chunks` iterations across two rows.
    pub(super) unsafe fn convert_pair_chunks(
        mut luma0: *const u8,
        mut luma1: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out0: *mut u8,
        mut out1: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: caller guarantees valid non-aliasing spans for `chunks` iterations.
        unsafe {
            let zero = _mm_setzero_si128();
            let c128 = _mm_set1_epi16(128);
            let alpha = _mm_set1_epi8(-1);
            let k42 = _mm_set1_epi16(42);
            let k4640 = _mm_set1_epi16(4640);
            let k153 = _mm_set1_epi16(153);
            let k100 = _mm_set1_epi16(100);
            let k48 = _mm_set1_epi16(48);

            for _ in 0..chunks {
                let cb16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cb.cast()), zero), c128);
                let cr16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cr.cast()), zero), c128);
                let r_c = _mm_mullo_epi16(cr16, k153);
                let g_c = _mm_sub_epi16(_mm_mullo_epi16(cr16, k48), _mm_mullo_epi16(cb16, k100));

                let cb_lo = _mm_unpacklo_epi16(cb16, cb16);
                let cb_hi = _mm_unpackhi_epi16(cb16, cb16);
                let cr_lo = _mm_unpacklo_epi16(cr16, cr16);
                let cr_hi = _mm_unpackhi_epi16(cr16, cr16);
                let r_c_lo = _mm_unpacklo_epi16(r_c, r_c);
                let r_c_hi = _mm_unpackhi_epi16(r_c, r_c);
                let g_c_lo = _mm_unpacklo_epi16(g_c, g_c);
                let g_c_hi = _mm_unpackhi_epi16(g_c, g_c);

                sse2_finish_row(
                    luma0, out0, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );
                sse2_finish_row(
                    luma1, out1, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );

                luma0 = luma0.add(16);
                luma1 = luma1.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out0 = out0.add(64);
                out1 = out1.add(64);
            }
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[allow(clippy::too_many_arguments)]
    unsafe fn avx2_finish_row(
        luma: *const u8,
        out: *mut u8,
        cb_lo: __m256i,
        cb_hi: __m256i,
        cr_lo: __m256i,
        cr_hi: __m256i,
        r_c_lo: __m256i,
        r_c_hi: __m256i,
        g_c_lo: __m256i,
        g_c_hi: __m256i,
        zero: __m256i,
        k42: __m256i,
        k4640: __m256i,
        alpha: __m256i,
    ) {
        // SAFETY: caller guarantees AVX2 and 32-byte luma / 128-byte output validity.
        unsafe {
            let vy = _mm256_loadu_si256(luma.cast());
            let y_lo = _mm256_unpacklo_epi8(vy, zero);
            let y_hi = _mm256_unpackhi_epi8(vy, zero);
            let yc_lo = _mm256_sub_epi16(_mm256_mullo_epi16(y_lo, k42), k4640);
            let yc_hi = _mm256_sub_epi16(_mm256_mullo_epi16(y_hi, k42), k4640);

            let r_lo = _mm256_add_epi16(
                _mm256_add_epi16(y_lo, cr_lo),
                _mm256_srai_epi16(_mm256_add_epi16(yc_lo, r_c_lo), 8),
            );
            let r_hi = _mm256_add_epi16(
                _mm256_add_epi16(y_hi, cr_hi),
                _mm256_srai_epi16(_mm256_add_epi16(yc_hi, r_c_hi), 8),
            );

            let g_lo = _mm256_add_epi16(
                _mm256_sub_epi16(y_lo, cr_lo),
                _mm256_srai_epi16(_mm256_add_epi16(yc_lo, g_c_lo), 8),
            );
            let g_hi = _mm256_add_epi16(
                _mm256_sub_epi16(y_hi, cr_hi),
                _mm256_srai_epi16(_mm256_add_epi16(yc_hi, g_c_hi), 8),
            );

            let b_lo = _mm256_add_epi16(
                _mm256_add_epi16(y_lo, _mm256_add_epi16(cb_lo, cb_lo)),
                _mm256_srai_epi16(_mm256_add_epi16(yc_lo, _mm256_slli_epi16(cb_lo, 2)), 8),
            );
            let b_hi = _mm256_add_epi16(
                _mm256_add_epi16(y_hi, _mm256_add_epi16(cb_hi, cb_hi)),
                _mm256_srai_epi16(_mm256_add_epi16(yc_hi, _mm256_slli_epi16(cb_hi, 2)), 8),
            );

            let r8 = _mm256_packus_epi16(r_lo, r_hi);
            let g8 = _mm256_packus_epi16(g_lo, g_hi);
            let b8 = _mm256_packus_epi16(b_lo, b_hi);

            let rg_lo = _mm256_unpacklo_epi8(r8, g8);
            let rg_hi = _mm256_unpackhi_epi8(r8, g8);
            let ba_lo = _mm256_unpacklo_epi8(b8, alpha);
            let ba_hi = _mm256_unpackhi_epi8(b8, alpha);

            let rgba0 = _mm256_unpacklo_epi16(rg_lo, ba_lo);
            let rgba1 = _mm256_unpackhi_epi16(rg_lo, ba_lo);
            let rgba2 = _mm256_unpacklo_epi16(rg_hi, ba_hi);
            let rgba3 = _mm256_unpackhi_epi16(rg_hi, ba_hi);

            _mm256_storeu_si256(out.cast(), _mm256_permute2x128_si256(rgba0, rgba1, 0x20));
            _mm256_storeu_si256(
                out.add(32).cast(),
                _mm256_permute2x128_si256(rgba2, rgba3, 0x20),
            );
            _mm256_storeu_si256(
                out.add(64).cast(),
                _mm256_permute2x128_si256(rgba0, rgba1, 0x31),
            );
            _mm256_storeu_si256(
                out.add(96).cast(),
                _mm256_permute2x128_si256(rgba2, rgba3, 0x31),
            );
        }
    }

    /// # Safety
    /// Caller guarantees AVX2 (`is_x86_feature_detected!("avx2")`) and that
    /// `luma` covers `chunks * 32` bytes, `cb` and `cr` each cover `chunks * 16`
    /// bytes, and `out` covers `chunks * 128` writable non-aliasing bytes.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn convert_chunks_avx2(
        mut luma: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: caller guarantees AVX2 and valid non-aliasing spans for `chunks` iterations.
        // Uses exact 16-bit algebraic decomposition:
        //   298*(y-16)+128 = 256*y + (42*y - 4640)
        //   409*(cr-128)   = 256*(cr-128) + 153*(cr-128)
        //  -208*(cr-128)   = -256*(cr-128) + 48*(cr-128)
        //   516*(cb-128)   = 512*(cb-128) + 4*(cb-128)
        // Every intermediate fits safely inside signed i16 ([-24224, +25501]).
        unsafe {
            let zero = _mm256_setzero_si256();
            let c128 = _mm256_set1_epi16(128);
            let k42 = _mm256_set1_epi16(42);
            let k4640 = _mm256_set1_epi16(4640);
            let k153 = _mm256_set1_epi16(153);
            let k100 = _mm256_set1_epi16(100);
            let k48 = _mm256_set1_epi16(48);
            let alpha = _mm256_set1_epi8(-1);

            for _ in 0..chunks {
                let cb16 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(_mm_loadu_si128(cb.cast())), c128);
                let cr16 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(_mm_loadu_si128(cr.cast())), c128);
                let r_c = _mm256_mullo_epi16(cr16, k153);
                let g_c = _mm256_sub_epi16(
                    _mm256_mullo_epi16(cr16, k48),
                    _mm256_mullo_epi16(cb16, k100),
                );
                let cb_lo = _mm256_unpacklo_epi16(cb16, cb16);
                let cb_hi = _mm256_unpackhi_epi16(cb16, cb16);
                let cr_lo = _mm256_unpacklo_epi16(cr16, cr16);
                let cr_hi = _mm256_unpackhi_epi16(cr16, cr16);
                let r_c_lo = _mm256_unpacklo_epi16(r_c, r_c);
                let r_c_hi = _mm256_unpackhi_epi16(r_c, r_c);
                let g_c_lo = _mm256_unpacklo_epi16(g_c, g_c);
                let g_c_hi = _mm256_unpackhi_epi16(g_c, g_c);

                avx2_finish_row(
                    luma, out, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );

                luma = luma.add(32);
                cb = cb.add(16);
                cr = cr.add(16);
                out = out.add(128);
            }
        }
    }

    /// # Safety
    /// Caller guarantees AVX2 and valid non-aliasing spans for `chunks` iterations across two rows.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn convert_pair_chunks_avx2(
        mut luma0: *const u8,
        mut luma1: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out0: *mut u8,
        mut out1: *mut u8,
        chunks: usize,
    ) {
        // SAFETY: caller guarantees AVX2 and valid non-aliasing spans for `chunks` iterations.
        unsafe {
            let zero = _mm256_setzero_si256();
            let c128 = _mm256_set1_epi16(128);
            let k42 = _mm256_set1_epi16(42);
            let k4640 = _mm256_set1_epi16(4640);
            let k153 = _mm256_set1_epi16(153);
            let k100 = _mm256_set1_epi16(100);
            let k48 = _mm256_set1_epi16(48);
            let alpha = _mm256_set1_epi8(-1);

            for _ in 0..chunks {
                let cb16 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(_mm_loadu_si128(cb.cast())), c128);
                let cr16 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(_mm_loadu_si128(cr.cast())), c128);
                let r_c = _mm256_mullo_epi16(cr16, k153);
                let g_c = _mm256_sub_epi16(
                    _mm256_mullo_epi16(cr16, k48),
                    _mm256_mullo_epi16(cb16, k100),
                );
                let cb_lo = _mm256_unpacklo_epi16(cb16, cb16);
                let cb_hi = _mm256_unpackhi_epi16(cb16, cb16);
                let cr_lo = _mm256_unpacklo_epi16(cr16, cr16);
                let cr_hi = _mm256_unpackhi_epi16(cr16, cr16);
                let r_c_lo = _mm256_unpacklo_epi16(r_c, r_c);
                let r_c_hi = _mm256_unpackhi_epi16(r_c, r_c);
                let g_c_lo = _mm256_unpacklo_epi16(g_c, g_c);
                let g_c_hi = _mm256_unpackhi_epi16(g_c, g_c);

                avx2_finish_row(
                    luma0, out0, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );
                avx2_finish_row(
                    luma1, out1, cb_lo, cb_hi, cr_lo, cr_hi, r_c_lo, r_c_hi, g_c_lo, g_c_hi, zero,
                    k42, k4640, alpha,
                );

                luma0 = luma0.add(32);
                luma1 = luma1.add(32);
                cb = cb.add(16);
                cr = cr.add(16);
                out0 = out0.add(128);
                out1 = out1.add(128);
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon {
    use std::arch::aarch64::*;

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    unsafe fn neon_finish_row(
        luma: *const u8,
        out: *mut u8,
        r0: int32x4_t,
        r1: int32x4_t,
        r2: int32x4_t,
        r3: int32x4_t,
        g0: int32x4_t,
        g1: int32x4_t,
        g2: int32x4_t,
        g3: int32x4_t,
        b0: int32x4_t,
        b1: int32x4_t,
        b2: int32x4_t,
        b3: int32x4_t,
        c298: int16x8_t,
        bias: int32x4_t,
        alpha: uint8x16_t,
    ) {
        // SAFETY: caller guarantees valid 16-byte luma and 64-byte output spans.
        unsafe {
            let vy = vld1q_u8(luma);
            let y_lo = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(vy)));
            let y_hi = vreinterpretq_s16_u16(vmovl_high_u8(vy));
            let l0 = vaddq_s32(vmull_s16(vget_low_s16(y_lo), vget_low_s16(c298)), bias);
            let l1 = vaddq_s32(vmull_high_s16(y_lo, c298), bias);
            let l2 = vaddq_s32(vmull_s16(vget_low_s16(y_hi), vget_low_s16(c298)), bias);
            let l3 = vaddq_s32(vmull_high_s16(y_hi, c298), bias);

            let channel = |c0, c1, c2, c3| {
                let s0 = vshrq_n_s32::<8>(vaddq_s32(l0, c0));
                let s1 = vshrq_n_s32::<8>(vaddq_s32(l1, c1));
                let s2 = vshrq_n_s32::<8>(vaddq_s32(l2, c2));
                let s3 = vshrq_n_s32::<8>(vaddq_s32(l3, c3));
                let n_lo = vcombine_s16(vmovn_s32(s0), vmovn_s32(s1));
                let n_hi = vcombine_s16(vmovn_s32(s2), vmovn_s32(s3));
                vcombine_u8(vqmovun_s16(n_lo), vqmovun_s16(n_hi))
            };

            let r = channel(r0, r1, r2, r3);
            let g = channel(g0, g1, g2, g3);
            let b = channel(b0, b1, b2, b3);
            vst4q_u8(out, uint8x16x4_t(r, g, b, alpha));
        }
    }

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

                neon_finish_row(
                    luma,
                    out,
                    vzip1q_s32(r_ch_03, r_ch_03),
                    vzip2q_s32(r_ch_03, r_ch_03),
                    vzip1q_s32(r_ch_47, r_ch_47),
                    vzip2q_s32(r_ch_47, r_ch_47),
                    vzip1q_s32(g_ch_03, g_ch_03),
                    vzip2q_s32(g_ch_03, g_ch_03),
                    vzip1q_s32(g_ch_47, g_ch_47),
                    vzip2q_s32(g_ch_47, g_ch_47),
                    vzip1q_s32(b_ch_03, b_ch_03),
                    vzip2q_s32(b_ch_03, b_ch_03),
                    vzip1q_s32(b_ch_47, b_ch_47),
                    vzip2q_s32(b_ch_47, b_ch_47),
                    c298,
                    bias,
                    alpha,
                );

                luma = luma.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out = out.add(64);
            }
        }
    }

    /// # Safety
    /// Caller guarantees NEON and valid non-aliasing spans for `chunks` iterations across two rows.
    pub(super) unsafe fn convert_pair_chunks(
        mut luma0: *const u8,
        mut luma1: *const u8,
        mut cb: *const u8,
        mut cr: *const u8,
        mut out0: *mut u8,
        mut out1: *mut u8,
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

                let r0 = vzip1q_s32(r_ch_03, r_ch_03);
                let r1 = vzip2q_s32(r_ch_03, r_ch_03);
                let r2 = vzip1q_s32(r_ch_47, r_ch_47);
                let r3 = vzip2q_s32(r_ch_47, r_ch_47);
                let g0 = vzip1q_s32(g_ch_03, g_ch_03);
                let g1 = vzip2q_s32(g_ch_03, g_ch_03);
                let g2 = vzip1q_s32(g_ch_47, g_ch_47);
                let g3 = vzip2q_s32(g_ch_47, g_ch_47);
                let b0 = vzip1q_s32(b_ch_03, b_ch_03);
                let b1 = vzip2q_s32(b_ch_03, b_ch_03);
                let b2 = vzip1q_s32(b_ch_47, b_ch_47);
                let b3 = vzip2q_s32(b_ch_47, b_ch_47);

                neon_finish_row(
                    luma0, out0, r0, r1, r2, r3, g0, g1, g2, g3, b0, b1, b2, b3, c298, bias, alpha,
                );
                neon_finish_row(
                    luma1, out1, r0, r1, r2, r3, g0, g1, g2, g3, b0, b1, b2, b3, c298, bias, alpha,
                );

                luma0 = luma0.add(16);
                luma1 = luma1.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out0 = out0.add(64);
                out1 = out1.add(64);
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

    /// Verbatim reference BT.601 limited-range single-pixel formula.
    fn reference_ycbcr(y: u8, cb: u8, cr: u8) -> [u8; 4] {
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

    fn convert_row_reference(luma: &[u8], cb: &[u8], cr: &[u8], out: &mut [u8], odd_left: bool) {
        let base = usize::from(odd_left);
        for (x, (&y, rgba)) in luma.iter().zip(out.as_chunks_mut::<4>().0).enumerate() {
            let cx = (base + x) / 2;
            rgba.copy_from_slice(&reference_ycbcr(y, cb[cx], cr[cx]));
        }
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
            for width in 0usize..=72 {
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
                    convert_row_reference(
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

                    let luma1: Vec<u8> = (0..offset + width).map(|_| next_u8()).collect();
                    let mut got0 = vec![0x5a; offset + width * 4];
                    let mut got1 = vec![0x5a; offset + width * 4];
                    let mut want1 = vec![0x5a; offset + width * 4];
                    convert_row_pair(
                        &luma[offset..],
                        &luma1[offset..],
                        &cb[offset..],
                        &cr[offset..],
                        &mut got0[offset..],
                        &mut got1[offset..],
                        odd_left,
                    );
                    convert_row_reference(
                        &luma1[offset..],
                        &cb[offset..],
                        &cr[offset..],
                        &mut want1[offset..],
                        odd_left,
                    );
                    assert_eq!(got0, want);
                    assert_eq!(got1, want1);

                    // Also verify the 2-row shared-chroma scalar path directly, even on
                    // hosts whose runtime dispatcher takes AVX2/SSE2/NEON.
                    if !odd_left {
                        let mut scalar0 = vec![0x5a; offset + width * 4];
                        let mut scalar1 = vec![0x5a; offset + width * 4];
                        convert_pair_scalar(
                            &luma[offset..],
                            &luma1[offset..],
                            &cb[offset..],
                            &cr[offset..],
                            &mut scalar0[offset..],
                            &mut scalar1[offset..],
                        );
                        assert_eq!(scalar0, want);
                        assert_eq!(scalar1, want1);
                    }
                }
            }
        }
    }

    #[test]
    fn simd_matches_scalar_at_saturation_and_rounding_boundaries() {
        let ys = [0, 1, 15, 16, 17, 127, 128, 235, 254, 255];
        let chromas = [0, 1, 64, 127, 128, 129, 192, 254, 255];
        let mut luma0 = [0u8; 32];
        let mut luma1 = [0u8; 32];
        let mut cb = [0u8; 16];
        let mut cr = [0u8; 16];

        let check_block = |l0: &[u8; 32], l1: &[u8; 32], u: &[u8; 16], v: &[u8; 16]| {
            let mut want0 = [0u8; 128];
            let mut want1 = [0u8; 128];
            convert_row_reference(l0, u, v, &mut want0, false);
            convert_row_reference(l1, u, v, &mut want1, false);

            let mut got = [0u8; 128];
            convert_pairs(l0, u, v, &mut got);
            assert_eq!(got, want0);

            let mut got0 = [0u8; 128];
            let mut got1 = [0u8; 128];
            convert_row_pair(l0, l1, u, v, &mut got0, &mut got1, false);
            assert_eq!(got0, want0);
            assert_eq!(got1, want1);

            let mut sc0 = [0u8; 128];
            let mut sc1 = [0u8; 128];
            convert_pair_scalar(l0, l1, u, v, &mut sc0, &mut sc1);
            assert_eq!(sc0, want0);
            assert_eq!(sc1, want1);

            #[cfg(target_arch = "x86_64")]
            {
                let mut sse0 = [0u8; 128];
                let mut sse1 = [0u8; 128];
                // SAFETY: SSE2 is baseline on x86_64 and buffers match 2 chunks.
                #[allow(unsafe_code)]
                unsafe {
                    x86::convert_pair_chunks(
                        l0.as_ptr(),
                        l1.as_ptr(),
                        u.as_ptr(),
                        v.as_ptr(),
                        sse0.as_mut_ptr(),
                        sse1.as_mut_ptr(),
                        2,
                    );
                }
                assert_eq!(sse0, want0);
                assert_eq!(sse1, want1);
            }
        };

        let mut count = 0usize;
        for &y in &ys {
            for &u in &chromas {
                for &v in &chromas {
                    let slot = count % 16;
                    count += 1;
                    luma0[slot * 2] = y;
                    luma0[slot * 2 + 1] = y.wrapping_add(13);
                    luma1[slot * 2] = 255u8.wrapping_sub(y);
                    luma1[slot * 2 + 1] = y.wrapping_mul(3).wrapping_add(7);
                    cb[slot] = u;
                    cr[slot] = v;
                    if slot == 15 {
                        check_block(&luma0, &luma1, &cb, &cr);
                    }
                }
            }
        }
        if !count.is_multiple_of(16) {
            check_block(&luma0, &luma1, &cb, &cr);
        }
    }

    #[test]
    fn planar_conversion_matches_reference_across_even_odd_sizes_and_pixel_formats() {
        #[derive(Clone, Copy, Debug)]
        enum Subsampling {
            Yuv420,
            Yuv422,
            Yuv444,
        }

        let mut seed = 0x9e37_79b9u32;
        let mut next_u8 = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        };

        let widths = [0usize, 1, 2, 3, 7, 8, 15, 16, 17, 31, 32, 33, 47, 48, 65];
        let heights = [0usize, 1, 2, 3, 4, 5, 7, 8, 9];

        for format in [
            Subsampling::Yuv420,
            Subsampling::Yuv422,
            Subsampling::Yuv444,
        ] {
            for &width in &widths {
                for &height in &heights {
                    let (x_shift, y_shift) = match format {
                        Subsampling::Yuv420 => (1, 1),
                        Subsampling::Yuv422 => (1, 0),
                        Subsampling::Yuv444 => (0, 0),
                    };
                    for left in [0usize, 1] {
                        for bottom in [0usize, 1] {
                            let yw = left + width;
                            let yh = bottom + height;
                            let cw = yw.div_ceil(1 << x_shift);
                            let ch = yh.div_ceil(1 << y_shift);
                            let y_plane: Vec<u8> = (0..yw * yh).map(|_| next_u8()).collect();
                            let cb_plane: Vec<u8> = (0..cw * ch).map(|_| next_u8()).collect();
                            let cr_plane: Vec<u8> = (0..cw * ch).map(|_| next_u8()).collect();

                            let mut want = vec![0u8; width * height * 4];
                            for row in 0..height {
                                let y = bottom + height - 1 - row;
                                for col in 0..width {
                                    let x = left + col;
                                    let luma = y_plane[y * yw + x];
                                    let cb = cb_plane[(y >> y_shift) * cw + (x >> x_shift)];
                                    let cr = cr_plane[(y >> y_shift) * cw + (x >> x_shift)];
                                    let at = (row * width + col) * 4;
                                    want[at..at + 4]
                                        .copy_from_slice(&reference_ycbcr(luma, cb, cr));
                                }
                            }

                            let mut got = vec![0u8; width * height * 4];
                            let stride = width * 4;
                            let odd_left = (left & 1) == 1;
                            match format {
                                Subsampling::Yuv420 => {
                                    let c_left = left / 2;
                                    let c_right = (left + width).div_ceil(2);
                                    let mut row = 0;
                                    while row < height {
                                        let y = bottom + height - 1 - row;
                                        let c_span = (y / 2) * cw + c_left..(y / 2) * cw + c_right;
                                        let cb = &cb_plane[c_span.clone()];
                                        let cr = &cr_plane[c_span];
                                        if row + 1 < height && (y & 1) == 1 {
                                            let l0 = &y_plane[y * yw + left..y * yw + left + width];
                                            let y1 = y - 1;
                                            let l1 =
                                                &y_plane[y1 * yw + left..y1 * yw + left + width];
                                            let (o0, o1) = got[row * stride..(row + 2) * stride]
                                                .split_at_mut(stride);
                                            convert_row_pair(l0, l1, cb, cr, o0, o1, odd_left);
                                            row += 2;
                                        } else {
                                            let l0 = &y_plane[y * yw + left..y * yw + left + width];
                                            let o0 = &mut got[row * stride..(row + 1) * stride];
                                            convert_row(l0, cb, cr, o0, odd_left);
                                            row += 1;
                                        }
                                    }
                                }
                                Subsampling::Yuv422 => {
                                    let c_left = left / 2;
                                    let c_right = (left + width).div_ceil(2);
                                    for row in 0..height {
                                        let y = bottom + height - 1 - row;
                                        let l0 = &y_plane[y * yw + left..y * yw + left + width];
                                        let cb = &cb_plane[y * cw + c_left..y * cw + c_right];
                                        let cr = &cr_plane[y * cw + c_left..y * cw + c_right];
                                        let o0 = &mut got[row * stride..(row + 1) * stride];
                                        convert_row(l0, cb, cr, o0, odd_left);
                                    }
                                }
                                Subsampling::Yuv444 => {
                                    for row in 0..height {
                                        let y = bottom + height - 1 - row;
                                        let l0 = &y_plane[y * yw + left..y * yw + left + width];
                                        let cb = &cb_plane[y * cw + left..y * cw + left + width];
                                        let cr = &cr_plane[y * cw + left..y * cw + left + width];
                                        let o0 = &mut got[row * stride..(row + 1) * stride];
                                        for (((&yv, &u), &v), px) in
                                            l0.iter().zip(cb).zip(cr).zip(o0.as_chunks_mut::<4>().0)
                                        {
                                            px.copy_from_slice(&ycbcr(yv, u, v));
                                        }
                                    }
                                }
                            }
                            assert_eq!(
                                got, want,
                                "format={format:?} {width}x{height} left={left} bottom={bottom}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[should_panic]
    fn convert_pairs_rejects_short_chroma_before_simd() {
        let mut out = [0u8; 64];
        convert_pairs(&[0; 16], &[0; 7], &[0; 8], &mut out);
    }

    #[test]
    #[should_panic]
    fn convert_row_pair_rejects_short_chroma_before_simd() {
        let mut out0 = [0u8; 64];
        let mut out1 = [0u8; 64];
        convert_row_pair(
            &[0; 16], &[0; 16], &[0; 7], &[0; 8], &mut out0, &mut out1, false,
        );
    }

    #[test]
    #[should_panic]
    fn convert_pairs_rejects_short_output_before_simd() {
        let mut out = [0u8; 63];
        convert_pairs(&[0; 16], &[0; 8], &[0; 8], &mut out);
    }
}
