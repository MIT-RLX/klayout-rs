//! Hierarchical connectivity extraction.
//!
//! Walks an instance hierarchy starting from a top cell and collects all
//! conductive shapes (with transforms composed), then merges them across
//! one or more conductor layers. Connectivity is established across:
//!
//! 1. Same-layer touching/overlapping shapes (always).
//! 2. Cross-layer via shapes — a polygon on `via_layer` connects pieces
//!    on the two `connects` layers it overlaps.
//!
//! v1 only handles polygon/box shapes (paths and texts that aren't on the
//! label layer are ignored — paths can be polygonized in a follow-up).

use crate::netlist::{Net, NetId, Netlist};
use klayout_core::{CellId, LayerIndex, Library, Polygon, Repetition, Shape, Trans, Vec2};
use klayout_geom::{intersection, merge, Region};
use smol_str::SmolStr;
use std::collections::HashMap;

/// One conducting layer + its label layer (often the same).
#[derive(Copy, Clone, Debug)]
pub struct Conductor {
    pub layer: LayerIndex,
    pub label_layer: LayerIndex,
}

/// A via that connects two conductors. Any polygon on `via.layer` whose
/// area intersects both `a` and `b` shapes joins the corresponding nets.
#[derive(Copy, Clone, Debug)]
pub struct Via {
    pub layer: LayerIndex,
    pub a: LayerIndex,
    pub b: LayerIndex,
}

#[derive(Default)]
pub struct ExtractConfig {
    pub conductors: Vec<Conductor>,
    pub vias: Vec<Via>,
}

/// Extract a flat (after-flatten) netlist from `top` walking the hierarchy.
/// All polygons on each conductor layer are flattened into the top frame
/// (transforms composed), then merged into nets per layer, then joined
/// across layers via the `vias` rules.
pub fn extract_hierarchical(
    lib: &Library,
    top: CellId,
    config: &ExtractConfig,
) -> Netlist {
    // Step 1: flatten conductive polygons per layer + collect labels.
    let mut by_layer: HashMap<LayerIndex, Vec<Polygon>> = HashMap::new();
    let mut by_via_layer: HashMap<LayerIndex, Vec<Polygon>> = HashMap::new();
    let mut labels: Vec<(SmolStr, klayout_core::Point, LayerIndex)> = Vec::new();

    let conductor_set: std::collections::HashSet<LayerIndex> =
        config.conductors.iter().map(|c| c.layer).collect();
    let label_set: std::collections::HashSet<LayerIndex> =
        config.conductors.iter().map(|c| c.label_layer).collect();
    let via_set: std::collections::HashSet<LayerIndex> =
        config.vias.iter().map(|v| v.layer).collect();

    walk(
        lib,
        top,
        Trans::IDENTITY,
        &conductor_set,
        &label_set,
        &via_set,
        &mut by_layer,
        &mut by_via_layer,
        &mut labels,
    );

    // Step 2: per-layer merge → preliminary nets, indexed by (layer, idx).
    // We keep a flat `pieces: Vec<MergedPiece>` so that vias can union them.
    let mut pieces: Vec<MergedPiece> = Vec::new();
    let mut by_layer_to_pieces: HashMap<LayerIndex, Vec<usize>> = HashMap::new();
    for c in &config.conductors {
        let polys = by_layer.remove(&c.layer).unwrap_or_default();
        if polys.is_empty() {
            continue;
        }
        let merged = merge(&Region::from_polygons(polys));
        let mut piece_indices = Vec::with_capacity(merged.polygons().len());
        for mp in merged.polygons() {
            let idx = pieces.len();
            pieces.push(MergedPiece {
                layer: c.layer,
                polygon: mp.clone(),
                bbox: mp.bbox(),
                parent: idx,
            });
            piece_indices.push(idx);
        }
        by_layer_to_pieces.insert(c.layer, piece_indices);
    }

    // Step 3: union-find across layers via vias.
    let mut uf = UnionFind::new(pieces.len());
    for via in &config.vias {
        let via_polys = by_via_layer.get(&via.layer).cloned().unwrap_or_default();
        if via_polys.is_empty() {
            continue;
        }
        let via_region = merge(&Region::from_polygons(via_polys));
        let a_pieces = by_layer_to_pieces.get(&via.a).cloned().unwrap_or_default();
        let b_pieces = by_layer_to_pieces.get(&via.b).cloned().unwrap_or_default();
        // For each via polygon, find pieces on a and b that it overlaps.
        for via_poly in via_region.polygons() {
            let via_one = Region::from_polygons([via_poly.clone()]);
            let mut hit_a: Vec<usize> = Vec::new();
            let mut hit_b: Vec<usize> = Vec::new();
            for &i in &a_pieces {
                let pi = &pieces[i];
                if !pi.bbox.intersects(&via_poly.bbox()) {
                    continue;
                }
                let r = Region::from_polygons([pi.polygon.clone()]);
                if !intersection(&r, &via_one).is_empty() {
                    hit_a.push(i);
                }
            }
            for &j in &b_pieces {
                let pj = &pieces[j];
                if !pj.bbox.intersects(&via_poly.bbox()) {
                    continue;
                }
                let r = Region::from_polygons([pj.polygon.clone()]);
                if !intersection(&r, &via_one).is_empty() {
                    hit_b.push(j);
                }
            }
            for &i in &hit_a {
                for &j in &hit_b {
                    uf.union(i, j);
                }
            }
        }
    }

    // Step 4: group pieces by union-find root, name each net by any label
    // whose anchor falls in the net's bbox.
    let mut net_groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..pieces.len() {
        let root = uf.find(i);
        net_groups.entry(root).or_default().push(i);
    }

    let mut netlist = Netlist::new();
    let mut auto_n = 0;
    let mut net_groups_sorted: Vec<_> = net_groups.into_iter().collect();
    net_groups_sorted.sort_by_key(|(root, _)| *root);
    for (_root, members) in net_groups_sorted {
        let mut bbox = klayout_core::Bbox::EMPTY;
        let mut polys: Vec<klayout_core::Polygon> = Vec::with_capacity(members.len());
        for &m in &members {
            bbox = bbox.union(&pieces[m].bbox);
            polys.push(pieces[m].polygon.clone());
        }
        // Find label inside bbox on any conductor's label layer.
        let mut name: Option<SmolStr> = None;
        for (lbl, anchor, lyr) in &labels {
            if !label_set.contains(lyr) {
                continue;
            }
            if bbox.contains(*anchor) {
                name = Some(lbl.clone());
                break;
            }
        }
        let net_name = name.unwrap_or_else(|| {
            let n = auto_n;
            auto_n += 1;
            SmolStr::from(format!("net_{n}"))
        });
        let mut net = Net::new(NetId(0), net_name);
        net.bbox = bbox;
        net.polygons = polys;
        netlist.insert(net);
    }
    netlist
}

#[derive(Clone)]
struct MergedPiece {
    #[allow(dead_code)]
    layer: LayerIndex,
    polygon: Polygon,
    bbox: klayout_core::Bbox,
    #[allow(dead_code)]
    parent: usize,
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx != ry {
            self.parent[rx] = ry;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    lib: &Library,
    cell_id: CellId,
    trans: Trans,
    conductors: &std::collections::HashSet<LayerIndex>,
    labels_layers: &std::collections::HashSet<LayerIndex>,
    vias: &std::collections::HashSet<LayerIndex>,
    by_layer: &mut HashMap<LayerIndex, Vec<Polygon>>,
    by_via_layer: &mut HashMap<LayerIndex, Vec<Polygon>>,
    labels: &mut Vec<(SmolStr, klayout_core::Point, LayerIndex)>,
) {
    let cell = lib.get(cell_id);
    for layer in cell.layers() {
        if conductors.contains(&layer) || vias.contains(&layer) {
            for shape in cell.shapes_on(layer) {
                let p = match shape {
                    Shape::Polygon(p) => Some(p.transform(trans)),
                    Shape::Box(r) => {
                        let bb = trans.apply_bbox(r.bbox);
                        if !bb.is_empty() {
                            Some(Polygon::rect(bb))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(poly) = p {
                    if conductors.contains(&layer) {
                        by_layer.entry(layer).or_default().push(poly);
                    } else {
                        by_via_layer.entry(layer).or_default().push(poly);
                    }
                }
            }
        }
        if labels_layers.contains(&layer) {
            for shape in cell.shapes_on(layer) {
                if let Shape::Text(t) = shape {
                    let anchor = trans.apply(t.anchor);
                    labels.push((t.string.clone(), anchor, layer));
                }
            }
        }
    }
    for inst in cell.instances() {
        let placements = expand_repetition(inst);
        for placement in placements {
            let composed = trans.compose(placement);
            walk(
                lib,
                inst.cell,
                composed,
                conductors,
                labels_layers,
                vias,
                by_layer,
                by_via_layer,
                labels,
            );
        }
    }
}

fn expand_repetition(inst: &klayout_core::Instance) -> Vec<Trans> {
    match &inst.repetition {
        None => vec![inst.trans],
        Some(Repetition::Regular {
            col,
            row,
            n_cols,
            n_rows,
        }) => {
            let mut out = Vec::with_capacity((*n_cols as usize) * (*n_rows as usize));
            for j in 0..*n_rows {
                for i in 0..*n_cols {
                    let extra = Trans::translate(Vec2::new(
                        col.x * i as i64 + row.x * j as i64,
                        col.y * i as i64 + row.y * j as i64,
                    ));
                    out.push(extra.compose(inst.trans));
                }
            }
            out
        }
        Some(Repetition::Irregular { offsets }) => {
            let mut out = Vec::with_capacity(offsets.len() + 1);
            out.push(inst.trans);
            for o in offsets {
                let extra = Trans::translate(*o);
                out.push(extra.compose(inst.trans));
            }
            out
        }
    }
}
