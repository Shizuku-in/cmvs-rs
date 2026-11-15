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
        first0.copy_from_slice(&ycbcr(y0, cb[0], cr[0]));
        first1.copy_from_slice(&ycbcr(y1, cb[0], cr[0]));
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
    let mut done_pairs = 0;
    #[cfg(not(target_arch = "x86_64"))]
    let done_pairs = 0;
    #[cfg(target_arch = "x86_64")]
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
    if done_pairs * 2 < luma0.len() {
        convert_pairs(
            &luma0[done_pairs * 2..],
            &cb[done_pairs..],
            &cr[done_pairs..],
            &mut out0[done_pairs * 8..],
        );
        convert_pairs(
            &luma1[done_pairs * 2..],
            &cb[done_pairs..],
            &cr[done_pairs..],
            &mut out1[done_pairs * 8..],
        );
    }
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
            let k100 = _mm_set1_epi16(-100);
            let k48 = _mm_set1_epi16(48);

            for _ in 0..chunks {
                let vy = _mm_loadu_si128(luma.cast());
                let y_lo = _mm_unpacklo_epi8(vy, zero);
                let y_hi = _mm_unpackhi_epi8(vy, zero);
                let yc_lo = _mm_sub_epi16(_mm_mullo_epi16(y_lo, k42), k4640);
                let yc_hi = _mm_sub_epi16(_mm_mullo_epi16(y_hi, k42), k4640);

                let cb16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cb.cast()), zero), c128);
                let cr16 = _mm_sub_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(cr.cast()), zero), c128);
                let cb_lo = _mm_unpacklo_epi16(cb16, cb16);
                let cb_hi = _mm_unpackhi_epi16(cb16, cb16);
                let cr_lo = _mm_unpacklo_epi16(cr16, cr16);
                let cr_hi = _mm_unpackhi_epi16(cr16, cr16);

                let r_c_lo = _mm_mullo_epi16(cr_lo, k153);
                let r_c_hi = _mm_mullo_epi16(cr_hi, k153);
                let g_c_lo =
                    _mm_add_epi16(_mm_mullo_epi16(cb_lo, k100), _mm_mullo_epi16(cr_lo, k48));
                let g_c_hi =
                    _mm_add_epi16(_mm_mullo_epi16(cb_hi, k100), _mm_mullo_epi16(cr_hi, k48));

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

                luma = luma.add(16);
                cb = cb.add(8);
                cr = cr.add(8);
                out = out.add(64);
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
                    convert_row_scalar(
                        &luma1[offset..],
                        &cb[offset..],
                        &cr[offset..],
                        &mut want1[offset..],
                        odd_left,
                    );
                    assert_eq!(got0, want);
                    assert_eq!(got1, want1);
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
