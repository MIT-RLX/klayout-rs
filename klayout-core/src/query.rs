//! Iteration and spatial-query traits. The core ships naive linear-scan
//! impls on `Library`/`Cell`; `klayout-geom` will layer R-trees underneath
//! without changing this surface.

use crate::cell::{Cell, CellId};
use crate::coord::{Bbox, Trans};
use crate::instance::Instance;
use crate::layer::LayerIndex;
use crate::shape::Shape;

pub struct ShapeRef<'a> {
    pub cell: CellId,
    pub layer: LayerIndex,
    pub index: usize,
    pub shape: &'a Shape,
}

pub struct InstanceRef<'a> {
    pub parent: CellId,
    pub index: usize,
    pub instance: &'a Instance,
}

pub trait LayoutQuery {
    fn shapes_in<'a>(
        &'a self,
        cell: CellId,
        layer: LayerIndex,
        bbox: Bbox,
    ) -> Box<dyn Iterator<Item = ShapeRef<'a>> + 'a>;

    fn instances_in<'a>(
        &'a self,
        cell: CellId,
        bbox: Bbox,
    ) -> Box<dyn Iterator<Item = InstanceRef<'a>> + 'a>;
}

pub trait HierarchyVisitor {
    fn enter_cell(&mut self, _cell: &Cell, _trans: Trans) {}
    fn exit_cell(&mut self, _cell: &Cell, _trans: Trans) {}
    fn visit_shape(&mut self, _layer: LayerIndex, _shape: &Shape, _trans: Trans) {}
    fn visit_instance(&mut self, _instance: &Instance, _trans: Trans) {}
}

pub trait Visit {
    fn walk<V: HierarchyVisitor>(&self, root: CellId, visitor: &mut V);
}
