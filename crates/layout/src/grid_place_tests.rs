//! Grid auto-placement on an occupancy bitmap (#1083) places every child exactly where the plain
//! cell-by-cell search did, on randomised grids with anchored, half-anchored and spanning
//! children, while no longer testing every cell of a span at every position.

use figcraft_doc::{AutoLayout, Document, NodeId, NodeKind};

use crate::{GridCell, MAX_GRID_CELLS, MAX_TRACKS, grid_place};

/// The cell-by-cell placement `grid_place` had before #1083, kept as the reference.
fn reference(doc: &Document, al: &AutoLayout, kids: &[NodeId]) -> Vec<GridCell> {
    let cols = (al.grid_columns as usize).clamp(1, MAX_TRACKS);
    let max_rows = (MAX_GRID_CELLS / cols).clamp(1, MAX_TRACKS);
    let mut occ: Vec<Vec<bool>> = Vec::new();
    let free = |occ: &Vec<Vec<bool>>, c: usize, r: usize, cs: usize, rs: usize| {
        (r..r + rs).all(|ri| occ.get(ri).is_none_or(|row| (c..c + cs).all(|ci| !row.get(ci).copied().unwrap_or(false))))
    };
    let mark = |occ: &mut Vec<Vec<bool>>, c: usize, r: usize, cs: usize, rs: usize| {
        while occ.len() < r + rs {
            occ.push(vec![false; cols]);
        }
        for row in occ.iter_mut().skip(r).take(rs) {
            for cell in row.iter_mut().skip(c).take(cs) {
                *cell = true;
            }
        }
    };
    let spans = |id: NodeId| {
        let cl = doc.node(id).unwrap().child_layout.clone();
        let cs = (cl.column_span() as usize).min(cols);
        let rs = (cl.row_span() as usize).min(max_rows);
        (cl.grid_column.map(|c| (c as usize).min(cols - cs)), cl.grid_row.map(|r| (r as usize).min(max_rows - rs)), cs, rs)
    };
    let mut placed: Vec<Option<GridCell>> = vec![None; kids.len()];
    for (i, id) in kids.iter().enumerate() {
        if let (Some(c), Some(r), cs, rs) = spans(*id) {
            mark(&mut occ, c, r, cs, rs);
            placed[i] = Some(GridCell { id: *id, column: c, row: r, column_span: cs, row_span: rs });
        }
    }
    let mut cursor = (0usize, 0usize);
    for (i, id) in kids.iter().enumerate() {
        if placed[i].is_some() {
            continue;
        }
        let (col, row, cs, rs) = spans(*id);
        let last_row = max_rows - rs;
        let (c, r) = match (col, row) {
            (Some(c), _) => (0..=last_row).find(|r| free(&occ, c, *r, cs, rs)).map(|r| (c, r)).unwrap_or((c, last_row)),
            (None, Some(r)) => (0..=cols - cs).find(|c| free(&occ, *c, r, cs, rs)).map(|c| (c, r)).unwrap_or((0, r)),
            (None, None) => {
                let mut found = None;
                let (mut r, mut c) = cursor;
                while r <= last_row {
                    if c + cs <= cols && free(&occ, c, r, cs, rs) {
                        found = Some((c, r));
                        break;
                    }
                    c += 1;
                    if c + cs > cols {
                        c = 0;
                        r += 1;
                    }
                }
                let (c, r) = found.unwrap_or((0, last_row));
                cursor = (r, c + cs);
                (c, r)
            }
        };
        mark(&mut occ, c, r, cs, rs);
        placed[i] = Some(GridCell { id: *id, column: c, row: r, column_span: cs, row_span: rs });
    }
    placed.into_iter().flatten().collect()
}

/// A small deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

/// A grid of `cols` columns with `n` children, each anchored, half-anchored or free, with spans
/// up to `span` (sometimes past the grid, which clamps them).
fn grid(rng: &mut Rng, cols: u32, n: usize, span: u64) -> (Document, AutoLayout, Vec<NodeId>) {
    let mut d = Document::new("grid");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let kids: Vec<NodeId> = (0..n)
        .map(|_| {
            let id = d.create(frame, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
            let anchor = rng.below(4);
            let (column, row) = (rng.below(u64::from(cols) + 3) as u32, rng.below(40) as u32);
            let cl = &mut d.get_mut(id).unwrap().child_layout;
            cl.grid_column = matches!(anchor, 1 | 3).then_some(column);
            cl.grid_row = matches!(anchor, 2 | 3).then_some(row);
            cl.grid_column_span = rng.below(span + 1) as u32;
            cl.grid_row_span = rng.below(span + 1) as u32;
            id
        })
        .collect();
    let al = AutoLayout { grid_columns: cols, ..Default::default() };
    (d, al, kids)
}

#[test]
fn placements_match_the_cell_by_cell_search_on_random_grids() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..400 {
        let cols = [1, 2, 3, 7, 63, 64, 65, 130][case % 8];
        let (n, span) = (rng.below(60) as usize + 1, [1, 3, 9, 70][case % 4]);
        let (d, al, kids) = grid(&mut rng, cols, n, span);
        let (cells, _) = grid_place(&d, &al, &kids);
        assert_eq!(cells, reference(&d, &al, &kids), "case {case}: {cols} columns, {n} children, spans up to {span}");
    }
}

#[test]
fn a_full_grid_puts_the_rest_on_its_last_rows_as_before() {
    // 1000 columns leave 1000 rows; a child too tall for what is left goes to the last row.
    let mut rng = Rng(7);
    let (mut d, al, kids) = grid(&mut rng, 1000, 30, 1);
    for (i, id) in kids.iter().enumerate() {
        let cl = &mut d.get_mut(*id).unwrap().child_layout;
        (cl.grid_column, cl.grid_row, cl.grid_column_span, cl.grid_row_span) = (Some(0), None, 1000, if i % 2 == 0 { 400 } else { 1 });
    }
    let (cells, _) = grid_place(&d, &al, &kids);
    assert_eq!(cells, reference(&d, &al, &kids));
    assert!(cells.iter().any(|c| c.row == 600 && c.row_span == 400), "a tall child past the free rows sits at the last row it fits");
}
