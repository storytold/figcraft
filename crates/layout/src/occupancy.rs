//! Which cells of a layout grid are taken, one bit per cell (#1083). Grid placement asks whether a
//! span of cells is free a 64-bit word at a time and jumps past the cell that blocks it, instead of
//! testing every cell of the span at every position.

/// The taken cells of a grid of `cols` columns, row by row.
pub(crate) struct Occupancy {
    cols: usize,
    words: usize,
    bits: Vec<u64>,
}

/// The bits of columns `c..end` that fall in word `w`.
fn mask(w: usize, c: usize, end: usize) -> u64 {
    let lo = c.max(w * 64) - w * 64;
    let hi = end.min((w + 1) * 64) - w * 64;
    match hi.saturating_sub(lo) {
        0 => 0,
        64 => u64::MAX,
        n => ((1u64 << n) - 1) << lo,
    }
}

/// The last taken column of `c..c + cs` in `row`, if any.
fn last_taken(row: &[u64], c: usize, cs: usize) -> Option<usize> {
    let end = c + cs;
    if cs == 0 {
        return None;
    }
    (c / 64..=(end - 1) / 64).rev().find_map(|w| {
        let v = row.get(w).copied().unwrap_or(0) & mask(w, c, end);
        (v != 0).then(|| w * 64 + 63 - v.leading_zeros() as usize)
    })
}

impl Occupancy {
    /// An empty grid of `cols` columns. Rows are stored only once something takes a cell in them
    /// (or a row below them), so an empty or sparse grid allocates little.
    pub(crate) fn new(cols: usize) -> Self {
        Self { cols, words: cols.div_ceil(64), bits: Vec::new() }
    }

    fn row(&self, r: usize) -> &[u64] {
        self.bits.get(r * self.words..(r + 1) * self.words).unwrap_or(&[])
    }

    /// Take columns `c..c + cs` of rows `r..r + rs`.
    pub(crate) fn mark(&mut self, c: usize, r: usize, cs: usize, rs: usize) {
        let end = c + cs;
        if cs == 0 || rs == 0 {
            return;
        }
        let need = (r + rs) * self.words;
        if self.bits.len() < need {
            self.bits.resize(need, 0);
        }
        for ri in r..r + rs {
            for w in c / 64..=(end - 1) / 64 {
                if let Some(word) = self.bits.get_mut(ri * self.words + w) {
                    *word |= mask(w, c, end);
                }
            }
        }
    }

    /// The first column from `from` where columns `c..c + cs` are free in rows `r..r + rs` and fit
    /// the grid (`scratch`: a buffer for the rows' union).
    pub(crate) fn first_column(&self, r: usize, rs: usize, from: usize, cs: usize, scratch: &mut Vec<u64>) -> Option<usize> {
        scratch.clear();
        scratch.resize(self.words, 0);
        for ri in r..r + rs {
            for (u, w) in scratch.iter_mut().zip(self.row(ri)) {
                *u |= w;
            }
        }
        let mut c = from;
        while c + cs <= self.cols {
            match last_taken(scratch, c, cs) {
                None => return Some(c),
                // No span starting at or before the taken column can include it free.
                Some(t) => c = t + 1,
            }
        }
        None
    }

    /// The first row from `from` (at most `last`) where columns `c..c + cs` are free in `rs` rows
    /// in a row.
    pub(crate) fn first_row(&self, c: usize, cs: usize, rs: usize, from: usize, last: usize) -> Option<usize> {
        let mut run = 0;
        for r in from..last + rs {
            if last_taken(self.row(r), c, cs).is_some() {
                run = 0;
                continue;
            }
            run += 1;
            if run == rs {
                return Some(r + 1 - rs);
            }
        }
        None
    }
}
