//! Integer reconstruction kernels. All packet parsing stays in safe Rust.

#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) enum Residual {
    Dc(i16),
    Ac([[i16; 8]; 8]),
}

#[cfg(any(test, not(target_arch = "x86_64")))]
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

#[inline]
pub(super) fn intra_dc(dst: &mut [u8], stride: usize, dc: i16) {
    let dst = &mut dst[..extent(stride)];
    let pixel = (128 + i32::from(dc)).clamp(0, 255) as u8;
    for row in 0..8 {
        dst[row * stride..row * stride + 8].fill(pixel);
    }
}

#[inline]
pub(super) fn inter_dc(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    dc: i16,
) {
    let n = extent(stride);
    let dst = &mut dst[..n];
    let first = &first[..n];
    let second = second.map(|s| &s[..n]);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked slices cover all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::inter_dc(dst, stride, first, second, dc);
    }
    #[cfg(not(target_arch = "x86_64"))]
    inter_scalar(dst, stride, first, second, &Residual::Dc(dc));
}

#[inline]
pub(super) fn intra_sparse2(dst: &mut [u8], stride: usize, c0: i16, c1: i16, c8: i16) {
    let dst = &mut dst[..extent(stride)];
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked destination covers all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::intra_sparse2(dst, stride, c0, c1, c8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut coeffs = [0i16; 64];
        coeffs[0] = c0;
        coeffs[1] = c1;
        coeffs[8] = c8;
        let ac = idct_partial::<2>(&coeffs);
        intra(dst, stride, &Residual::Ac(ac));
    }
}

#[inline]
pub(super) fn inter_sparse2(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    c0: i16,
    c1: i16,
    c8: i16,
) {
    let n = extent(stride);
    let dst = &mut dst[..n];
    let first = &first[..n];
    let second = second.map(|s| &s[..n]);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked slices cover all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::inter_sparse2(dst, stride, first, second, c0, c1, c8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut coeffs = [0i16; 64];
        coeffs[0] = c0;
        coeffs[1] = c1;
        coeffs[8] = c8;
        let ac = idct_partial::<2>(&coeffs);
        inter_scalar(dst, stride, first, second, &Residual::Ac(ac));
    }
}

#[inline]
pub(super) fn intra_sparse4(dst: &mut [u8], stride: usize, s: &[i16; 10]) {
    let dst = &mut dst[..extent(stride)];
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked destination covers all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::intra_sparse4(dst, stride, s);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let coeffs = sparse4_array(s);
        let ac = idct_partial::<4>(&coeffs);
        intra(dst, stride, &Residual::Ac(ac));
    }
}

#[inline]
pub(super) fn inter_sparse4(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    s: &[i16; 10],
) {
    let n = extent(stride);
    let dst = &mut dst[..n];
    let first = &first[..n];
    let second = second.map(|s| &s[..n]);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked slices cover all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::inter_sparse4(dst, stride, first, second, s);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let coeffs = sparse4_array(s);
        let ac = idct_partial::<4>(&coeffs);
        inter_scalar(dst, stride, first, second, &Residual::Ac(ac));
    }
}

#[inline]
pub(super) fn intra_full(dst: &mut [u8], stride: usize, coeffs: &[i16; 64]) {
    let dst = &mut dst[..extent(stride)];
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked destination covers all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::intra_full(dst, stride, coeffs);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let ac = idct_scalar(coeffs);
        intra(dst, stride, &Residual::Ac(ac));
    }
}

#[inline]
pub(super) fn inter_full(
    dst: &mut [u8],
    stride: usize,
    first: &[u8],
    second: Option<&[u8]>,
    coeffs: &[i16; 64],
) {
    let n = extent(stride);
    let dst = &mut dst[..n];
    let first = &first[..n];
    let second = second.map(|s| &s[..n]);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline; checked slices cover all 8 strided rows.
    #[allow(unsafe_code)]
    unsafe {
        x86::inter_full(dst, stride, first, second, coeffs);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let ac = idct_scalar(coeffs);
        inter_scalar(dst, stride, first, second, &Residual::Ac(ac));
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn sparse4_array(s: &[i16; 10]) -> [i16; 64] {
    let mut coeffs = [0i16; 64];
    coeffs[0] = s[0];
    coeffs[1] = s[1];
    coeffs[8] = s[2];
    coeffs[16] = s[3];
    coeffs[9] = s[4];
    coeffs[2] = s[5];
    coeffs[3] = s[6];
    coeffs[10] = s[7];
    coeffs[17] = s[8];
    coeffs[24] = s[9];
    coeffs
}

pub(super) fn idct_sparse2(c0: i16, c1: i16, c8: i16) -> [[i16; 8]; 8] {
    let mut coeffs = [0i16; 64];
    coeffs[0] = c0;
    coeffs[1] = c1;
    coeffs[8] = c8;
    idct(&coeffs, 3)
}

pub(super) fn idct_sparse4(s: &[i16; 10]) -> [[i16; 8]; 8] {
    let mut coeffs = [0i16; 64];
    coeffs[0] = s[0];
    coeffs[1] = s[1];
    coeffs[8] = s[2];
    coeffs[16] = s[3];
    coeffs[9] = s[4];
    coeffs[2] = s[5];
    coeffs[3] = s[6];
    coeffs[10] = s[7];
    coeffs[17] = s[8];
    coeffs[24] = s[9];
    idct(&coeffs, 10)
}

/// Reconstruct one validated strided 8x8 rectangle. The slice checks cover every
/// row before entering SIMD, so kernels do not repeat eight bounds checks.
#[cfg(test)]
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

#[cfg(any(test, not(target_arch = "x86_64")))]
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
fn idct_1d<const N: usize>(y: [i16; N]) -> [i16; 8] {
    let y = std::array::from_fn::<_, 8, _>(|i| if i < N { i32::from(y[i]) } else { 0 });
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
fn idct_partial<const N: usize>(coeffs: &[i16; 64]) -> [[i16; 8]; 8] {
    let mut rows = [[0i16; 8]; 8];
    for (src, dst) in coeffs.as_chunks::<8>().0.iter().zip(&mut rows).take(N) {
        *dst = idct_1d::<N>(src[..N].try_into().unwrap());
    }
    for x in 0..8 {
        let column = idct_1d::<N>(std::array::from_fn(|y| rows[y][x]));
        for y in 0..8 {
            rows[y][x] = ((i32::from(column[y]) + 8) >> 4) as i16;
        }
    }
    rows
}

#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) fn idct_scalar(coeffs: &[i16; 64]) -> [[i16; 8]; 8] {
    idct_partial::<8>(coeffs)
}

#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) fn idct_sparse_scalar(coeffs: &[i16; 64], ncoeffs: u8) -> [[i16; 8]; 8] {
    match ncoeffs {
        // The first 3/10 zig-zag coefficients fit inside a 2x2/4x4 corner.
        0..=3 => idct_partial::<2>(coeffs),
        4..=10 => idct_partial::<4>(coeffs),
        _ => idct_scalar(coeffs),
    }
}

#[inline]
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
        idct_sparse_scalar(coeffs, ncoeffs)
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

#[inline]
pub(super) fn filter_h(pixels: &mut [u8], start: usize, stride: usize, limit: u8) {
    let end = start
        .checked_add(
            stride
                .checked_mul(3)
                .and_then(|n| n.checked_add(8))
                .expect("Theora filter_h stride overflow"),
        )
        .expect("Theora filter_h offset overflow");
    let slice = &mut pixels[start..end];
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline on x86-64. `slice` covers 4 rows of 8 bytes at
    // `stride` intervals.
    #[allow(unsafe_code)]
    unsafe {
        x86::filter_h(slice.as_mut_ptr(), stride, limit);
    }
    #[cfg(not(target_arch = "x86_64"))]
    filter_scalar(slice, stride, 1, limit);
}

#[inline]
pub(super) fn filter_v(pixels: &mut [u8], start: usize, stride: usize, limit: u8) {
    let end = start
        .checked_add(
            stride
                .checked_mul(7)
                .and_then(|n| n.checked_add(4))
                .expect("Theora filter_v stride overflow"),
        )
        .expect("Theora filter_v offset overflow");
    let slice = &mut pixels[start..end];
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is baseline on x86-64. `slice` covers 8 rows of 4 bytes at
    // `stride` intervals.
    #[allow(unsafe_code)]
    unsafe {
        x86::filter_v(slice.as_mut_ptr(), stride, limit);
    }
    #[cfg(not(target_arch = "x86_64"))]
    filter_scalar(slice, 1, stride, limit);
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn filter_scalar(slice: &mut [u8], step: usize, advance: usize, limit: u8) {
    let limit = i32::from(limit);
    for i in 0..8 {
        let a = advance * i;
        let p0 = i32::from(slice[a]);
        let p1 = i32::from(slice[a + step]);
        let p2 = i32::from(slice[a + step * 2]);
        let p3 = i32::from(slice[a + step * 3]);
        let r = (p0 - 3 * p1 + 3 * p2 - p3 + 4) >> 3;
        let delta = r.signum() * r.abs().min((2 * limit - r.abs()).max(0));
        slice[a + step] = (p1 + delta).clamp(0, 255) as u8;
        slice[a + step * 2] = (p2 - delta).clamp(0, 255) as u8;
    }
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod x86 {
    #[cfg(test)]
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

    #[inline]
    unsafe fn apply_rows<const INTRA: bool, const HALF: bool>(
        dst: *mut u8,
        stride: usize,
        first: *const u8,
        second: *const u8,
        rows: [__m128i; 8],
    ) {
        // SAFETY: callers check all strided 8x8 footprints before dispatch.
        unsafe {
            let zero = _mm_setzero_si128();
            let intra_pred = _mm_set1_epi16(128);
            let ones = _mm_set1_epi8(1);
            for (row, r) in rows.into_iter().enumerate() {
                let pred = if INTRA {
                    intra_pred
                } else {
                    let a = _mm_loadl_epi64(first.add(row * stride).cast());
                    let p = if HALF {
                        let b = _mm_loadl_epi64(second.add(row * stride).cast());
                        let odd = _mm_and_si128(_mm_xor_si128(a, b), ones);
                        _mm_sub_epi8(_mm_avg_epu8(a, b), odd)
                    } else {
                        a
                    };
                    _mm_unpacklo_epi8(p, zero)
                };
                let pixels = _mm_packus_epi16(_mm_adds_epi16(pred, r), zero);
                _mm_storel_epi64(dst.add(row * stride).cast(), pixels);
            }
        }
    }

    #[inline]
    pub(super) unsafe fn inter_dc(
        dst: &mut [u8],
        stride: usize,
        first: &[u8],
        second: Option<&[u8]>,
        dc: i16,
    ) {
        // SAFETY: wrapper checked all footprints and SSE2 availability.
        unsafe {
            let d = dst.as_mut_ptr();
            let a = first.as_ptr();
            match second {
                None => block::<false, false, true>(
                    d,
                    stride,
                    a,
                    std::ptr::null(),
                    dc,
                    std::ptr::null(),
                ),
                Some(b) => {
                    block::<false, true, true>(d, stride, a, b.as_ptr(), dc, std::ptr::null())
                }
            }
        }
    }

    #[inline]
    pub(super) unsafe fn intra_sparse2(dst: &mut [u8], stride: usize, c0: i16, c1: i16, c8: i16) {
        // SAFETY: wrapper checked dst and SSE2.
        unsafe {
            let rows = idct_sparse2_rows(c0, c1, c8);
            apply_rows::<true, false>(
                dst.as_mut_ptr(),
                stride,
                std::ptr::null(),
                std::ptr::null(),
                rows,
            );
        }
    }

    #[inline]
    pub(super) unsafe fn inter_sparse2(
        dst: &mut [u8],
        stride: usize,
        first: &[u8],
        second: Option<&[u8]>,
        c0: i16,
        c1: i16,
        c8: i16,
    ) {
        // SAFETY: wrapper checked all footprints and SSE2 availability.
        unsafe {
            let rows = idct_sparse2_rows(c0, c1, c8);
            let d = dst.as_mut_ptr();
            let a = first.as_ptr();
            match second {
                None => apply_rows::<false, false>(d, stride, a, std::ptr::null(), rows),
                Some(b) => apply_rows::<false, true>(d, stride, a, b.as_ptr(), rows),
            }
        }
    }

    #[inline]
    pub(super) unsafe fn intra_sparse4(dst: &mut [u8], stride: usize, s: &[i16; 10]) {
        // SAFETY: wrapper checked dst and SSE2.
        unsafe {
            let rows = idct_sparse4_rows(s);
            apply_rows::<true, false>(
                dst.as_mut_ptr(),
                stride,
                std::ptr::null(),
                std::ptr::null(),
                rows,
            );
        }
    }

    #[inline]
    pub(super) unsafe fn inter_sparse4(
        dst: &mut [u8],
        stride: usize,
        first: &[u8],
        second: Option<&[u8]>,
        s: &[i16; 10],
    ) {
        // SAFETY: wrapper checked all footprints and SSE2 availability.
        unsafe {
            let rows = idct_sparse4_rows(s);
            let d = dst.as_mut_ptr();
            let a = first.as_ptr();
            match second {
                None => apply_rows::<false, false>(d, stride, a, std::ptr::null(), rows),
                Some(b) => apply_rows::<false, true>(d, stride, a, b.as_ptr(), rows),
            }
        }
    }

    #[inline]
    pub(super) unsafe fn intra_full(dst: &mut [u8], stride: usize, coeffs: &[i16; 64]) {
        // SAFETY: wrapper checked dst and SSE2.
        unsafe {
            let rows = idct_full_rows(coeffs);
            apply_rows::<true, false>(
                dst.as_mut_ptr(),
                stride,
                std::ptr::null(),
                std::ptr::null(),
                rows,
            );
        }
    }

    #[inline]
    pub(super) unsafe fn inter_full(
        dst: &mut [u8],
        stride: usize,
        first: &[u8],
        second: Option<&[u8]>,
        coeffs: &[i16; 64],
    ) {
        // SAFETY: wrapper checked all footprints and SSE2 availability.
        unsafe {
            let rows = idct_full_rows(coeffs);
            let d = dst.as_mut_ptr();
            let a = first.as_ptr();
            match second {
                None => apply_rows::<false, false>(d, stride, a, std::ptr::null(), rows),
                Some(b) => apply_rows::<false, true>(d, stride, a, b.as_ptr(), rows),
            }
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[inline]
    unsafe fn round_rows(rows: [__m128i; 8]) -> [__m128i; 8] {
        // SAFETY: register-only SSE2 arithmetic; widened (X+8)>>4 without signed overflow.
        unsafe {
            let ones = _mm_set1_epi16(1);
            rows.map(|v| {
                _mm_add_epi16(
                    _mm_srai_epi16(v, 4),
                    _mm_and_si128(_mm_srli_epi16(v, 3), ones),
                )
            })
        }
    }

    #[inline]
    unsafe fn idct_sparse2_rows(c0: i16, c1: i16, c8: i16) -> [__m128i; 8] {
        // SAFETY: register-only SSE2 arithmetic.
        unsafe {
            let zero = _mm_setzero_si128();
            let y0 =
                _mm_cvtsi32_si128((u32::from(c0 as u16) | (u32::from(c8 as u16) << 16)) as i32);
            let y1 = _mm_cvtsi32_si128(i32::from(c1 as u16));
            let rows = sparse_rows(
                transform([y0, y1, zero, zero, zero, zero, zero, zero]),
                false,
            );
            round_rows(transform(rows))
        }
    }

    #[inline]
    unsafe fn idct_sparse4_rows(s: &[i16; 10]) -> [__m128i; 8] {
        // SAFETY: register-only SSE2 arithmetic.
        unsafe {
            let zero = _mm_setzero_si128();
            let y0 = _mm_set_epi16(0, 0, 0, 0, s[9], s[3], s[2], s[0]);
            let y1 = _mm_set_epi16(0, 0, 0, 0, 0, s[8], s[4], s[1]);
            let y2 = _mm_set_epi16(0, 0, 0, 0, 0, 0, s[7], s[5]);
            let y3 = _mm_cvtsi32_si128(i32::from(s[6] as u16));
            let rows = sparse_rows(transform([y0, y1, y2, y3, zero, zero, zero, zero]), true);
            round_rows(transform(rows))
        }
    }

    #[inline]
    unsafe fn idct_full_rows(coeffs: &[i16; 64]) -> [__m128i; 8] {
        // SAFETY: fixed 8x8 i16 array covers all 8 unaligned loads.
        unsafe {
            let rows = std::array::from_fn(|i| _mm_loadu_si128(coeffs.as_ptr().add(i * 8).cast()));
            round_rows(transform(transpose(transform(transpose(rows)))))
        }
    }

    pub(super) unsafe fn idct(coeffs: &[i16; 64], ncoeffs: u8) -> [[i16; 8]; 8] {
        // SAFETY: SSE2 and fixed 8x8 footprints are guaranteed by the caller.
        // Each transform runs eight independent 1D IDCTs, one per lane.
        unsafe {
            let rows = if ncoeffs <= 3 {
                idct_sparse2_rows(coeffs[0], coeffs[1], coeffs[8])
            } else if ncoeffs <= 10 {
                idct_sparse4_rows(&[
                    coeffs[0], coeffs[1], coeffs[8], coeffs[16], coeffs[9], coeffs[2], coeffs[3],
                    coeffs[10], coeffs[17], coeffs[24],
                ])
            } else {
                idct_full_rows(coeffs)
            };
            let mut out = [[0i16; 8]; 8];
            for (v, row) in rows.into_iter().zip(&mut out) {
                _mm_storeu_si128(row.as_mut_ptr().cast(), v);
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

    #[inline]
    unsafe fn filter_edge_8(
        p0: __m128i,
        p1: __m128i,
        p2: __m128i,
        p3: __m128i,
        limit: u8,
    ) -> (__m128i, __m128i) {
        // SAFETY: register-only SSE2 arithmetic. All intermediate values fit in
        // i16 without overflow (|(p0 - p3) + 3*(p2 - p1) + 4| <= 1024).
        unsafe {
            let zero = _mm_setzero_si128();
            let diff = _mm_sub_epi16(p2, p1);
            let sum = _mm_add_epi16(
                _mm_sub_epi16(p0, p3),
                _mm_add_epi16(
                    _mm_add_epi16(diff, _mm_slli_epi16(diff, 1)),
                    _mm_set1_epi16(4),
                ),
            );
            let r = _mm_srai_epi16(sum, 3);
            let abs_r = _mm_max_epi16(r, _mm_sub_epi16(zero, r));
            let two_limit = _mm_set1_epi16(2 * i16::from(limit));
            let bound = _mm_max_epi16(zero, _mm_sub_epi16(two_limit, abs_r));
            let abs_delta = _mm_min_epi16(abs_r, bound);
            let sign = _mm_srai_epi16(r, 15);
            let delta = _mm_sub_epi16(_mm_xor_si128(abs_delta, sign), sign);
            (
                _mm_packus_epi16(_mm_add_epi16(p1, delta), zero),
                _mm_packus_epi16(_mm_sub_epi16(p2, delta), zero),
            )
        }
    }

    #[inline]
    pub(super) unsafe fn filter_h(ptr: *mut u8, stride: usize, limit: u8) {
        // SAFETY: caller validated 4 rows of 8 bytes at `stride` intervals.
        unsafe {
            let zero = _mm_setzero_si128();
            let r0 = ptr;
            let r1 = ptr.add(stride);
            let r2 = ptr.add(stride * 2);
            let r3 = ptr.add(stride * 3);
            let p0 = _mm_unpacklo_epi8(_mm_loadl_epi64(r0.cast()), zero);
            let p1 = _mm_unpacklo_epi8(_mm_loadl_epi64(r1.cast()), zero);
            let p2 = _mm_unpacklo_epi8(_mm_loadl_epi64(r2.cast()), zero);
            let p3 = _mm_unpacklo_epi8(_mm_loadl_epi64(r3.cast()), zero);
            let (p1_new, p2_new) = filter_edge_8(p0, p1, p2, p3, limit);
            _mm_storel_epi64(r1.cast(), p1_new);
            _mm_storel_epi64(r2.cast(), p2_new);
        }
    }

    #[inline]
    pub(super) unsafe fn filter_v(ptr: *mut u8, stride: usize, limit: u8) {
        // SAFETY: caller validated 8 rows of 4 bytes at `stride` intervals.
        unsafe {
            let load4 =
                |r: usize| _mm_cvtsi32_si128(ptr.add(r * stride).cast::<i32>().read_unaligned());
            let t01 = _mm_unpacklo_epi8(load4(0), load4(1));
            let t23 = _mm_unpacklo_epi8(load4(2), load4(3));
            let t45 = _mm_unpacklo_epi8(load4(4), load4(5));
            let t67 = _mm_unpacklo_epi8(load4(6), load4(7));
            let q0 = _mm_unpacklo_epi16(t01, t23);
            let q1 = _mm_unpacklo_epi16(t45, t67);
            let ab = _mm_unpacklo_epi32(q0, q1);
            let cd = _mm_unpackhi_epi32(q0, q1);
            let zero = _mm_setzero_si128();
            let p0 = _mm_unpacklo_epi8(ab, zero);
            let p1 = _mm_unpackhi_epi8(ab, zero);
            let p2 = _mm_unpacklo_epi8(cd, zero);
            let p3 = _mm_unpackhi_epi8(cd, zero);
            let (p1_new, p2_new) = filter_edge_8(p0, p1, p2, p3, limit);
            let p12 = _mm_unpacklo_epi8(p1_new, p2_new);
            let lo = _mm_cvtsi128_si64(p12) as u64;
            let hi = _mm_cvtsi128_si64(_mm_srli_si128(p12, 8)) as u64;
            for r in 0..4 {
                ptr.add(r * stride + 1)
                    .cast::<u16>()
                    .write_unaligned((lo >> (r * 16)) as u16);
                ptr.add((r + 4) * stride + 1)
                    .cast::<u16>()
                    .write_unaligned((hi >> (r * 16)) as u16);
            }
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

    #[test]
    fn loop_filter_matches_scalar_across_strides_limits_and_extremes() {
        let mut seed = 0x7412_a93du32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        };
        for stride in [8, 9, 16, 31, 128] {
            for offset in 0..16 {
                let len = offset + stride * 7 + 8 + 16;
                let base: Vec<u8> = (0..len).map(|_| next()).collect();
                for limit in [0, 1, 5, 31, 63, 127, 255] {
                    let mut got = base.clone();
                    let mut want = base.clone();
                    filter_h(&mut got, offset, stride, limit);
                    filter_scalar(&mut want[offset..], stride, 1, limit);
                    assert_eq!(
                        got, want,
                        "filter_h stride={stride} offset={offset} limit={limit}"
                    );

                    let mut got = base.clone();
                    let mut want = base.clone();
                    filter_v(&mut got, offset, stride, limit);
                    filter_scalar(&mut want[offset..], 1, stride, limit);
                    assert_eq!(
                        got, want,
                        "filter_v stride={stride} offset={offset} limit={limit}"
                    );
                }
            }
        }
        // Check extreme pixel transitions [0, 255, 0, 255] and [255, 0, 255, 0].
        for limit in [0, 1, 64, 127, 255] {
            let mut got = [0u8; 64];
            for r in 0..8 {
                got[r * 8..r * 8 + 4].copy_from_slice(if r % 2 == 0 {
                    &[0, 255, 0, 255]
                } else {
                    &[255, 0, 255, 0]
                });
            }
            let mut want = got;
            filter_v(&mut got, 0, 8, limit);
            filter_scalar(&mut want, 1, 8, limit);
            assert_eq!(got, want);
        }
    }

    #[test]
    #[should_panic]
    fn filter_v_rejects_short_slice_before_simd() {
        filter_v(&mut [0; 8 * 7 + 3], 0, 8, 10);
    }

    #[test]
    #[should_panic]
    fn filter_h_rejects_short_slice_before_simd() {
        filter_h(&mut [0; 8 * 3 + 7], 0, 8, 10);
    }

    #[test]
    fn fused_kernels_match_reference_intra_and_inter() {
        let mut seed = 0x5f37_59dfu32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let stride = 24;
        let first: Vec<u8> = (0..stride * 12).map(|_| next() as u8).collect();
        let second: Vec<u8> = (0..stride * 12).map(|_| next() as u8).collect();
        for offset in [0, 3, 7] {
            let mut got = vec![0u8; stride * 12];
            let mut want = vec![0u8; stride * 12];
            for dc in [-512i16, -33, 0, 41, 511] {
                got.fill(0x5a);
                want.fill(0x5a);
                intra_dc(&mut got[offset..], stride, dc);
                intra(&mut want[offset..], stride, &Residual::Dc(dc));
                assert_eq!(got, want);
                for sec in [None, Some(&second[offset..])] {
                    got.fill(0x5a);
                    want.fill(0x5a);
                    inter_dc(&mut got[offset..], stride, &first[offset..], sec, dc);
                    inter(
                        &mut want[offset..],
                        stride,
                        &first[offset..],
                        sec,
                        &Residual::Dc(dc),
                    );
                    assert_eq!(got, want);
                }
            }
            for _ in 0..8 {
                let c0 = (next() % 1025) as i16 - 512;
                let c1 = (next() % 1025) as i16 - 512;
                let c8 = (next() % 1025) as i16 - 512;
                let mut coeffs = [0i16; 64];
                coeffs[0] = c0;
                coeffs[1] = c1;
                coeffs[8] = c8;
                let res = Residual::Ac(idct_scalar(&coeffs));
                got.fill(0x3c);
                want.fill(0x3c);
                intra_sparse2(&mut got[offset..], stride, c0, c1, c8);
                intra(&mut want[offset..], stride, &res);
                assert_eq!(got, want);
                for sec in [None, Some(&second[offset..])] {
                    got.fill(0x3c);
                    want.fill(0x3c);
                    inter_sparse2(
                        &mut got[offset..],
                        stride,
                        &first[offset..],
                        sec,
                        c0,
                        c1,
                        c8,
                    );
                    inter(&mut want[offset..], stride, &first[offset..], sec, &res);
                    assert_eq!(got, want);
                }

                let mut s = [0i16; 10];
                for slot in &mut s {
                    *slot = (next() % 1025) as i16 - 512;
                }
                let zz4 = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24];
                let mut coeffs4 = [0i16; 64];
                for (&val, &ci) in s.iter().zip(&zz4) {
                    coeffs4[ci] = val;
                }
                let res4 = Residual::Ac(idct_scalar(&coeffs4));
                got.fill(0x3c);
                want.fill(0x3c);
                intra_sparse4(&mut got[offset..], stride, &s);
                intra(&mut want[offset..], stride, &res4);
                assert_eq!(got, want);
                for sec in [None, Some(&second[offset..])] {
                    got.fill(0x3c);
                    want.fill(0x3c);
                    inter_sparse4(&mut got[offset..], stride, &first[offset..], sec, &s);
                    inter(&mut want[offset..], stride, &first[offset..], sec, &res4);
                    assert_eq!(got, want);
                }

                let mut full = [0i16; 64];
                for slot in &mut full {
                    *slot = (next() % 1025) as i16 - 512;
                }
                let res_full = Residual::Ac(idct_scalar(&full));
                got.fill(0x3c);
                want.fill(0x3c);
                intra_full(&mut got[offset..], stride, &full);
                intra(&mut want[offset..], stride, &res_full);
                assert_eq!(got, want);
                for sec in [None, Some(&second[offset..])] {
                    got.fill(0x3c);
                    want.fill(0x3c);
                    inter_full(&mut got[offset..], stride, &first[offset..], sec, &full);
                    inter(
                        &mut want[offset..],
                        stride,
                        &first[offset..],
                        sec,
                        &res_full,
                    );
                    assert_eq!(got, want);
                }
            }
        }
    }
}
