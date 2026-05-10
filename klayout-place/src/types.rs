//! Placement data model.

use klayout_core::Point;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct Cell {
    pub name: SmolStr,
    pub width: i64,
    pub height: i64,
    /// Current placement (initialised to (0,0); updated by stages).
    pub position: Point,
    /// Fixed cells (primary I/O pads, pre-placed macros) don't move.
    pub fixed: bool,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub name: SmolStr,
    /// Indices into the `Placement::cells` array.
    pub cell_indices: Vec<usize>,
    /// Net weight (clocks / criticals get higher weight; default 1.0).
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct Row {
    /// Row's bottom-left corner.
    pub origin: Point,
    /// Site (placement-grid) width along x.
    pub site_width: i64,
    /// Number of sites in this row.
    pub num_sites: u32,
    /// Row height (= cell height for legal cells).
    pub height: i64,
}

impl Row {
    pub fn x_max(&self) -> i64 {
        self.origin.x + self.site_width * self.num_sites as i64
    }

    pub fn y_max(&self) -> i64 {
        self.origin.y + self.height
    }

    pub fn snap_x(&self, x: i64) -> i64 {
        let off = x - self.origin.x;
        let q = (off + self.site_width / 2) / self.site_width;
        let snapped = self.origin.x + q * self.site_width;
        snapped
            .max(self.origin.x)
            .min(self.x_max() - self.site_width)
    }
}

#[derive(Default, Clone, Debug)]
pub struct Placement {
    pub cells: Vec<Cell>,
    pub nets: Vec<Net>,
    pub rows: Vec<Row>,
}

impl Placement {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_cell(&mut self, c: Cell) -> usize {
        self.cells.push(c);
        self.cells.len() - 1
    }

    pub fn add_net(&mut self, n: Net) {
        self.nets.push(n);
    }

    pub fn add_row(&mut self, r: Row) {
        self.rows.push(r);
    }

    /// Half-perimeter wirelength: sum over nets of `bbox(cells).w +
    /// bbox(cells).h`.
    pub fn hpwl(&self) -> i64 {
        let mut total = 0i64;
        for n in &self.nets {
            if n.cell_indices.len() < 2 {
                continue;
            }
            let mut min_x = i64::MAX;
            let mut max_x = i64::MIN;
            let mut min_y = i64::MAX;
            let mut max_y = i64::MIN;
            for &ci in &n.cell_indices {
                let c = &self.cells[ci];
                let cx = c.position.x + c.width / 2;
                let cy = c.position.y + c.height / 2;
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);
            }
            total = total.saturating_add((max_x - min_x) + (max_y - min_y));
        }
        total
    }
}
