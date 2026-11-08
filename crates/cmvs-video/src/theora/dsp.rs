//! Integer reconstruction kernels. All packet parsing stays in safe Rust.

pub(super) enum Residual {
    Dc(i16),
    Ac([[i16; 8]; 8]),
}

impl Residual {
    pub fn row(&self, row: usize) -> [i16; 8] {
        match self {
            Self::Dc(dc) => [*dc; 8],
            Self::Ac(rows) => rows[row],
        }
    }
}

fn extent(stride: usize) -> usize {
    stride
        .checked_mul(7)
        .and_then(|n| n.checked_add(8))
        .expect("Theora block stride overflow")
}

/// Reconstruct one validated strided 8x8 rectangle. The slice checks cover every
/// row before entering SIMD, so kernels do not repeat eight bounds checks.
#[inline]
pub(super) fn inter(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    residual: &Residual,
) {
    let n = extent(stride);
    let dst = &mut dst[..n];
    let first = &first[..n];
    let second = second.map(|s| &s[..n]);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline. The checked rectangles cover all eight rows;
    // dst is exclusive and reference planes are read-only, including aliasing
    // first/second references. Residual arrays have exactly eight i16 rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::inter(dst, stride, first, second, residual);
    }
    #[cfg(not(target_arch = "x86_64"))]
    inter_scalar(dst, stride, first, second, residual);
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn inter_scalar(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    residual: &Residual,
) {
    for row in 0..8 {
        let start = row * stride;
        let a: &[u8; 8] = first[start..start + 8].try_into().unwrap();
        let mut pred = *a;
        if let Some(b) = second {
            average(&mut pred, a, b[start..start + 8].try_into().unwrap());
        }
        put(
            (&mut dst[start..start + 8]).try_into().unwrap(),
            &pred,
            &residual.row(row),
        );
    }
}

#[inline]
pub(super) fn intra(dst: &mut [u8], stride: usize, residual: &Residual) {
    let dst = &mut dst[..extent(stride)];
    if let Residual::Dc(dc) = residual {
        let pixel = (128 + i32::from(*dc)).clamp(0, 255) as u8;
        for row in 0..8 {
            dst[row * stride..row * stride + 8].fill(pixel);
        }
        return;
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; the checked destination covers eight complete
    // rows, and the INTRA specialization does not access reference pointers.
    #[allow(unsafe_code)]
    unsafe {
        x86::intra(dst, stride, residual);
    }
    #[cfg(not(target_arch = "x86_64"))]
    for row in 0..8 {
        put(
            (&mut dst[row * stride..row * stride + 8])
                .try_into()
                .unwrap(),
            &[128; 8],
            &residual.row(row),
        );
    }
}

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

pub(super) fn idct(coeffs: &[i16; 64], ncoeffs: u8) -> [[i16; 8]; 8] {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is guaranteed on x86-64. The fixed input and output contain
    // eight rows of exactly eight i16 lanes; all accesses are unaligned.
    #[allow(unsafe_code)]
    unsafe {
        x86::idct(coeffs, ncoeffs)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = ncoeffs;
        idct_scalar(coeffs)
    }
}

#[cfg(any(test, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub(super) fn put_scalar(dst: &mut [u8; 8], pred: &[u8; 8], residual: &[i16; 8]) {
    for i in 0..8 {
        dst[i] = (i32::from(pred[i]) + i32::from(residual[i])).clamp(0, 255) as u8;
    }
}

#[inline]
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

#[inline]
#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) fn average(dst: &mut [u8; 8], a: &[u8; 8], b: &[u8; 8]) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is the architecture baseline; the fixed arrays cover all
    // unaligned eight-byte accesses, and dst cannot alias the inputs.
    #[allow(unsafe_code)]
    unsafe {
        x86::average(dst, a, b);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: supported AArch64 targets enable NEON; the fixed arrays cover
    // all eight-byte vector accesses and mutable output is exclusive.
    #[allow(unsafe_code)]
    unsafe {
        neon::average(dst, a, b);
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    for i in 0..8 {
        dst[i] = ((u16::from(a[i]) + u16::from(b[i])) >> 1) as u8;
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod x86 {
    use super::Residual;
    use std::arch::x86_64::*;

    #[inline]
    unsafe fn block<const INTRA: bool, const HALF: bool, const DC: bool>(
        dst: *mut u8,
        stride: usize,
        first: *const u8,
        second: *const u8,
        dc: i16,
        residual: *const i16,
    ) {
        // SAFETY: callers check all strided 8x8 footprints before dispatch.
        // Null source/residual pointers are passed only to specializations
        // whose constant branches never load those pointers.
        unsafe {
            let zero = _mm_setzero_si128();
            let dc = _mm_set1_epi16(dc);
            for row in 0..8 {
                let pred = if INTRA {
                    _mm_set1_epi16(128)
                } else {
                    let a = _mm_loadl_epi64(first.add(row * stride).cast());
                    let p = if HALF {
                        let b = _mm_loadl_epi64(second.add(row * stride).cast());
                        let odd = _mm_and_si128(_mm_xor_si128(a, b), _mm_set1_epi8(1));
                        _mm_sub_epi8(_mm_avg_epu8(a, b), odd)
                    } else {
                        a
                    };
                    _mm_unpacklo_epi8(p, zero)
                };
                let r = if DC {
                    dc
                } else {
                    _mm_loadu_si128(residual.add(row * 8).cast())
                };
                let pixels = _mm_packus_epi16(_mm_adds_epi16(pred, r), zero);
                _mm_storel_epi64(dst.add(row * stride).cast(), pixels);
            }
        }
    }

    pub(super) unsafe fn inter(
        dst: &mut [u8],
        stride: usize,
        first: &[u8],
        second: Option<&[u8]>,
        residual: &Residual,
    ) {
        // SAFETY: wrapper has checked all footprints and SSE2 availability.
        unsafe {
            let d = dst.as_mut_ptr();
            let a = first.as_ptr();
            match (second, residual) {
                (None, Residual::Dc(dc)) => block::<false, false, true>(
                    d,
                    stride,
                    a,
                    std::ptr::null(),
                    *dc,
                    std::ptr::null(),
                ),
                (Some(b), Residual::Dc(dc)) => {
                    block::<false, true, true>(d, stride, a, b.as_ptr(), *dc, std::ptr::null())
                }
                (None, Residual::Ac(rows)) => block::<false, false, false>(
                    d,
                    stride,
                    a,
                    std::ptr::null(),
                    0,
                    rows.as_ptr().cast(),
                ),
                (Some(b), Residual::Ac(rows)) => {
                    block::<false, true, false>(d, stride, a, b.as_ptr(), 0, rows.as_ptr().cast())
                }
            }
        }
    }

    pub(super) unsafe fn intra(dst: &mut [u8], stride: usize, residual: &Residual) {
        if let Residual::Ac(rows) = residual {
            // SAFETY: wrapper checked dst and SSE2. The INTRA specialization
            // never dereferences reference planes, so those pointers are null.
            unsafe {
                block::<true, false, false>(
                    dst.as_mut_ptr(),
                    stride,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    rows.as_ptr().cast(),
                );
            }
        }
    }

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

    #[inline]
    unsafe fn sparse_rows(v: [__m128i; 8], four: bool) -> [__m128i; 8] {
        // SAFETY: SSE2 register shuffles. Input vectors contain at most two or
        // four active rows in their low lanes; all discarded lanes are zero.
        unsafe {
            let a = [
                _mm_unpacklo_epi16(v[0], v[1]),
                _mm_unpacklo_epi16(v[2], v[3]),
                _mm_unpacklo_epi16(v[4], v[5]),
                _mm_unpacklo_epi16(v[6], v[7]),
            ];
            let lo = [
                _mm_unpacklo_epi32(a[0], a[1]),
                _mm_unpacklo_epi32(a[2], a[3]),
            ];
            let zero = _mm_setzero_si128();
            let (row2, row3) = if four {
                let hi = [
                    _mm_unpackhi_epi32(a[0], a[1]),
                    _mm_unpackhi_epi32(a[2], a[3]),
                ];
                (
                    _mm_unpacklo_epi64(hi[0], hi[1]),
                    _mm_unpackhi_epi64(hi[0], hi[1]),
                )
            } else {
                (zero, zero)
            };
            [
                _mm_unpacklo_epi64(lo[0], lo[1]),
                _mm_unpackhi_epi64(lo[0], lo[1]),
                row2,
                row3,
                zero,
                zero,
                zero,
                zero,
            ]
        }
    }

    pub(super) unsafe fn idct(coeffs: &[i16; 64], ncoeffs: u8) -> [[i16; 8]; 8] {
        // SAFETY: SSE2 and fixed 8x8 footprints are guaranteed by the caller.
        // Each transform runs eight independent 1D IDCTs, one per lane.
        unsafe {
            let zero = _mm_setzero_si128();
            let rows = if ncoeffs <= 3 {
                // First three zig-zag positions occupy only a 2x2 triangle.
                let y0 = _mm_cvtsi32_si128(
                    (u32::from(coeffs[0] as u16) | (u32::from(coeffs[8] as u16) << 16)) as i32,
                );
                let y1 = _mm_cvtsi32_si128(i32::from(coeffs[1] as u16));
                sparse_rows(
                    transform([y0, y1, zero, zero, zero, zero, zero, zero]),
                    false,
                )
            } else if ncoeffs <= 10 {
                // First ten zig-zag positions occupy a 4x4 triangle.
                let y0 = _mm_set_epi16(0, 0, 0, 0, coeffs[24], coeffs[16], coeffs[8], coeffs[0]);
                let y1 = _mm_set_epi16(0, 0, 0, 0, 0, coeffs[17], coeffs[9], coeffs[1]);
                let y2 = _mm_set_epi16(0, 0, 0, 0, 0, 0, coeffs[10], coeffs[2]);
                let y3 = _mm_cvtsi32_si128(i32::from(coeffs[3] as u16));
                sparse_rows(transform([y0, y1, y2, y3, zero, zero, zero, zero]), true)
            } else {
                let rows =
                    std::array::from_fn(|i| _mm_loadu_si128(coeffs.as_ptr().add(i * 8).cast()));
                transpose(transform(transpose(rows)))
            };
            let rows = transform(rows);
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

    #[cfg(test)]
    #[inline]
    pub(super) unsafe fn average(dst: &mut [u8; 8], a: &[u8; 8], b: &[u8; 8]) {
        // SAFETY: caller guarantees SSE2 and eight-byte footprints. The SSE2
        // rounded-up average is corrected for odd sums to truncate downward.
        unsafe {
            let a = _mm_loadl_epi64(a.as_ptr().cast());
            let b = _mm_loadl_epi64(b.as_ptr().cast());
            let odd = _mm_and_si128(_mm_xor_si128(a, b), _mm_set1_epi8(1));
            _mm_storel_epi64(
                dst.as_mut_ptr().cast(),
                _mm_sub_epi8(_mm_avg_epu8(a, b), odd),
            );
        }
    }

    #[inline]
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

    #[inline]
    pub(super) unsafe fn average(dst: &mut [u8; 8], a: &[u8; 8], b: &[u8; 8]) {
        // SAFETY: caller guarantees NEON and fixed eight-byte footprints.
        unsafe {
            vst1_u8(
                dst.as_mut_ptr(),
                vhadd_u8(vld1_u8(a.as_ptr()), vld1_u8(b.as_ptr())),
            );
        }
    }

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
    fn strided_reconstruction_matches_scalar_with_unaligned_rows_and_guards() {
        let mut random = 0x92a1_1234u32;
        for stride in [8, 9, 16, 31, 128] {
            for offset in 0..16 {
                let size = offset + extent(stride) + 16;
                let a: Vec<_> = (0..size).map(|i| (i * 71 + 13) as u8).collect();
                let b: Vec<_> = (0..size).map(|i| (i * 97 + 5) as u8).collect();
                for case in 0..16 {
                    let residual = if case < 8 {
                        Residual::Dc([i16::MIN, -256, -1, 0, 1, 255, 32513, i16::MAX][case])
                    } else {
                        Residual::Ac(std::array::from_fn(|_| {
                            std::array::from_fn(|_| {
                                random ^= random << 13;
                                random ^= random >> 17;
                                random ^= random << 5;
                                random as i16
                            })
                        }))
                    };
                    for second in [None, Some(&b[offset..]), Some(&a[offset..])] {
                        let mut got = vec![0x6d; size];
                        let mut want = got.clone();
                        inter_scalar(&mut want[offset..], stride, &a[offset..], second, &residual);
                        inter(&mut got[offset..], stride, &a[offset..], second, &residual);
                        assert_eq!(got, want, "stride {stride} offset {offset} case {case}");
                    }
                    let mut got = vec![0x6d; size];
                    let mut want = got.clone();
                    for row in 0..8 {
                        let start = offset + row * stride;
                        put_scalar(
                            (&mut want[start..start + 8]).try_into().unwrap(),
                            &[128; 8],
                            &residual.row(row),
                        );
                    }
                    intra(&mut got[offset..], stride, &residual);
                    assert_eq!(got, want);
                }
            }
        }
    }

    #[test]
    #[should_panic]
    fn strided_reconstruction_rejects_short_reference_before_simd() {
        inter(&mut [0; 64], 8, &[0; 63], None, &Residual::Dc(0));
    }

    #[test]
    #[should_panic]
    fn strided_reconstruction_rejects_stride_overflow_before_simd() {
        inter(&mut [0; 64], usize::MAX, &[0; 64], None, &Residual::Dc(0));
    }

    #[test]
    fn motion_average_truncates_down_for_every_pair_of_samples() {
        for a in 0..=255 {
            for b in 0..=255 {
                let mut got = [0; 8];
                average(&mut got, &[a; 8], &[b; 8]);
                assert_eq!(got, [((u16::from(a) + u16::from(b)) / 2) as u8; 8]);
            }
        }
    }

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
