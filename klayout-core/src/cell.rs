//! Cells: frozen, immutable artifacts produced by freezing a `CellBuilder`.
//!
//! Invariants held after `CellBuilder::freeze`:
//! * `content_hash` is a deterministic blake3 of the canonical encoding.
//!   Same shapes/instances/ports/properties → same hash, byte-stable across
//!   runs and platforms.
//! * `local_bbox` is the union of shape bboxes (instance subtrees not included).
//! * `shapes` keys are sorted by `LayerIndex`; per-layer shape order is the
//!   insertion order from the builder, but hashing sorts canonically.
//! * `instances`, `ports`, `properties` are stored in insertion order;
//!   hashing sorts canonically. UI/IO consumers may sort if they care.
//!
//! Mutation after freeze is impossible — there is no public `&mut Cell`.

use crate::coord::{Bbox, Point, Trans};
use crate::hash::ContentHash;
use crate::instance::{Instance, Repetition};
use crate::layer::LayerIndex;
use crate::library::Library;
use crate::port::Port;
use crate::properties::{Properties, PropertyValue};
use crate::shape::Shape;
use smol_str::SmolStr;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellName(pub SmolStr);

impl CellName {
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for CellName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl<S: Into<SmolStr>> From<S> for CellName {
    fn from(s: S) -> Self {
        Self(s.into())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(NonZeroU32);

impl CellId {
    pub(crate) fn from_slot(slot: usize) -> Self {
        // CellId packs a 1-based slot into a NonZeroU32; the +1 cannot
        // wrap because `slot` comes from a Vec index in the library
        // arena (well below 2^32 in any plausible design), and the
        // result is therefore always ≥ 1.
        let n = u32::try_from(slot + 1)
            .expect("CellId arena slot exceeded u32::MAX (≈4B cells)");
        Self(NonZeroU32::new(n).expect("CellId: slot+1 ≥ 1 by construction"))
    }

    pub(crate) fn slot(self) -> usize {
        self.0.get() as usize - 1
    }

    pub fn raw(self) -> u32 {
        self.0.get()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ShapeBag {
    shapes: Vec<Shape>,
}

impl ShapeBag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, s: Shape) {
        self.shapes.push(s);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter()
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub(crate) name: CellName,
    pub(crate) shapes: BTreeMap<LayerIndex, ShapeBag>,
    pub(crate) instances: Vec<Instance>,
    pub(crate) ports: Vec<Port>,
    pub(crate) properties: Properties,
    pub(crate) local_bbox: Bbox,
    pub(crate) content_hash: ContentHash,
}

impl Cell {
    pub fn name(&self) -> &CellName {
        &self.name
    }

    pub fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    pub fn local_bbox(&self) -> Bbox {
        self.local_bbox
    }

    pub fn shapes_on(&self, layer: LayerIndex) -> impl Iterator<Item = &Shape> {
        self.shapes
            .get(&layer)
            .map(|b| b.iter())
            .into_iter()
            .flatten()
    }

    pub fn layers(&self) -> impl Iterator<Item = LayerIndex> + '_ {
        self.shapes.keys().copied()
    }

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    pub fn port(&self, name: &str) -> Option<&Port> {
        self.ports.iter().find(|p| p.name == name)
    }

    pub fn properties(&self) -> &Properties {
        &self.properties
    }

    /// Full bbox including transformed local bboxes of instance subtrees.
    /// Walks the hierarchy via `lib`. Repetitions are accounted for.
    pub fn full_bbox(&self, lib: &Library) -> Bbox {
        let mut b = self.local_bbox;
        for inst in &self.instances {
            let child = lib.get(inst.cell);
            let child_b = child.full_bbox(lib);
            let placed = inst.trans.apply_bbox(child_b);
            if let Some(rep) = &inst.repetition {
                let rb = rep.placement_bbox();
                if !rb.is_empty() {
                    b = b.union(&Bbox::new(
                        Point::new(placed.min.x + rb.min.x, placed.min.y + rb.min.y),
                        Point::new(placed.max.x + rb.max.x, placed.max.y + rb.max.y),
                    ));
                    continue;
                }
            }
            b = b.union(&placed);
        }
        b
    }
}

#[derive(Clone, Debug)]
pub struct CellBuilder {
    name: CellName,
    shapes: BTreeMap<LayerIndex, ShapeBag>,
    instances: Vec<Instance>,
    ports: Vec<Port>,
    properties: Properties,
}

impl CellBuilder {
    pub fn new(name: impl Into<CellName>) -> Self {
        Self {
            name: name.into(),
            shapes: BTreeMap::new(),
            instances: Vec::new(),
            ports: Vec::new(),
            properties: Properties::new(),
        }
    }

    pub fn name(&self) -> &CellName {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<CellName>) {
        self.name = name.into();
    }

    pub fn add_shape(&mut self, layer: LayerIndex, shape: impl Into<Shape>) {
        self.shapes.entry(layer).or_default().push(shape.into());
    }

    pub fn add_instance(&mut self, inst: Instance) {
        self.instances.push(inst);
    }

    pub fn instantiate(&mut self, cell: CellId, trans: Trans) {
        self.instances.push(Instance::new(cell, trans));
    }

    pub fn add_port(&mut self, port: Port) {
        self.ports.push(port);
    }

    pub fn set_property(&mut self, key: impl Into<SmolStr>, value: PropertyValue) {
        self.properties.set(key, value);
    }

    pub fn properties_mut(&mut self) -> &mut Properties {
        &mut self.properties
    }

    pub fn freeze(self, lib: &Library) -> Cell {
        let local_bbox = compute_local_bbox(&self.shapes);
        let content_hash = canonical_hash(
            &self.name,
            &self.shapes,
            &self.instances,
            &self.ports,
            &self.properties,
            lib,
        );
        Cell {
            name: self.name,
            shapes: self.shapes,
            instances: self.instances,
            ports: self.ports,
            properties: self.properties,
            local_bbox,
            content_hash,
        }
    }
}

fn compute_local_bbox(shapes: &BTreeMap<LayerIndex, ShapeBag>) -> Bbox {
    let mut b = Bbox::EMPTY;
    for bag in shapes.values() {
        for s in bag.iter() {
            b = b.union(&s.bbox());
        }
    }
    b
}

// ---------- canonical hashing ----------

fn canonical_hash(
    name: &CellName,
    shapes: &BTreeMap<LayerIndex, ShapeBag>,
    instances: &[Instance],
    ports: &[Port],
    properties: &Properties,
    lib: &Library,
) -> ContentHash {
    let mut h = blake3::Hasher::new();

    // The cell *name* is intentionally not hashed: two cells with the same
    // geometry but different names should dedup. IO can preserve the name
    // outside the hash.
    let _ = name;

    // Shapes: hash keyed by GDS (layer, datatype), NOT LayerIndex —
    // LayerIndex is a per-library handle and depends on registration order,
    // but the GDS pair is stable across libraries. This keeps content
    // hashes invariant after a write/read round-trip.
    let mut by_gds: Vec<((u16, u16), &ShapeBag)> = shapes
        .iter()
        .map(|(idx, bag)| (lib.layer_info(*idx).key(), bag))
        .collect();
    by_gds.sort_by_key(|(k, _)| *k);
    write_u32(&mut h, by_gds.len() as u32);
    for ((l, dt), bag) in by_gds {
        write_u16(&mut h, l);
        write_u16(&mut h, dt);
        let mut encoded: Vec<Vec<u8>> = bag.iter().map(canonical_shape_bytes).collect();
        encoded.sort();
        write_u32(&mut h, encoded.len() as u32);
        for e in &encoded {
            write_bytes(&mut h, e);
        }
    }

    // Instances: child content hash + canonical trans + repetition + props.
    // Sort by the canonical encoding for determinism regardless of insertion order.
    let mut inst_enc: Vec<Vec<u8>> = instances
        .iter()
        .map(|i| canonical_instance_bytes(i, lib))
        .collect();
    inst_enc.sort();
    write_u32(&mut h, inst_enc.len() as u32);
    for e in &inst_enc {
        write_bytes(&mut h, e);
    }

    // Ports: sorted by name. Layer reference encoded via the stable GDS
    // (layer, datatype) pair, not the per-library LayerIndex.
    let mut port_enc: Vec<Vec<u8>> = ports
        .iter()
        .map(|p| canonical_port_bytes(p, lib))
        .collect();
    port_enc.sort();
    write_u32(&mut h, port_enc.len() as u32);
    for e in &port_enc {
        write_bytes(&mut h, e);
    }

    // Properties: sorted by key.
    let props_sorted = properties.sorted_iter();
    write_u32(&mut h, props_sorted.len() as u32);
    for (k, v) in props_sorted {
        write_bytes(&mut h, k.as_bytes());
        write_property(&mut h, v);
    }

    ContentHash(*h.finalize().as_bytes())
}

fn canonical_shape_bytes(s: &Shape) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.push(s.discriminant());
    match s {
        Shape::Polygon(p) => {
            put_u32(&mut buf, p.hull.len() as u32);
            for pt in &p.hull {
                put_point(&mut buf, *pt);
            }
            put_u32(&mut buf, p.holes.len() as u32);
            for hole in &p.holes {
                put_u32(&mut buf, hole.len() as u32);
                for pt in hole {
                    put_point(&mut buf, *pt);
                }
            }
        }
        Shape::Path(p) => {
            put_u32(&mut buf, p.points.len() as u32);
            for pt in &p.points {
                put_point(&mut buf, *pt);
            }
            put_i64(&mut buf, p.width);
            put_i64(&mut buf, p.begin_ext);
            put_i64(&mut buf, p.end_ext);
            buf.push(match p.cap {
                crate::shape::PathCap::Flat => 0,
                crate::shape::PathCap::Round => 1,
                crate::shape::PathCap::Extended => 2,
            });
        }
        Shape::Box(r) => {
            put_point(&mut buf, r.bbox.min);
            put_point(&mut buf, r.bbox.max);
        }
        Shape::Text(t) => {
            put_u32(&mut buf, t.string.len() as u32);
            buf.extend_from_slice(t.string.as_bytes());
            put_point(&mut buf, t.anchor);
            put_i32(&mut buf, t.size);
            buf.push(t.halign as u8);
            buf.push(t.valign as u8);
        }
    }
    buf
}

fn canonical_trans_bytes(buf: &mut Vec<u8>, t: &Trans) {
    buf.push(t.rot as u8);
    buf.push(t.mirror as u8);
    put_i64(buf, t.disp.x);
    put_i64(buf, t.disp.y);
}

fn canonical_repetition_bytes(buf: &mut Vec<u8>, r: &Option<Repetition>) {
    match r {
        None => buf.push(0),
        Some(Repetition::Regular {
            row,
            col,
            n_rows,
            n_cols,
        }) => {
            buf.push(1);
            put_i64(buf, row.x);
            put_i64(buf, row.y);
            put_i64(buf, col.x);
            put_i64(buf, col.y);
            put_u32(buf, *n_rows);
            put_u32(buf, *n_cols);
        }
        Some(Repetition::Irregular { offsets }) => {
            buf.push(2);
            put_u32(buf, offsets.len() as u32);
            for o in offsets {
                put_i64(buf, o.x);
                put_i64(buf, o.y);
            }
        }
    }
}

fn canonical_instance_bytes(i: &Instance, lib: &Library) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    let child = lib.get(i.cell);
    buf.extend_from_slice(&child.content_hash.0);
    canonical_trans_bytes(&mut buf, &i.trans);
    canonical_repetition_bytes(&mut buf, &i.repetition);
    let props = i.properties.sorted_iter();
    put_u32(&mut buf, props.len() as u32);
    for (k, v) in props {
        put_u32(&mut buf, k.len() as u32);
        buf.extend_from_slice(k.as_bytes());
        canonical_property(&mut buf, v);
    }
    buf
}

fn canonical_port_bytes(p: &Port, lib: &Library) -> Vec<u8> {
    let mut buf = Vec::with_capacity(48);
    put_u32(&mut buf, p.name.len() as u32);
    buf.extend_from_slice(p.name.as_bytes());
    put_u32(&mut buf, p.kind.0);
    put_point(&mut buf, p.center);
    buf.push(p.angle as u8);
    put_i64(&mut buf, p.width);
    let info = lib.layer_info(p.layer);
    put_u16(&mut buf, info.layer);
    put_u16(&mut buf, info.datatype);
    buf
}

fn canonical_property(buf: &mut Vec<u8>, v: &PropertyValue) {
    match v {
        PropertyValue::Int(i) => {
            buf.push(0);
            put_i64(buf, *i);
        }
        PropertyValue::Float(f) => {
            buf.push(1);
            buf.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        PropertyValue::String(s) => {
            buf.push(2);
            put_u32(buf, s.len() as u32);
            buf.extend_from_slice(s.as_bytes());
        }
        PropertyValue::Bytes(b) => {
            buf.push(3);
            put_u32(buf, b.len() as u32);
            buf.extend_from_slice(b);
        }
        PropertyValue::List(items) => {
            buf.push(4);
            put_u32(buf, items.len() as u32);
            for it in items {
                canonical_property(buf, it);
            }
        }
    }
}

fn write_property(h: &mut blake3::Hasher, v: &PropertyValue) {
    let mut buf = Vec::new();
    canonical_property(&mut buf, v);
    write_bytes(h, &buf);
}

// ---- byte writers ----
fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_point(buf: &mut Vec<u8>, p: Point) {
    put_i64(buf, p.x);
    put_i64(buf, p.y);
}

fn write_u16(h: &mut blake3::Hasher, v: u16) {
    h.update(&v.to_le_bytes());
}
fn write_u32(h: &mut blake3::Hasher, v: u32) {
    h.update(&v.to_le_bytes());
}
fn write_bytes(h: &mut blake3::Hasher, b: &[u8]) {
    h.update(&(b.len() as u32).to_le_bytes());
    h.update(b);
}
