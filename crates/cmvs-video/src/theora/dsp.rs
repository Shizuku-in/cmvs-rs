//! Integer reconstruction kernels. All packet parsing stays in safe Rust.

// Theora I §7.9.3.1. Narrowing at the specified intermediate steps is normative.
#[cfg(any(test, not(target_arch = "x86_64")))]
fn idct_1d(y: [i16; 8]) -> [i16; 8] {
    let y = y.map(i32::from);
    let mul = |c: i32, v: i32| (c * v) >> 16;
    let a = mul(46341, i32::from((y[0] + y[4]) as i16));
    let b = mul(46341, i32::from((y[0] - y[4]) as i16));
    let c = mul(25080, y[2]) - mul(60547, y[6]);
    let d = mul(60547, y[2]) + mul(25080, y[6]);
    let e = mul(12785, y[1]) - mul(64277, y[7]);
    let f = mul(54491, y[5]) - mul(36410, y[3]);
    let g = mul(36410, y[5]) + mul(54491, y[3]);
    let h = mul(64277, y[1]) + mul(12785, y[7]);
    let ef = mul(46341, i32::from((e - f) as i16));
    let hg = mul(46341, i32::from((h - g) as i16));
    let t = [a + d, b + c, b - c, a - d, e + f, hg - ef, hg + ef, h + g];
    [
        t[0] + t[7],
        t[1] + t[6],
        t[2] + t[5],
        t[3] + t[4],
        t[3] - t[4],
        t[2] - t[5],
        t[1] - t[6],
        t[0] - t[7],
    ]
    .map(|v| v as i16)
}

#[allow(clippy::needless_range_loop)] // The second pass traverses columns.
#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) fn idct_scalar(coeffs: &[i16; 64]) -> [[i16; 8]; 8] {
    let mut rows = [[0i16; 8]; 8];
    for (src, dst) in coeffs.as_chunks::<8>().0.iter().zip(&mut rows) {
        *dst = idct_1d(*src);
    }
    for x in 0..8 {
        let column = idct_1d(std::array::from_fn(|y| rows[y][x]));
        for y in 0..8 {
            rows[y][x] = ((i32::from(column[y]) + 8) >> 4) as i16;
        }
    }
    rows
}

pub(super) fn idct(coeffs: &[i16; 64]) -> [[i16; 8]; 8] {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is guaranteed on x86-64. The fixed input and output contain
    // eight rows of exactly eight i16 lanes; all accesses are unaligned.
    #[allow(unsafe_code)]
    unsafe {
        x86::idct(coeffs)
    }
    #[cfg(not(target_arch = "x86_64"))]
    idct_scalar(coeffs)
}

#[cfg(any(test, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub(super) fn put_scalar(dst: &mut [u8; 8], pred: &[u8; 8], residual: &[i16; 8]) {
    for i in 0..8 {
        dst[i] = (i32::from(pred[i]) + i32::from(residual[i])).clamp(0, 255) as u8;
    }
}

pub(super) fn put(dst: &mut [u8; 8], pred: &[u8; 8], residual: &[i16; 8]) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is part of x86-64. References prove the 8-byte pixel and
    // 16-byte residual footprints; unaligned loads/stores require no alignment.
    #[allow(unsafe_code)]
    unsafe {
        x86::put(dst, pred, residual);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: Advanced SIMD is enabled by the supported AArch64 targets. The
    // fixed arrays prove every load and store footprint and cannot alias.
    #[allow(unsafe_code)]
    unsafe {
        neon::put(dst, pred, residual);
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    put_scalar(dst, pred, residual);
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod x86 {
    use std::arch::x86_64::*;

    #[inline]
    unsafe fn multiply<const C: i32>(v: __m128i) -> __m128i {
        // SAFETY: SSE2 is guaranteed by the caller. For C >= 32768 the signed
        // high multiply uses C-65536; adding v restores floor(C*v/65536).
        unsafe {
            let product = _mm_mulhi_epi16(v, _mm_set1_epi16(C as i16));
            if C >= 32768 {
                _mm_add_epi16(product, v)
            } else {
                product
            }
        }
    }

    #[inline]
    unsafe fn transform(y: [__m128i; 8]) -> [__m128i; 8] {
        // SAFETY: only SSE2 register operations. Wraps commute with additions;
        // all inputs to high multiplies have the normative i16 truncation.
        unsafe {
            let a = multiply::<46341>(_mm_add_epi16(y[0], y[4]));
            let b = multiply::<46341>(_mm_sub_epi16(y[0], y[4]));
            let c = _mm_sub_epi16(multiply::<25080>(y[2]), multiply::<60547>(y[6]));
            let d = _mm_add_epi16(multiply::<60547>(y[2]), multiply::<25080>(y[6]));
            let e = _mm_sub_epi16(multiply::<12785>(y[1]), multiply::<64277>(y[7]));
            let f = _mm_sub_epi16(multiply::<54491>(y[5]), multiply::<36410>(y[3]));
            let g = _mm_add_epi16(multiply::<36410>(y[5]), multiply::<54491>(y[3]));
            let h = _mm_add_epi16(multiply::<64277>(y[1]), multiply::<12785>(y[7]));
            let ef = multiply::<46341>(_mm_sub_epi16(e, f));
            let hg = multiply::<46341>(_mm_sub_epi16(h, g));
            let t = [
                _mm_add_epi16(a, d),
                _mm_add_epi16(b, c),
                _mm_sub_epi16(b, c),
                _mm_sub_epi16(a, d),
                _mm_add_epi16(e, f),
                _mm_sub_epi16(hg, ef),
                _mm_add_epi16(hg, ef),
                _mm_add_epi16(h, g),
            ];
            [
                _mm_add_epi16(t[0], t[7]),
                _mm_add_epi16(t[1], t[6]),
                _mm_add_epi16(t[2], t[5]),
                _mm_add_epi16(t[3], t[4]),
                _mm_sub_epi16(t[3], t[4]),
                _mm_sub_epi16(t[2], t[5]),
                _mm_sub_epi16(t[1], t[6]),
                _mm_sub_epi16(t[0], t[7]),
            ]
        }
    }

    #[inline]
    unsafe fn transpose(v: [__m128i; 8]) -> [__m128i; 8] {
        // SAFETY: all operations shuffle SSE2 registers, without memory access.
        unsafe {
            let a = [
                _mm_unpacklo_epi16(v[0], v[1]),
                _mm_unpackhi_epi16(v[0], v[1]),
                _mm_unpacklo_epi16(v[2], v[3]),
                _mm_unpackhi_epi16(v[2], v[3]),
                _mm_unpacklo_epi16(v[4], v[5]),
                _mm_unpackhi_epi16(v[4], v[5]),
                _mm_unpacklo_epi16(v[6], v[7]),
                _mm_unpackhi_epi16(v[6], v[7]),
            ];
            let b = [
                _mm_unpacklo_epi32(a[0], a[2]),
                _mm_unpackhi_epi32(a[0], a[2]),
                _mm_unpacklo_epi32(a[1], a[3]),
                _mm_unpackhi_epi32(a[1], a[3]),
                _mm_unpacklo_epi32(a[4], a[6]),
                _mm_unpackhi_epi32(a[4], a[6]),
                _mm_unpacklo_epi32(a[5], a[7]),
                _mm_unpackhi_epi32(a[5], a[7]),
            ];
            [
                _mm_unpacklo_epi64(b[0], b[4]),
                _mm_unpackhi_epi64(b[0], b[4]),
                _mm_unpacklo_epi64(b[1], b[5]),
                _mm_unpackhi_epi64(b[1], b[5]),
                _mm_unpacklo_epi64(b[2], b[6]),
                _mm_unpackhi_epi64(b[2], b[6]),
                _mm_unpacklo_epi64(b[3], b[7]),
                _mm_unpackhi_epi64(b[3], b[7]),
            ]
        }
    }

    pub(super) unsafe fn idct(coeffs: &[i16; 64]) -> [[i16; 8]; 8] {
        // SAFETY: SSE2 and fixed 8x8 footprints are guaranteed by the caller.
        // Each transform runs eight independent 1D IDCTs, one per lane.
        unsafe {
            let rows = std::array::from_fn(|i| _mm_loadu_si128(coeffs.as_ptr().add(i * 8).cast()));
            let rows = transform(transpose(transform(transpose(rows))));
            let mut out = [[0i16; 8]; 8];
            for (v, row) in rows.into_iter().zip(&mut out) {
                // Widened (X+8)>>4, expressed without overflowing signed i16.
                let rounded = _mm_add_epi16(
                    _mm_srai_epi16(v, 4),
                    _mm_and_si128(_mm_srli_epi16(v, 3), _mm_set1_epi16(1)),
                );
                _mm_storeu_si128(row.as_mut_ptr().cast(), rounded);
            }
            out
        }
    }

    pub(super) unsafe fn put(dst: &mut [u8; 8], pred: &[u8; 8], residual: &[i16; 8]) {
        // SAFETY: the caller guarantees SSE2; fixed references cover each load
        // and store. Saturated signed addition followed by unsigned packing is
        // identical to widening the sum and clamping it into 0..=255.
        unsafe {
            let p = _mm_loadl_epi64(pred.as_ptr().cast());
            let p = _mm_unpacklo_epi8(p, _mm_setzero_si128());
            let r = _mm_loadu_si128(residual.as_ptr().cast());
            let pixels = _mm_packus_epi16(_mm_adds_epi16(p, r), _mm_setzero_si128());
            _mm_storel_epi64(dst.as_mut_ptr().cast(), pixels);
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon {
    use std::arch::aarch64::*;

    pub(super) unsafe fn put(dst: &mut [u8; 8], pred: &[u8; 8], residual: &[i16; 8]) {
        // SAFETY: the caller guarantees NEON and fixed arrays cover the vector
        // footprints. The u8 predictor extends into positive signed i16 lanes.
        unsafe {
            let p = vreinterpretq_s16_u16(vmovl_u8(vld1_u8(pred.as_ptr())));
            let r = vld1q_s16(residual.as_ptr());
            vst1_u8(dst.as_mut_ptr(), vqmovun_s16(vqaddq_s16(p, r)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_put_matches_scalar_including_signed_overflow() {
        for p in 0..=255 {
            let pred = [p; 8];
            for residual in [i16::MIN, -256, -1, 0, 1, 255, 32513, i16::MAX] {
                let mut want = [0; 8];
                let mut got = [0; 8];
                put_scalar(&mut want, &pred, &[residual; 8]);
                put(&mut got, &pred, &[residual; 8]);
                assert_eq!(got, want);
            }
        }
    }
}
