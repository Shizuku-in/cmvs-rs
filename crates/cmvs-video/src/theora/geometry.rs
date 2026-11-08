use super::headers::Ident;

pub(super) struct Block {
    pub plane: usize,
    pub x: usize,
    pub y: usize,
    pub mb: usize,
    pub sb: usize,
}

pub(super) struct Geometry {
    pub blocks: Vec<Block>,
    pub macroblocks: Vec<[usize; 6]>, // Luma in raster order, Cb, Cr.
    pub raster: [Vec<usize>; 3],
    pub sizes: [(usize, usize); 3],
    pub superblocks: usize,
}

impl Geometry {
    pub fn new(ident: &Ident) -> Self {
        let (mw, mh) = (ident.width / 16, ident.height / 16);
        let mut macro_map = vec![0; mw * mh];
        let mut macroblocks = Vec::with_capacity(mw * mh);
        for sy in 0..mh.div_ceil(2) {
            for sx in 0..mw.div_ceil(2) {
                for (dx, dy) in [(0, 0), (0, 1), (1, 1), (1, 0)] {
                    let (x, y) = (sx * 2 + dx, sy * 2 + dy);
                    if x < mw && y < mh {
                        macro_map[y * mw + x] = macroblocks.len();
                        macroblocks.push([0; 6]);
                    }
                }
            }
        }
        let sizes = [
            (ident.width, ident.height),
            (ident.width / 2, ident.height / 2),
            (ident.width / 2, ident.height / 2),
        ];
        let mut raster = std::array::from_fn(|p| vec![0; sizes[p].0 * sizes[p].1 / 64]);
        let mut blocks = Vec::with_capacity(ident.width * ident.height * 3 / 128);
        let mut superblocks = 0;
        const HILBERT: [(usize, usize); 16] = [
            (0, 0),
            (1, 0),
            (1, 1),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 3),
            (1, 2),
            (2, 2),
            (2, 3),
            (3, 3),
            (3, 2),
            (3, 1),
            (2, 1),
            (2, 0),
            (3, 0),
        ];
        for plane in 0..3 {
            let (bw, bh) = (sizes[plane].0 / 8, sizes[plane].1 / 8);
            for sy in 0..bh.div_ceil(4) {
                for sx in 0..bw.div_ceil(4) {
                    for (dx, dy) in HILBERT {
                        let (x, y) = (sx * 4 + dx, sy * 4 + dy);
                        if x >= bw || y >= bh {
                            continue;
                        }
                        let (mx, my, slot) = if plane == 0 {
                            (x / 2, y / 2, (y % 2) * 2 + x % 2)
                        } else {
                            (x, y, plane + 3)
                        };
                        let mb = macro_map[my * mw + mx];
                        macroblocks[mb][slot] = blocks.len();
                        raster[plane][y * bw + x] = blocks.len();
                        blocks.push(Block {
                            plane,
                            x,
                            y,
                            mb,
                            sb: superblocks,
                        });
                    }
                    superblocks += 1;
                }
            }
        }
        Self {
            blocks,
            macroblocks,
            raster,
            sizes,
            superblocks,
        }
    }
}
