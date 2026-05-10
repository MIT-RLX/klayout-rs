//! OASIS modal variables (per spec §10).
//!
//! OASIS records may omit fields whose values match the most recently
//! used value of a corresponding "modal variable." The reader maintains
//! these and substitutes when an info-byte bit indicates the field is
//! absent. Coverage now includes the modal variables for RECTANGLE,
//! POLYGON, PATH, TEXT, TRAPEZOID, CTRAPEZOID, CIRCLE, and PLACEMENT.

#[derive(Default, Clone)]
pub(crate) struct Modal {
    pub layer: u64,
    pub datatype: u64,
    pub textlayer: u64,
    pub texttype: u64,
    pub geometry_x: i64,
    pub geometry_y: i64,
    pub geometry_w: u64,
    pub geometry_h: u64,
    pub text_x: i64,
    pub text_y: i64,
    pub text_string: String,
    pub placement_x: i64,
    pub placement_y: i64,
    pub last_polygon_points: Vec<(i64, i64)>,
    pub last_path_points: Vec<(i64, i64)>,
    pub path_halfwidth: u64,
    pub path_start_extension: i64,
    pub path_end_extension: i64,
    pub last_cellname: Option<String>,
    pub circle_radius: u64,
    pub trap_w: u64,
    pub trap_h: u64,
    pub trap_a: i64,
    pub trap_b: i64,
    /// Last decoded repetition as a vector of (dx, dy) offsets relative
    /// to the base position. `None` until the first repetition is seen
    /// in this modal scope.
    pub last_repetition: Option<Vec<(i64, i64)>>,
}
