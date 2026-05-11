//! DEF reader.
//!
//! Two entry points:
//! * [`read_def`] — convenience that inserts the top cell into a library
//!   and returns its `CellId`. Drops design metadata.
//! * [`read_def_full`] — returns a [`DefDesign`] with the parsed `Row`,
//!   `Track`, `DesignPin`, `RouteNet`, `Blockage`, etc. tables in
//!   addition to the placed top cell. Optional [`crate::LefLibrary`] metadata
//!   supplies LEF default routing width for `ROUTED` segments with no explicit width.
//!
//! Coverage:
//! * Header (VERSION, DIVIDERCHAR, BUSBITCHARS, NAMESCASESENSITIVE).
//! * `DESIGN <name> ;`
//! * `UNITS DISTANCE MICRONS <n> ;`
//! * `DIEAREA ( x y ) ( x y ) [ ( x y ) ... ] ;`
//! * `ROW`, `TRACKS`, `GCELLGRID` (single-line statements, full).
//! * `VIAS <count> ; ... END VIAS` — design-level via definitions.
//! * `COMPONENTS <count> ; - <inst> <macro> + PLACED|FIXED|UNPLACED ...`
//! * `PINS <count> ; - <name> + NET <net> + DIRECTION ... + USE ... +
//!   LAYER <l> ( x y ) ( x y ) + PLACED ( x y ) <orient> ; ...`
//! * `NETS <count> ; - <name> ( comp pin ) ... + ROUTED <layer> ( x y )
//!   ( x y ) [ NEW <layer> ... ] ;`
//! * `SPECIALNETS <count> ; - <name> + ROUTED <layer> <width> + RECT
//!   <layer> ( x y ) ( x y ) ;`
//! * `BLOCKAGES <count> ; - LAYER <l> + RECT ( x y ) ( x y ) ; ...`
//! * `REGIONS <count> ; ... END REGIONS`
//! * `GROUPS <count> ; ... END GROUPS`

use crate::error::{LefError, Result};
use crate::tokenizer::{Token, Tokenizer};
use crate::types::*;
use klayout_core::{
    Bbox, CellBuilder, CellId, Instance, LayerInfo, Library, Path, Point, Polygon, Rect, Rot4,
    Trans, Vec2,
};
use smol_str::SmolStr;
use std::collections::HashMap;

pub fn read_def(src: &[u8], lib: &Library) -> Result<CellId> {
    let design = read_def_full(src, lib, None)?;
    design.top.ok_or(LefError::MissingField("DESIGN"))
}

/// `lef_tech`: optional [`LefLibrary`] from [`crate::read_lef_full`], used for
/// LEF per-layer default routing width when a NETS `ROUTED` segment omits width.
pub fn read_def_full(
    src: &[u8],
    lib: &Library,
    lef_tech: Option<&LefLibrary>,
) -> Result<DefDesign> {
    let mut p = DefParser::new(src, lib, lef_tech)?;
    p.parse_top()?;
    Ok(p.into_design())
}

struct DefParser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    line: usize,
    lib: &'a Library,
    top_builder: Option<CellBuilder>,
    top_id: Option<CellId>,
    design: DefDesign,
    /// LEF `WIDTH` per routing layer (DBU), for default NETS wire width.
    routing_width_dbu: HashMap<SmolStr, i64>,
}

impl<'a> DefParser<'a> {
    fn new(src: &[u8], lib: &'a Library, lef_tech: Option<&LefLibrary>) -> Result<Self> {
        let routing_width_dbu = lef_tech
            .map(|l| l.routing_width_dbu(lib.dbu()))
            .unwrap_or_default();
        let mut tk = Tokenizer::new(src);
        let mut tokens = Vec::new();
        let mut last_line = 1;
        while let Some(t) = tk.next_token()? {
            tokens.push(t);
            last_line = tk.line;
        }
        let design = DefDesign {
            units_dbu_per_micron: lib.dbu(),
            ..Default::default()
        };
        Ok(Self {
            tokens,
            pos: 0,
            line: last_line,
            lib,
            top_builder: None,
            top_id: None,
            design,
            routing_width_dbu,
        })
    }

    fn into_design(mut self) -> DefDesign {
        self.design.top = self.top_id;
        self.design
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect_word(&mut self) -> Result<String> {
        match self.advance() {
            Some(Token::Word(s)) => Ok(s),
            Some(other) => Err(LefError::Expected {
                line: self.line,
                expected: "word",
                got: format!("{other:?}"),
            }),
            None => Err(LefError::UnexpectedEof),
        }
    }

    fn expect_string_or_word(&mut self) -> Result<String> {
        match self.advance() {
            Some(Token::Word(s)) | Some(Token::String(s)) => Ok(s),
            Some(other) => Err(LefError::Expected {
                line: self.line,
                expected: "word or string",
                got: format!("{other:?}"),
            }),
            None => Err(LefError::UnexpectedEof),
        }
    }

    fn expect_number(&mut self) -> Result<f64> {
        match self.advance() {
            Some(Token::Number(n)) => Ok(n),
            Some(other) => Err(LefError::Expected {
                line: self.line,
                expected: "number",
                got: format!("{other:?}"),
            }),
            None => Err(LefError::UnexpectedEof),
        }
    }

    fn expect(&mut self, want: Token) -> Result<()> {
        match self.advance() {
            Some(t) if t == want => Ok(()),
            Some(other) => Err(LefError::Expected {
                line: self.line,
                expected: "token",
                got: format!("{other:?} (wanted {want:?})"),
            }),
            None => Err(LefError::UnexpectedEof),
        }
    }

    fn skip_until_semicolon(&mut self) {
        while let Some(t) = self.advance() {
            if let Token::Semicolon = t {
                return;
            }
        }
    }

    fn parse_top(&mut self) -> Result<()> {
        let mut design_name: Option<String> = None;
        let mut die_area: Option<Bbox> = None;

        while let Some(t) = self.peek().cloned() {
            match t {
                Token::Word(w) => match w.to_ascii_uppercase().as_str() {
                    "VERSION" => {
                        self.advance();
                        if let Some(Token::Number(n)) = self.advance() {
                            self.design.version = Some(n);
                        }
                        self.skip_until_semicolon();
                    }
                    "DIVIDERCHAR" => {
                        self.advance();
                        if let Some(Token::String(s)) = self.advance() {
                            self.design.divider_char = Some(SmolStr::from(s));
                        }
                        self.skip_until_semicolon();
                    }
                    "BUSBITCHARS" => {
                        self.advance();
                        if let Some(Token::String(s)) = self.advance() {
                            self.design.bus_bit_chars = Some(SmolStr::from(s));
                        }
                        self.skip_until_semicolon();
                    }
                    "NAMESCASESENSITIVE" | "PROPERTYDEFINITIONS" | "TECHNOLOGY"
                    | "HISTORY" => {
                        self.skip_until_semicolon();
                    }
                    "DESIGN" => {
                        self.advance();
                        let name = self.expect_word()?;
                        self.skip_until_semicolon();
                        self.design.design_name = SmolStr::from(name.clone());
                        design_name = Some(name);
                    }
                    "UNITS" => {
                        self.advance();
                        let _distance = self.expect_word()?;
                        let _microns = self.expect_word()?;
                        let n = self.expect_number()?;
                        self.skip_until_semicolon();
                        self.design.units_dbu_per_micron = n.round().max(1.0) as i64;
                    }
                    "DIEAREA" => {
                        self.advance();
                        die_area = Some(self.parse_diearea()?);
                        self.design.diearea = die_area;
                    }
                    "ROW" => {
                        self.advance();
                        let row = self.parse_row()?;
                        self.design.rows.push(row);
                    }
                    "TRACKS" => {
                        self.advance();
                        let tr = self.parse_tracks()?;
                        self.design.tracks.push(tr);
                    }
                    "GCELLGRID" => {
                        self.advance();
                        let gc = self.parse_gcell_grid()?;
                        self.design.gcell_grids.push(gc);
                    }
                    "VIAS" => {
                        self.advance();
                        self.parse_design_vias()?;
                    }
                    "COMPONENTS" => {
                        self.advance();
                        self.ensure_top_builder(design_name.as_deref(), die_area);
                        self.parse_components()?;
                    }
                    "PINS" => {
                        self.advance();
                        self.ensure_top_builder(design_name.as_deref(), die_area);
                        self.parse_pins()?;
                    }
                    "NETS" => {
                        self.advance();
                        self.ensure_top_builder(design_name.as_deref(), die_area);
                        self.parse_nets(false)?;
                    }
                    "SPECIALNETS" => {
                        self.advance();
                        self.ensure_top_builder(design_name.as_deref(), die_area);
                        self.parse_nets(true)?;
                    }
                    "BLOCKAGES" => {
                        self.advance();
                        self.parse_blockages()?;
                    }
                    "REGIONS" => {
                        self.advance();
                        self.parse_regions()?;
                    }
                    "GROUPS" => {
                        self.advance();
                        self.parse_groups()?;
                    }
                    "STYLES" | "SCANCHAINS" | "FILLS" | "COMPONENTMASKSHIFT"
                    | "NONDEFAULTRULES" => {
                        // Bracketed, skip.
                        self.advance();
                        self.skip_to_end_section(&w.to_ascii_uppercase())?;
                    }
                    "END" => {
                        self.advance();
                        if let Some(Token::Word(_)) = self.peek() {
                            self.advance();
                        }
                        break;
                    }
                    _ => self.skip_until_semicolon(),
                },
                _ => {
                    self.advance();
                }
            }
        }

        if let Some(builder) = self.top_builder.take() {
            self.top_id = Some(self.lib.insert(builder));
        } else if let Some(name) = design_name {
            let mut cb = CellBuilder::new(name);
            if let Some(da) = die_area {
                let l = self.lib.layer(LayerInfo::named("DIEAREA", 0, 0));
                cb.add_shape(l, Rect::new(da));
            }
            self.top_id = Some(self.lib.insert(cb));
        }
        Ok(())
    }

    fn ensure_top_builder(&mut self, design: Option<&str>, die_area: Option<Bbox>) {
        if self.top_builder.is_some() {
            return;
        }
        let name = design.map(|s| s.to_string()).unwrap_or_else(|| "top".to_string());
        let mut cb = CellBuilder::new(name);
        if let Some(da) = die_area {
            let l = self.lib.layer(LayerInfo::named("DIEAREA", 0, 0));
            cb.add_shape(l, Rect::new(da));
        }
        self.top_builder = Some(cb);
    }

    fn parse_diearea(&mut self) -> Result<Bbox> {
        let mut min_x = i64::MAX;
        let mut min_y = i64::MAX;
        let mut max_x = i64::MIN;
        let mut max_y = i64::MIN;
        loop {
            match self.advance() {
                Some(Token::OpenParen) => {
                    let x = self.expect_number()? as i64;
                    let y = self.expect_number()? as i64;
                    self.expect(Token::CloseParen)?;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
                Some(Token::Semicolon) => break,
                Some(_) => {}
                None => return Err(LefError::UnexpectedEof),
            }
        }
        Ok(Bbox::new(Point::new(min_x, min_y), Point::new(max_x, max_y)))
    }

    fn parse_row(&mut self) -> Result<Row> {
        // ROW <name> <site> <x> <y> <orient> [DO <nx> BY <ny> STEP <sx> <sy>] ;
        let name = self.expect_word()?;
        let site = self.expect_word()?;
        let x = self.expect_number()? as i64;
        let y = self.expect_number()? as i64;
        let orient = self.expect_word()?;
        let mut row = Row {
            name: SmolStr::from(name),
            site: SmolStr::from(site),
            origin: (x, y),
            orient: SmolStr::from(orient),
            num_x: 1,
            num_y: 1,
            step_x: 0,
            step_y: 0,
        };
        loop {
            match self.advance() {
                Some(Token::Semicolon) => break,
                Some(Token::Word(w)) => {
                    let upper = w.to_ascii_uppercase();
                    if upper == "DO" {
                        row.num_x = self.expect_number()? as u32;
                        let by = self.expect_word()?;
                        if by.eq_ignore_ascii_case("BY") {
                            row.num_y = self.expect_number()? as u32;
                        }
                    } else if upper == "STEP" {
                        row.step_x = self.expect_number()? as i64;
                        row.step_y = self.expect_number()? as i64;
                    }
                }
                _ => {}
            }
        }
        Ok(row)
    }

    fn parse_tracks(&mut self) -> Result<Track> {
        // TRACKS X|Y <start> DO <num> STEP <step> [LAYER <l> ...] ;
        let dir_word = self.expect_word()?;
        let direction = if dir_word.eq_ignore_ascii_case("X") {
            TrackDirection::X
        } else {
            TrackDirection::Y
        };
        let start = self.expect_number()? as i64;
        let mut num_tracks = 1u32;
        let mut step = 0i64;
        let mut layers: Vec<SmolStr> = Vec::new();
        loop {
            match self.advance() {
                Some(Token::Semicolon) => break,
                Some(Token::Word(w)) => {
                    let upper = w.to_ascii_uppercase();
                    if upper == "DO" {
                        num_tracks = self.expect_number()? as u32;
                    } else if upper == "STEP" {
                        step = self.expect_number()? as i64;
                    } else if upper == "LAYER" {
                        // LAYER <name> [<name> ...] (ends at MASK or ;)
                        loop {
                            match self.peek().cloned() {
                                Some(Token::Word(s))
                                    if !s.eq_ignore_ascii_case("MASK")
                                        && !s.eq_ignore_ascii_case("DO")
                                        && !s.eq_ignore_ascii_case("STEP") =>
                                {
                                    self.advance();
                                    layers.push(SmolStr::from(s));
                                }
                                _ => break,
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Track {
            direction,
            start,
            num_tracks,
            step,
            layers,
        })
    }

    fn parse_gcell_grid(&mut self) -> Result<GcellGrid> {
        // GCELLGRID X|Y <start> DO <num> STEP <step> ;
        let dir_word = self.expect_word()?;
        let direction = if dir_word.eq_ignore_ascii_case("X") {
            TrackDirection::X
        } else {
            TrackDirection::Y
        };
        let start = self.expect_number()? as i64;
        let mut num = 1u32;
        let mut step = 0i64;
        loop {
            match self.advance() {
                Some(Token::Semicolon) => break,
                Some(Token::Word(w)) => {
                    let upper = w.to_ascii_uppercase();
                    if upper == "DO" {
                        num = self.expect_number()? as u32;
                    } else if upper == "STEP" {
                        step = self.expect_number()? as i64;
                    }
                }
                _ => {}
            }
        }
        Ok(GcellGrid {
            direction,
            start,
            num,
            step,
        })
    }

    fn parse_design_vias(&mut self) -> Result<()> {
        // VIAS <count> ; - <name> [+ VIARULE <r>] [+ RECT <l> ( x y ) ( x y )] ; ... END VIAS
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let name = self.expect_word()?;
                    let mut via = DefVia {
                        name: SmolStr::from(name),
                        ..Default::default()
                    };
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                let upper = kw.to_ascii_uppercase();
                                match upper.as_str() {
                                    "VIARULE" => {
                                        via.via_rule =
                                            Some(SmolStr::from(self.expect_word()?));
                                    }
                                    "RECT" => {
                                        let layer = self.expect_word()?;
                                        self.expect(Token::OpenParen)?;
                                        let x0 = self.expect_number()? as i64;
                                        let y0 = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        self.expect(Token::OpenParen)?;
                                        let x1 = self.expect_number()? as i64;
                                        let y1 = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        via.shapes.push(ViaShape {
                                            layer: SmolStr::from(layer),
                                            bbox: Bbox::new(
                                                Point::new(x0, y0),
                                                Point::new(x1, y1),
                                            ),
                                        });
                                    }
                                    _ => {
                                        // Skip to next + or ;
                                        loop {
                                            match self.peek() {
                                                Some(Token::Plus) | Some(Token::Semicolon)
                                                | None => break,
                                                _ => {
                                                    self.advance();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }
                    self.design.vias.push(via);
                }
                _ => {}
            }
        }
    }

    fn parse_components(&mut self) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let inst_name = self.expect_word()?;
                    let macro_name = self.expect_word()?;
                    let mut placed_at: Option<(i64, i64)> = None;
                    let mut orient: Option<String> = None;

                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                match kw.to_ascii_uppercase().as_str() {
                                    "PLACED" | "FIXED" | "COVER" => {
                                        self.expect(Token::OpenParen)?;
                                        let x = self.expect_number()? as i64;
                                        let y = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        let or = self.expect_word()?;
                                        placed_at = Some((x, y));
                                        orient = Some(or);
                                    }
                                    "UNPLACED" => {}
                                    _ => loop {
                                        match self.peek() {
                                            Some(Token::Plus) | Some(Token::Semicolon)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    },
                                }
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }

                    let cell_id = self
                        .lib
                        .by_name(&macro_name)
                        .ok_or_else(|| LefError::UnknownMacro(macro_name.clone()))?;
                    // Master width/height needed for orientation
                    // normalisation (FN/FS/FE/FW shift the origin by
                    // master dimensions to match OpenDB).
                    let master = self.lib.get(cell_id);
                    let mb = master.local_bbox();
                    let mw = if mb.is_empty() { 0 } else { mb.max.x - mb.min.x };
                    let mh = if mb.is_empty() { 0 } else { mb.max.y - mb.min.y };
                    let trans = match (placed_at, orient.as_deref()) {
                        (Some((x, y)), Some(o)) => orient_to_trans(o, x, y, mw, mh),
                        (Some((x, y)), None) => Trans::translate(Vec2::new(x, y)),
                        _ => Trans::IDENTITY,
                    };
                    if let Some(cb) = self.top_builder.as_mut() {
                        let mut inst = Instance::new(cell_id, trans);
                        inst.properties.set(
                            "def_name",
                            klayout_core::PropertyValue::String(SmolStr::from(inst_name)),
                        );
                        cb.add_instance(inst);
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_pins(&mut self) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let pin_name = self.expect_word()?;
                    let mut pin = DesignPin {
                        name: SmolStr::from(pin_name.clone()),
                        net: SmolStr::default(),
                        ..Default::default()
                    };
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                match kw.to_ascii_uppercase().as_str() {
                                    "NET" => {
                                        pin.net = SmolStr::from(self.expect_string_or_word()?);
                                    }
                                    "DIRECTION" => {
                                        let d = self.expect_word()?;
                                        pin.direction = match d.to_ascii_uppercase().as_str() {
                                            "INPUT" => Some(PinDirection::Input),
                                            "OUTPUT" => Some(PinDirection::Output),
                                            "INOUT" => Some(PinDirection::Inout),
                                            "FEEDTHRU" => Some(PinDirection::Feedthru),
                                            _ => None,
                                        };
                                    }
                                    "USE" => {
                                        let u = self.expect_word()?;
                                        pin.use_ = match u.to_ascii_uppercase().as_str() {
                                            "SIGNAL" => Some(PinUse::Signal),
                                            "POWER" => Some(PinUse::Power),
                                            "GROUND" => Some(PinUse::Ground),
                                            "CLOCK" => Some(PinUse::Clock),
                                            "ANALOG" => Some(PinUse::Analog),
                                            "RESET" => Some(PinUse::Reset),
                                            "TIEOFF" => Some(PinUse::Tieoff),
                                            "SCAN" => Some(PinUse::Scan),
                                            _ => None,
                                        };
                                    }
                                    "LAYER" => {
                                        let lname = self.expect_word()?;
                                        // ( x0 y0 ) ( x1 y1 )
                                        self.expect(Token::OpenParen)?;
                                        let x0 = self.expect_number()? as i64;
                                        let y0 = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        self.expect(Token::OpenParen)?;
                                        let x1 = self.expect_number()? as i64;
                                        let y1 = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        pin.layer = Some(SmolStr::from(lname));
                                        pin.layer_bbox = Some(Bbox::new(
                                            Point::new(x0, y0),
                                            Point::new(x1, y1),
                                        ));
                                    }
                                    "PLACED" | "FIXED" | "COVER" => {
                                        if kw.eq_ignore_ascii_case("FIXED") {
                                            pin.fixed = true;
                                        }
                                        self.expect(Token::OpenParen)?;
                                        let x = self.expect_number()? as i64;
                                        let y = self.expect_number()? as i64;
                                        self.expect(Token::CloseParen)?;
                                        let or = self.expect_word()?;
                                        pin.placed = Some((x, y));
                                        pin.orient = Some(SmolStr::from(or));
                                    }
                                    _ => loop {
                                        match self.peek() {
                                            Some(Token::Plus) | Some(Token::Semicolon)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    },
                                }
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }
                    // Add pin geometry to the top cell as a Box on the
                    // pin's layer (if specified).
                    if let (Some(layer_name), Some(bbox), Some(pos)) =
                        (&pin.layer, pin.layer_bbox, pin.placed)
                    {
                        // Find or create the layer.
                        let layer_idx = self
                            .lib
                            .layer_by_name(layer_name)
                            .unwrap_or_else(|| self.lib.layer(LayerInfo::named(
                                layer_name.as_str(),
                                hash_layer_name(layer_name),
                                0,
                            )));
                        let translated = Bbox::new(
                            Point::new(bbox.min.x + pos.0, bbox.min.y + pos.1),
                            Point::new(bbox.max.x + pos.0, bbox.max.y + pos.1),
                        );
                        if let Some(cb) = self.top_builder.as_mut() {
                            cb.add_shape(layer_idx, Rect::new(translated));
                        }
                    }
                    // Bus expansion: if pin name is `A[3:0]`, expand to scalar.
                    if let Some(bus) = klayout_core::parse_bus(
                        pin.name.as_str(),
                        klayout_core::BusBitChars::default(),
                    ) {
                        if bus.width() > 1 {
                            let chars = klayout_core::BusBitChars::default();
                            for scalar_name in bus.expand(chars) {
                                let mut p = pin.clone();
                                p.name = SmolStr::from(scalar_name);
                                p.bus = Some(bus.clone());
                                self.design.pins.push(p);
                            }
                        } else {
                            let mut p = pin;
                            p.bus = Some(bus);
                            self.design.pins.push(p);
                        }
                    } else {
                        self.design.pins.push(pin);
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_nets(&mut self, special: bool) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let net_name = self.expect_string_or_word()?;
                    let mut net = RouteNet {
                        name: SmolStr::from(net_name),
                        special,
                        ..Default::default()
                    };
                    // Optional connection points before any +.
                    while matches!(self.peek(), Some(Token::OpenParen)) {
                        self.advance();
                        let inst = self.expect_string_or_word()?;
                        let pin = self.expect_string_or_word()?;
                        self.expect(Token::CloseParen)?;
                        net.connects.push(NetConnect {
                            instance: if inst == "PIN" {
                                None
                            } else {
                                Some(SmolStr::from(inst))
                            },
                            pin: SmolStr::from(pin),
                        });
                    }
                    // Then attribute and routing chunks.
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                let upper = kw.to_ascii_uppercase();
                                match upper.as_str() {
                                    "USE" => {
                                        let u = self.expect_word()?;
                                        net.use_ = match u.to_ascii_uppercase().as_str() {
                                            "SIGNAL" => Some(PinUse::Signal),
                                            "POWER" => Some(PinUse::Power),
                                            "GROUND" => Some(PinUse::Ground),
                                            "CLOCK" => Some(PinUse::Clock),
                                            "ANALOG" => Some(PinUse::Analog),
                                            _ => None,
                                        };
                                    }
                                    "ROUTED" | "FIXED" | "COVER" | "NEW" => {
                                        let segs = self.parse_route_segments(special)?;
                                        net.segments.extend(segs);
                                    }
                                    "SOURCE" | "WEIGHT" | "PROPERTY" | "ORIGINAL"
                                    | "FREQUENCY" | "PATTERN" | "ESTCAP" | "NONDEFAULTRULE"
                                    | "SHIELD" | "SHIELDNET" | "SUBNET"
                                    | "VOLTAGE" => {
                                        loop {
                                            match self.peek() {
                                                Some(Token::Plus) | Some(Token::Semicolon)
                                                | None => break,
                                                _ => {
                                                    self.advance();
                                                }
                                            }
                                        }
                                    }
                                    _ => loop {
                                        match self.peek() {
                                            Some(Token::Plus) | Some(Token::Semicolon)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    },
                                }
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }

                    // Materialize wire segments into top cell as Path/Rect.
                    self.materialize_segments(&net);

                    if special {
                        self.design.special_nets.push(net);
                    } else {
                        self.design.nets.push(net);
                    }
                }
                _ => {}
            }
        }
    }

    /// Parse one chunk of routed segments (after the keyword like ROUTED/NEW).
    /// Format: `<layer> [<width>] ( x y ) ( x y | * y | x * ) ... [VIA <vianame>] ;` /
    /// runs end at + or ;.
    fn parse_route_segments(&mut self, special: bool) -> Result<Vec<RouteSegment>> {
        let mut out: Vec<RouteSegment> = Vec::new();
        let layer = SmolStr::from(self.expect_word()?);
        // Optional taper / nondefault width. For SPECIALNETS the width is
        // explicit immediately after the layer name.
        let mut width: Option<i64> = None;
        if special {
            if let Some(Token::Number(n)) = self.peek().cloned() {
                self.advance();
                width = Some(n as i64);
            }
        } else if let Some(Token::Word(w)) = self.peek().cloned() {
            if w.eq_ignore_ascii_case("TAPER") || w.eq_ignore_ascii_case("STYLE") {
                self.advance();
                let _ = self.expect_word()?;
            }
        }
        let mut points: Vec<Point> = Vec::new();
        let mut last: Option<Point> = None;
        loop {
            match self.peek().cloned() {
                Some(Token::OpenParen) => {
                    self.advance();
                    // x can be "*" (= last x) or a number; same for y.
                    let x = match self.advance() {
                        Some(Token::Number(n)) => n as i64,
                        Some(Token::Word(w)) if w == "*" => last.map(|p| p.x).unwrap_or(0),
                        Some(other) => {
                            return Err(LefError::Expected {
                                line: self.line,
                                expected: "number or *",
                                got: format!("{other:?}"),
                            })
                        }
                        None => return Err(LefError::UnexpectedEof),
                    };
                    let y = match self.advance() {
                        Some(Token::Number(n)) => n as i64,
                        Some(Token::Word(w)) if w == "*" => last.map(|p| p.y).unwrap_or(0),
                        Some(other) => {
                            return Err(LefError::Expected {
                                line: self.line,
                                expected: "number or *",
                                got: format!("{other:?}"),
                            })
                        }
                        None => return Err(LefError::UnexpectedEof),
                    };
                    // Optional extension or via name token before ).
                    while let Some(t) = self.peek().cloned() {
                        match t {
                            Token::CloseParen => break,
                            _ => {
                                self.advance();
                            }
                        }
                    }
                    self.expect(Token::CloseParen)?;
                    let pt = Point::new(x, y);
                    points.push(pt);
                    last = Some(pt);
                }
                Some(Token::Word(w)) => {
                    let upper = w.to_ascii_uppercase();
                    if upper == "RECT" {
                        // SPECIALNETS RECT layer ( x0 y0 ) ( x1 y1 )
                        self.advance();
                        let layer_name = self.expect_word()?;
                        self.expect(Token::OpenParen)?;
                        let x0 = self.expect_number()? as i64;
                        let y0 = self.expect_number()? as i64;
                        self.expect(Token::CloseParen)?;
                        self.expect(Token::OpenParen)?;
                        let x1 = self.expect_number()? as i64;
                        let y1 = self.expect_number()? as i64;
                        self.expect(Token::CloseParen)?;
                        out.push(RouteSegment::Rect {
                            layer: SmolStr::from(layer_name),
                            bbox: Bbox::new(Point::new(x0, y0), Point::new(x1, y1)),
                        });
                    } else if upper == "VIRTUAL" || upper == "MASK" {
                        self.advance();
                        // Skip arg
                        if let Some(Token::Word(_)) = self.peek() {
                            self.advance();
                        }
                    } else if upper == "VIA" {
                        // `... ( x y ) VIA VIA12` (explicit keyword).
                        self.advance();
                        let via_name = self.expect_string_or_word()?;
                        if points.len() >= 2 {
                            out.push(RouteSegment::Wire {
                                layer: layer.clone(),
                                points: std::mem::take(&mut points),
                                width,
                            });
                        }
                        if let Some(at) = last {
                            out.push(RouteSegment::Via {
                                via_name: SmolStr::from(via_name),
                                at,
                            });
                        }
                    } else if w.starts_with('+') || w == "+" {
                        // Continuation marker — terminates this segment.
                        break;
                    } else {
                        // Bare via cell name after the last vertex.
                        let via_name = w.clone();
                        self.advance();
                        if points.len() >= 2 {
                            out.push(RouteSegment::Wire {
                                layer: layer.clone(),
                                points: std::mem::take(&mut points),
                                width,
                            });
                        }
                        if let Some(at) = last {
                            out.push(RouteSegment::Via {
                                via_name: SmolStr::from(via_name),
                                at,
                            });
                        }
                    }
                }
                Some(Token::Plus) | Some(Token::Semicolon) | None => break,
                _ => {
                    self.advance();
                }
            }
        }
        if points.len() >= 2 {
            out.push(RouteSegment::Wire {
                layer,
                points,
                width,
            });
        }
        Ok(out)
    }

    fn materialize_segments(&mut self, net: &RouteNet) {
        let Some(cb) = self.top_builder.as_mut() else {
            return;
        };
        for seg in &net.segments {
            match seg {
                RouteSegment::Wire {
                    layer,
                    points,
                    width,
                } => {
                    if points.len() < 2 {
                        continue;
                    }
                    let layer_idx = self
                        .lib
                        .layer_by_name(layer)
                        .unwrap_or_else(|| self.lib.layer(LayerInfo::named(
                            layer.as_str(),
                            hash_layer_name(layer),
                            0,
                        )));
                    let w_explicit = *width;
                    let w = w_explicit.unwrap_or_else(|| {
                        self.routing_width_dbu
                            .get(layer)
                            .copied()
                            .unwrap_or(0)
                    });
                    let mut path = Path::new(points.iter().copied(), w);
                    if w > 0 && w_explicit.is_none() {
                        let h = w / 2;
                        path.begin_ext = h;
                        path.end_ext = h;
                    }
                    cb.add_shape(layer_idx, path);
                }
                RouteSegment::Rect { layer, bbox } => {
                    let layer_idx = self
                        .lib
                        .layer_by_name(layer)
                        .unwrap_or_else(|| self.lib.layer(LayerInfo::named(
                            layer.as_str(),
                            hash_layer_name(layer),
                            0,
                        )));
                    cb.add_shape(layer_idx, Rect::new(*bbox));
                }
                RouteSegment::Via { .. } => {}
            }
        }
    }

    fn parse_blockages(&mut self) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    // - LAYER <name> <bbox-list> ; or - PLACEMENT <bbox-list> ;
                    let kind_kw = self.expect_word()?;
                    let mut blockage = Blockage {
                        kind: BlockageKind::Routing,
                        layer: None,
                        bboxes: Vec::new(),
                        component: None,
                    };
                    match kind_kw.to_ascii_uppercase().as_str() {
                        "LAYER" => {
                            blockage.layer = Some(SmolStr::from(self.expect_word()?));
                        }
                        "PLACEMENT" => {
                            blockage.kind = BlockageKind::Placement;
                        }
                        _ => {}
                    }
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let attr = self.expect_word()?;
                                let upper = attr.to_ascii_uppercase();
                                if upper == "COMPONENT" {
                                    blockage.component =
                                        Some(SmolStr::from(self.expect_word()?));
                                } else {
                                    // Skip.
                                    loop {
                                        match self.peek() {
                                            Some(Token::Plus)
                                            | Some(Token::Semicolon)
                                            | Some(Token::OpenParen)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Token::OpenParen) => {
                                let x0 = self.expect_number()? as i64;
                                let y0 = self.expect_number()? as i64;
                                self.expect(Token::CloseParen)?;
                                self.expect(Token::OpenParen)?;
                                let x1 = self.expect_number()? as i64;
                                let y1 = self.expect_number()? as i64;
                                self.expect(Token::CloseParen)?;
                                blockage.bboxes.push(Bbox::new(
                                    Point::new(x0, y0),
                                    Point::new(x1, y1),
                                ));
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }
                    self.design.blockages.push(blockage);
                }
                _ => {}
            }
        }
    }

    fn parse_regions(&mut self) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let name = self.expect_word()?;
                    let mut region = Region {
                        name: SmolStr::from(name),
                        ..Default::default()
                    };
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::OpenParen) => {
                                let x0 = self.expect_number()? as i64;
                                let y0 = self.expect_number()? as i64;
                                self.expect(Token::CloseParen)?;
                                self.expect(Token::OpenParen)?;
                                let x1 = self.expect_number()? as i64;
                                let y1 = self.expect_number()? as i64;
                                self.expect(Token::CloseParen)?;
                                region.bboxes.push(Bbox::new(
                                    Point::new(x0, y0),
                                    Point::new(x1, y1),
                                ));
                            }
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                let upper = kw.to_ascii_uppercase();
                                if upper == "TYPE" {
                                    let t = self.expect_word()?;
                                    region.kind = Some(SmolStr::from(t));
                                } else {
                                    loop {
                                        match self.peek() {
                                            Some(Token::Plus)
                                            | Some(Token::Semicolon)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    }
                                }
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }
                    self.design.regions.push(region);
                }
                _ => {}
            }
        }
    }

    fn parse_groups(&mut self) -> Result<()> {
        let _count = self.expect_number()?;
        self.skip_until_semicolon();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Token::Minus => {
                    let name = self.expect_word()?;
                    let mut group = Group {
                        name: SmolStr::from(name),
                        ..Default::default()
                    };
                    loop {
                        match self.advance() {
                            Some(Token::Semicolon) => break,
                            Some(Token::Plus) => {
                                let kw = self.expect_word()?;
                                let upper = kw.to_ascii_uppercase();
                                if upper == "REGION" {
                                    group.region =
                                        Some(SmolStr::from(self.expect_word()?));
                                } else {
                                    loop {
                                        match self.peek() {
                                            Some(Token::Plus)
                                            | Some(Token::Semicolon)
                                            | None => break,
                                            _ => {
                                                self.advance();
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Token::Word(s)) => {
                                group.members.push(SmolStr::from(s));
                            }
                            Some(_) => {}
                            None => return Err(LefError::UnexpectedEof),
                        }
                    }
                    self.design.groups.push(group);
                }
                _ => {}
            }
        }
    }

    fn skip_to_end_section(&mut self, section: &str) -> Result<()> {
        loop {
            match self.advance() {
                Some(Token::Word(w)) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(n)) = self.peek().cloned() {
                        if n.eq_ignore_ascii_case(section) {
                            self.advance();
                            return Ok(());
                        }
                    }
                }
                Some(_) => {}
                None => return Err(LefError::UnexpectedEof),
            }
        }
    }
}

/// Convert a DEF orientation + placed (x, y) into our internal `Trans`.
/// Master width/height (in DBU) are needed to apply OpenDB's
/// post-rotation/mirror origin normalisation: for orientations whose
/// natural transformation moves the master's bounding-box off the
/// placed point, we shift the origin so the master's lower-left visible
/// corner lands at (placed_x, placed_y) plus the appropriate offset.
///
/// Empirically (`openroad/orfs:latest` 26Q2):
/// ```text
///   N   (R0, !mirror) -> (+0,    +0)
///   S   (R180,!mirror)-> (+w,    +h)
///   E   (R270,!mirror)-> (+0,    +w)
///   W   (R90, !mirror)-> (+h,    +0)
///   FN  (R0,  mirror) -> (+w,    +0)   (= MY)
///   FS  (R180,mirror) -> (+0,    +h)   (= MX)
///   FE  (R270,mirror) -> (+h,    +w)   (= MYR90)
///   FW  (R90, mirror) -> (+0,    +0)   (= MXR90)
/// ```
fn orient_to_trans(orient: &str, x: i64, y: i64, w: i64, h: i64) -> Trans {
    let (rot, mirror, dx, dy) = match orient.to_ascii_uppercase().as_str() {
        "N"  => (Rot4::R0,   false, 0, 0),
        "S"  => (Rot4::R180, false, w, h),
        "E"  => (Rot4::R270, false, 0, w),
        "W"  => (Rot4::R90,  false, h, 0),
        "FN" => (Rot4::R0,   true,  w, 0),
        "FS" => (Rot4::R180, true,  0, h),
        "FE" => (Rot4::R270, true,  h, w),
        "FW" => (Rot4::R90,  true,  0, 0),
        _    => (Rot4::R0,   false, 0, 0),
    };
    Trans::new(rot, mirror, Vec2::new(x + dx, y + dy))
}

/// Hash a layer name to a stable u16 GDS layer number for round-tripping
/// when no explicit GDS mapping exists.
fn hash_layer_name(name: &str) -> u16 {
    let mut h: u32 = 0x811c9dc5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    // Avoid 0; cap to keep generated layer numbers in a sane range.
    let n = (h & 0x7FFF) as u16;
    if n == 0 {
        1
    } else {
        n
    }
}

#[allow(dead_code)]
fn _polygon_marker(_: Polygon) {}
