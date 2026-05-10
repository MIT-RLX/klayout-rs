//! Stylizer: centerline `Path` / routed segments → emitted shapes.

use crate::detailed::RoutedNet;
use crate::multilayer::RouteSegment;
use klayout_core::{Bbox, LayerIndex, Path, Point, Rect, Shape};

pub trait Stylizer {
    /// Convert a centerline path to one or more `(LayerIndex, Shape)` pairs
    /// to add to a cell. The default cap from the path is preserved.
    fn stylize(&self, layer: LayerIndex, path: Path) -> Vec<(LayerIndex, Shape)>;
}

/// Emit the centerline as a single `Path` shape on `layer`. Simplest
/// possible stylizer; matches what most digital flows want.
pub struct WirePathStylizer;

impl Stylizer for WirePathStylizer {
    fn stylize(&self, layer: LayerIndex, path: Path) -> Vec<(LayerIndex, Shape)> {
        vec![(layer, Shape::Path(path))]
    }
}

/// Per-layer wire/via geometry parameters for the routed-shape stylizer.
pub struct RoutedShapeParams {
    /// LayerIndex per stack-layer-index (the routing-stack's `layer_idx`
    /// is an index into this vec).
    pub wire_layers: Vec<LayerIndex>,
    /// LayerIndex for the cut layer between two metal layers. Length
    /// must equal `wire_layers.len() - 1`. Index `i` is the cut layer
    /// between `wire_layers[i]` and `wire_layers[i+1]`.
    pub via_cut_layers: Vec<LayerIndex>,
    /// Wire width per stack-layer-index. Used as the emitted `Path` width.
    pub wire_widths: Vec<i64>,
    /// Single-cut size (square) per via-cut layer.
    pub via_cut_size: Vec<i64>,
    /// Pitch between adjacent cuts in a via array.
    pub via_cut_pitch: Vec<i64>,
}

/// Convert a [`RoutedNet`] into shapes ready to add to a cell.
///
/// Wires emit as `Path` on the matched layer with width
/// `wire_widths[layer_idx]`. Vias emit a `Box` at the via center on
/// the cut layer between the two metals; for `cut_count > 1`, an
/// approximately-square N×N grid of cuts is emitted with `via_cut_pitch`.
pub fn routed_to_shapes(
    net: &RoutedNet,
    params: &RoutedShapeParams,
) -> Vec<(LayerIndex, Shape)> {
    let mut out: Vec<(LayerIndex, Shape)> = Vec::new();
    for seg in &net.segments {
        match seg {
            RouteSegment::Wire { layer_idx, points } => {
                if *layer_idx >= params.wire_layers.len() || points.len() < 2 {
                    continue;
                }
                let layer = params.wire_layers[*layer_idx];
                let width = params.wire_widths.get(*layer_idx).copied().unwrap_or(1);
                let p = Path::new(points.iter().copied(), width);
                out.push((layer, Shape::Path(p)));
            }
            RouteSegment::Via {
                at,
                from_layer,
                to_layer,
                cut_count,
            } => {
                let lo = (*from_layer).min(*to_layer);
                let hi = (*from_layer).max(*to_layer);
                // For each layer pair traversed (lo→lo+1, lo+1→lo+2, ...,
                // hi-1→hi), emit cuts on the cut layer in between.
                for cut_idx in lo..hi {
                    if cut_idx >= params.via_cut_layers.len() {
                        continue;
                    }
                    let cut_layer = params.via_cut_layers[cut_idx];
                    let size = params.via_cut_size.get(cut_idx).copied().unwrap_or(1);
                    let pitch = params
                        .via_cut_pitch
                        .get(cut_idx)
                        .copied()
                        .unwrap_or(size * 2);
                    let n = via_array_dimension(*cut_count);
                    let total_extent = (n as i64 - 1) * pitch;
                    let start_x = at.x - total_extent / 2;
                    let start_y = at.y - total_extent / 2;
                    for i in 0..n {
                        for j in 0..n {
                            let cx = start_x + i as i64 * pitch;
                            let cy = start_y + j as i64 * pitch;
                            let bbox = Bbox::new(
                                Point::new(cx - size / 2, cy - size / 2),
                                Point::new(cx + size / 2 + size % 2, cy + size / 2 + size % 2),
                            );
                            out.push((cut_layer, Shape::Box(Rect::new(bbox))));
                        }
                    }
                }
            }
        }
    }
    out
}

fn via_array_dimension(cut_count: u32) -> u32 {
    // Approximate as a square grid: ⌈√n⌉.
    if cut_count <= 1 {
        1
    } else {
        let s = (cut_count as f64).sqrt().ceil() as u32;
        s.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detailed::RoutedNet;
    use crate::multilayer::RouteSegment;
    use klayout_core::{LayerInfo, Library, Point};

    fn lib_with_layers() -> (Library, RoutedShapeParams) {
        let lib = Library::new("t", 1);
        let m1 = lib.layer(LayerInfo::gds(10, 0));
        let m2 = lib.layer(LayerInfo::gds(20, 0));
        let v12 = lib.layer(LayerInfo::gds(11, 0));
        let params = RoutedShapeParams {
            wire_layers: vec![m1, m2],
            via_cut_layers: vec![v12],
            wire_widths: vec![2, 4],
            via_cut_size: vec![2],
            via_cut_pitch: vec![4],
        };
        (lib, params)
    }

    #[test]
    fn wire_emits_path() {
        let (_lib, params) = lib_with_layers();
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![RouteSegment::Wire {
                layer_idx: 0,
                points: vec![Point::new(0, 0), Point::new(50, 0)],
            }],
        };
        let shapes = routed_to_shapes(&net, &params);
        assert_eq!(shapes.len(), 1);
        match &shapes[0].1 {
            Shape::Path(p) => assert_eq!(p.width, 2),
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn via_emits_single_cut_when_cut_count_is_one() {
        let (_lib, params) = lib_with_layers();
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![RouteSegment::Via {
                at: Point::new(20, 30),
                from_layer: 0,
                to_layer: 1,
                cut_count: 1,
            }],
        };
        let shapes = routed_to_shapes(&net, &params);
        assert_eq!(shapes.len(), 1);
        match &shapes[0].1 {
            Shape::Box(r) => {
                assert!(r.bbox.contains(Point::new(20, 30)));
            }
            _ => panic!("expected Box"),
        }
    }

    #[test]
    fn via_array_emits_grid_for_high_cut_count() {
        let (_lib, params) = lib_with_layers();
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![RouteSegment::Via {
                at: Point::new(0, 0),
                from_layer: 0,
                to_layer: 1,
                cut_count: 4,
            }],
        };
        let shapes = routed_to_shapes(&net, &params);
        // 4 cuts → 2×2 grid → 4 box shapes.
        assert_eq!(shapes.len(), 4);
    }

    #[test]
    fn multi_layer_via_emits_cuts_per_traversed_layer() {
        // Add a second cut layer for via from layer 1 to layer 2.
        let lib = Library::new("t", 1);
        let m1 = lib.layer(LayerInfo::gds(10, 0));
        let m2 = lib.layer(LayerInfo::gds(20, 0));
        let m3 = lib.layer(LayerInfo::gds(30, 0));
        let v12 = lib.layer(LayerInfo::gds(11, 0));
        let v23 = lib.layer(LayerInfo::gds(21, 0));
        let params = RoutedShapeParams {
            wire_layers: vec![m1, m2, m3],
            via_cut_layers: vec![v12, v23],
            wire_widths: vec![2, 2, 2],
            via_cut_size: vec![2, 2],
            via_cut_pitch: vec![4, 4],
        };
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![RouteSegment::Via {
                at: Point::new(0, 0),
                from_layer: 0,
                to_layer: 2,
                cut_count: 1,
            }],
        };
        let shapes = routed_to_shapes(&net, &params);
        // One cut on each via layer → 2 boxes total.
        assert_eq!(shapes.len(), 2);
    }
}
