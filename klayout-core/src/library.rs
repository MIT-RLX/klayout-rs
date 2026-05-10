//! Library: top-level container.
//!
//! Owns the layer table, the cell arena, and the dedup caches. `Send + Sync`
//! by construction so `Library::build` can be called concurrently from
//! multiple threads. Internal state uses `RwLock` for the layer table and
//! cell arena (append-only, so reads dominate) and `DashMap` for dedup
//! lookups.

use crate::cell::{Cell, CellBuilder, CellId, CellName};
use crate::component::{BuildCtx, Component};
use crate::hash::{ContentHash, ParamHash};
use crate::layer::{LayerIndex, LayerInfo, LayerTable};
use dashmap::DashMap;
use parking_lot::RwLock;
use smol_str::SmolStr;
use std::sync::Arc;

pub struct Library {
    name: SmolStr,
    dbu: i64,
    grid: i64,
    layers: RwLock<LayerTable>,
    cells: RwLock<Vec<Arc<Cell>>>,
    by_hash: DashMap<ContentHash, CellId>,
    by_name: DashMap<CellName, CellId>,
    by_params: DashMap<ParamHash, CellId>,
}

impl Library {
    /// Create a new library. `dbu` is database units per micron (e.g. 1000
    /// for a 1 nm grid). `grid` is the manufacturing grid in DBU; coordinates
    /// not on this grid are rejected by the (future) grid-check pass.
    pub fn new(name: impl Into<SmolStr>, dbu: i64) -> Self {
        Self {
            name: name.into(),
            dbu,
            grid: 1,
            layers: RwLock::new(LayerTable::new()),
            cells: RwLock::new(Vec::new()),
            by_hash: DashMap::new(),
            by_name: DashMap::new(),
            by_params: DashMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dbu(&self) -> i64 {
        self.dbu
    }

    pub fn grid(&self) -> i64 {
        self.grid
    }

    pub fn set_grid(&mut self, grid: i64) {
        assert!(grid >= 1, "grid must be >= 1");
        self.grid = grid;
    }

    pub fn layer(&self, info: LayerInfo) -> LayerIndex {
        self.layers.write().get_or_insert(info)
    }

    pub fn layer_info(&self, idx: LayerIndex) -> LayerInfo {
        self.layers.read().info(idx).clone()
    }

    pub fn layer_by_gds(&self, layer: u16, datatype: u16) -> Option<LayerIndex> {
        self.layers.read().by_gds(layer, datatype)
    }

    pub fn layer_by_name(&self, name: &str) -> Option<LayerIndex> {
        self.layers.read().by_name(name)
    }

    /// Freeze a builder and insert into the library, returning a `CellId`.
    /// If a cell with the same `ContentHash` already exists, the existing
    /// `CellId` is returned and the builder is discarded.
    pub fn insert(&self, builder: CellBuilder) -> CellId {
        let cell = builder.freeze(self);
        if let Some(existing) = self.by_hash.get(&cell.content_hash()) {
            return *existing;
        }
        let mut cells = self.cells.write();
        let slot = cells.len();
        let id = CellId::from_slot(slot);
        let arc = Arc::new(cell);
        let hash = arc.content_hash();
        let name = arc.name().clone();
        cells.push(arc);
        drop(cells);
        self.by_hash.insert(hash, id);
        if !name.is_empty() {
            self.by_name.entry(name).or_insert(id);
        }
        id
    }

    pub fn get(&self, id: CellId) -> Arc<Cell> {
        self.cells.read()[id.slot()].clone()
    }

    pub fn try_get(&self, id: CellId) -> Option<Arc<Cell>> {
        self.cells.read().get(id.slot()).cloned()
    }

    pub fn by_name(&self, name: &str) -> Option<CellId> {
        let key = CellName::new(name);
        self.by_name.get(&key).map(|v| *v)
    }

    pub fn by_content(&self, hash: ContentHash) -> Option<CellId> {
        self.by_hash.get(&hash).map(|v| *v)
    }

    pub fn cell_count(&self) -> usize {
        self.cells.read().len()
    }

    pub fn build<C: Component>(&self, c: &C) -> CellId {
        let pk = c.param_hash();
        if let Some(id) = self.by_params.get(&pk) {
            return *id;
        }
        let ctx = BuildCtx::new(self);
        let mut builder = c.build(&ctx);
        if builder.name().is_empty() {
            builder.set_name(c.cell_name());
        }
        let id = self.insert(builder);
        self.by_params.insert(pk, id);
        id
    }

    /// Cells with no parents in this library. Walks all cells and subtracts
    /// any referenced as instance children.
    pub fn top_cells(&self) -> Vec<CellId> {
        let cells = self.cells.read();
        let mut is_child = vec![false; cells.len()];
        for c in cells.iter() {
            for inst in c.instances() {
                is_child[inst.cell.slot()] = true;
            }
        }
        cells
            .iter()
            .enumerate()
            .filter_map(|(i, _)| {
                if !is_child[i] {
                    Some(CellId::from_slot(i))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn all_cells(&self) -> Vec<(CellId, Arc<Cell>)> {
        self.cells
            .read()
            .iter()
            .enumerate()
            .map(|(i, c)| (CellId::from_slot(i), c.clone()))
            .collect()
    }
}
