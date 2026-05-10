//! LEF reader.
//!
//! Two entry points:
//! * [`read_lef`] — convenience that returns just a `Library` populated
//!   with macro cells. Drops layer/via/site metadata.
//! * [`read_lef_full`] — returns a [`LefLibrary`] with the library plus
//!   parsed `LayerSpec`, `ViaSpec`, `SiteSpec`, and `MacroSpec`.
//!
//! Coverage:
//! * Header records (`VERSION`, `BUSBITCHARS`, `DIVIDERCHAR`,
//!   `MANUFACTURINGGRID`, `NAMESCASESENSITIVE`, `USEMINSPACING`,
//!   `CLEARANCEMEASURE`).
//! * `UNITS DATABASE MICRONS <n> ;`
//! * `LAYER <name> ... END <name>` — TYPE, DIRECTION, WIDTH, PITCH,
//!   OFFSET, SPACING (basic), AREA (min area), MINSTEP, MINIMUMCUT,
//!   THICKNESS, RESISTANCE/CAPACITANCE per-square, EDGE-CAPACITANCE,
//!   ANTENNAGATEAREA / ANTENNADIFFAREA factors, MAXVIASTACK.
//!   Unrecognized property statements are skipped to `;`.
//! * `VIA <name> [DEFAULT] ... END <name>` — RESISTANCE, LAYER ... RECT
//!   (multiple cuts), VIARULE / CUTSIZE / LAYERS for generated vias.
//! * `SITE <name> ... END <name>` — CLASS, SIZE, SYMMETRY, ROWPATTERN.
//! * `MACRO <name> ... END <name>` — CLASS, ORIGIN, SIZE, SYMMETRY,
//!   FOREIGN, SITE, PIN, OBS.
//! * `PIN <name> ... END <name>` — DIRECTION, USE, SHAPE, antenna props,
//!   one or more `PORT` blocks.
//! * `PORT` blocks — `LAYER <name> ;`, `RECT x0 y0 x1 y1 ;`,
//!   `POLYGON x0 y0 x1 y1 ... ;`, multiple statements allowed.
//! * `OBS` blocks — same geometry vocabulary as `PORT`.
//!
//! Anything not recognized is skipped tolerantly.

use crate::error::{LefError, Result};
use crate::tokenizer::{Token, Tokenizer};
use crate::types::*;
use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Polygon, Rect};
use smol_str::SmolStr;

pub fn read_lef(src: &[u8]) -> Result<Library> {
    Ok(read_lef_full(src)?.library)
}

pub fn read_lef_full(src: &[u8]) -> Result<LefLibrary> {
    let mut p = Parser::new(src)?;
    p.parse_top()?;
    Ok(p.into_lef_library())
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    line: usize,
    lib: Library,
    next_gds_layer: u16,
    layer_index: std::collections::HashMap<String, u16>,

    version: Option<f64>,
    bus_bit_chars: Option<SmolStr>,
    divider_char: Option<SmolStr>,
    manufacturing_grid: Option<f64>,
    layers: Vec<LayerSpec>,
    vias: Vec<ViaSpec>,
    sites: Vec<SiteSpec>,
    macros: Vec<MacroSpec>,
}

impl Parser {
    fn new(src: &[u8]) -> Result<Self> {
        let mut tk = Tokenizer::new(src);
        let mut tokens = Vec::new();
        let mut last_line = 1;
        while let Some(t) = tk.next_token()? {
            tokens.push(t);
            last_line = tk.line;
        }
        Ok(Self {
            tokens,
            pos: 0,
            line: last_line,
            lib: Library::new("lef", 1000),
            next_gds_layer: 1,
            layer_index: std::collections::HashMap::new(),
            version: None,
            bus_bit_chars: None,
            divider_char: None,
            manufacturing_grid: None,
            layers: Vec::new(),
            vias: Vec::new(),
            sites: Vec::new(),
            macros: Vec::new(),
        })
    }

    fn into_lef_library(self) -> LefLibrary {
        LefLibrary {
            library: self.lib,
            version: self.version,
            bus_bit_chars: self.bus_bit_chars,
            divider_char: self.divider_char,
            manufacturing_grid: self.manufacturing_grid,
            layers: self.layers,
            vias: self.vias,
            sites: self.sites,
            macros: self.macros,
        }
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

    fn expect_semicolon(&mut self) -> Result<()> {
        match self.advance() {
            Some(Token::Semicolon) => Ok(()),
            Some(other) => Err(LefError::Expected {
                line: self.line,
                expected: ";",
                got: format!("{other:?}"),
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

    /// Skip everything until `END [name]` at the current nesting level.
    fn skip_block_to_end(&mut self, expected_name: Option<&str>) {
        let mut depth = 0;
        while let Some(t) = self.advance() {
            if let Token::Word(w) = &t {
                if w.eq_ignore_ascii_case("END") {
                    if depth == 0 {
                        if let Some(name) = expected_name {
                            if let Some(Token::Word(n)) = self.peek() {
                                if n == name {
                                    self.advance();
                                }
                            }
                        }
                        return;
                    } else {
                        depth -= 1;
                    }
                } else if w.eq_ignore_ascii_case("MACRO")
                    || w.eq_ignore_ascii_case("PIN")
                    || w.eq_ignore_ascii_case("PORT")
                    || w.eq_ignore_ascii_case("OBS")
                    || w.eq_ignore_ascii_case("SITE")
                    || w.eq_ignore_ascii_case("LAYER")
                {
                    depth += 1;
                }
            }
        }
    }

    fn parse_top(&mut self) -> Result<()> {
        while let Some(t) = self.peek().cloned() {
            match t {
                Token::Word(w) => match w.to_ascii_uppercase().as_str() {
                    "VERSION" => {
                        self.advance();
                        let n = self.expect_number()?;
                        self.expect_semicolon()?;
                        self.version = Some(n);
                    }
                    "BUSBITCHARS" => {
                        self.advance();
                        if let Some(Token::String(s)) = self.advance() {
                            self.bus_bit_chars = Some(SmolStr::from(s));
                        }
                        self.skip_until_semicolon();
                    }
                    "DIVIDERCHAR" => {
                        self.advance();
                        if let Some(Token::String(s)) = self.advance() {
                            self.divider_char = Some(SmolStr::from(s));
                        }
                        self.skip_until_semicolon();
                    }
                    "MANUFACTURINGGRID" => {
                        self.advance();
                        let n = self.expect_number()?;
                        self.expect_semicolon()?;
                        self.manufacturing_grid = Some(n);
                    }
                    "NAMESCASESENSITIVE" | "NOWIREEXTENSIONATPIN"
                    | "PROPERTYDEFINITIONS" | "USEMINSPACING" | "CLEARANCEMEASURE"
                    | "FIXEDMASK" => {
                        self.skip_until_semicolon();
                    }
                    "UNITS" => {
                        self.advance();
                        self.parse_units()?;
                    }
                    "SITE" => {
                        self.advance();
                        self.parse_site()?;
                    }
                    "LAYER" => {
                        self.advance();
                        self.parse_layer()?;
                    }
                    "VIA" => {
                        self.advance();
                        self.parse_via()?;
                    }
                    "VIARULE" | "NONDEFAULTRULE" | "PROPERTY" | "DENSITY" => {
                        // Bracketed sections we don't decode in detail.
                        self.advance();
                        let _name = match self.peek().cloned() {
                            Some(Token::Word(n)) => {
                                self.advance();
                                Some(n)
                            }
                            _ => None,
                        };
                        self.skip_block_to_end(None);
                    }
                    "MACRO" => {
                        self.advance();
                        self.parse_macro()?;
                    }
                    "EXTENSIONS" | "BEGINEXT" => {
                        self.skip_until_semicolon();
                    }
                    "END" => {
                        self.advance();
                        if let Some(Token::Word(_)) = self.peek() {
                            self.advance();
                        }
                        return Ok(());
                    }
                    _ => {
                        self.skip_until_semicolon();
                    }
                },
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    fn parse_units(&mut self) -> Result<()> {
        loop {
            match self.advance() {
                Some(Token::Word(w)) if w.eq_ignore_ascii_case("END") => {
                    if let Some(Token::Word(_)) = self.peek() {
                        self.advance();
                    }
                    return Ok(());
                }
                Some(Token::Word(w)) if w.eq_ignore_ascii_case("DATABASE") => {
                    let _kind = self.expect_word()?;
                    let n = self.expect_number()?;
                    let dbu = n.round().max(1.0) as i64;
                    let mut new_lib = Library::new("lef", dbu);
                    std::mem::swap(&mut self.lib, &mut new_lib);
                    self.expect_semicolon()?;
                }
                Some(Token::Semicolon) => {}
                Some(_) => self.skip_until_semicolon(),
                None => return Err(LefError::UnexpectedEof),
            }
        }
    }

    fn register_layer(&mut self, name: &str) -> u16 {
        if let Some(g) = self.layer_index.get(name) {
            return *g;
        }
        let g = self.next_gds_layer;
        self.next_gds_layer += 1;
        self.layer_index.insert(name.to_string(), g);
        self.lib.layer(LayerInfo::named(name, g, 0));
        g
    }

    fn parse_layer(&mut self) -> Result<()> {
        let name = self.expect_word()?;
        let gds = self.register_layer(&name);
        let mut spec = LayerSpec {
            name: SmolStr::from(&name),
            gds_layer: Some(gds),
            gds_datatype: Some(0),
            ..Default::default()
        };
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            if let Token::Word(w) = t {
                let upper = w.to_ascii_uppercase();
                match upper.as_str() {
                    "END" => {
                        if let Some(Token::Word(n)) = self.peek().cloned() {
                            if n == name {
                                self.advance();
                            }
                        }
                        self.layers.push(spec);
                        return Ok(());
                    }
                    "TYPE" => {
                        let kind = self.expect_word()?;
                        spec.layer_type = Some(match kind.to_ascii_uppercase().as_str() {
                            "ROUTING" => LayerType::Routing,
                            "CUT" => LayerType::Cut,
                            "MASTERSLICE" => LayerType::Masterslice,
                            "OVERLAP" => LayerType::Overlap,
                            "IMPLANT" => LayerType::Implant,
                            _ => LayerType::Other,
                        });
                        self.expect_semicolon()?;
                    }
                    "DIRECTION" => {
                        let dir = self.expect_word()?;
                        spec.direction = match dir.to_ascii_uppercase().as_str() {
                            "HORIZONTAL" => Some(RoutingDirection::Horizontal),
                            "VERTICAL" => Some(RoutingDirection::Vertical),
                            "DIAG45" => Some(RoutingDirection::Diag45),
                            "DIAG135" => Some(RoutingDirection::Diag135),
                            _ => None,
                        };
                        self.expect_semicolon()?;
                    }
                    "WIDTH" => {
                        spec.width = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "PITCH" => {
                        let n = self.expect_number()?;
                        // Optional second number for separate XY pitches.
                        if let Some(Token::Number(_)) = self.peek() {
                            self.advance();
                        }
                        spec.pitch = Some(n);
                        self.expect_semicolon()?;
                    }
                    "OFFSET" => {
                        let n = self.expect_number()?;
                        if let Some(Token::Number(_)) = self.peek() {
                            self.advance();
                        }
                        spec.offset = Some(n);
                        self.expect_semicolon()?;
                    }
                    "SPACING" => {
                        let n = self.expect_number()?;
                        let mut rule = SpacingRule {
                            min_spacing: n,
                            same_net: false,
                            range_min: None,
                            range_max: None,
                        };
                        // Optional modifiers up to ;
                        while let Some(Token::Word(w)) = self.peek().cloned() {
                            let upper = w.to_ascii_uppercase();
                            if upper == "SAMENET" {
                                self.advance();
                                rule.same_net = true;
                            } else if upper == "RANGE" {
                                self.advance();
                                rule.range_min = Some(self.expect_number()?);
                                rule.range_max = Some(self.expect_number()?);
                            } else {
                                break;
                            }
                        }
                        self.skip_until_semicolon();
                        spec.spacing.push(rule);
                    }
                    "AREA" => {
                        spec.min_area = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "MINSTEP" => {
                        spec.min_step = Some(self.expect_number()?);
                        self.skip_until_semicolon();
                    }
                    "MINIMUMCUT" => {
                        spec.minimum_cut = Some(self.expect_number()?);
                        self.skip_until_semicolon();
                    }
                    "EDGECAPACITANCE" => {
                        spec.edge_capacitance = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "RESISTANCE" => {
                        // RESISTANCE RPERSQ <n> ; or RESISTANCE <n> ;
                        if let Some(Token::Word(w)) = self.peek().cloned() {
                            if w.eq_ignore_ascii_case("RPERSQ") {
                                self.advance();
                                spec.resistance_per_sq = Some(self.expect_number()?);
                                self.expect_semicolon()?;
                                continue;
                            }
                        }
                        spec.resistance_per_sq = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "CAPACITANCE" => {
                        if let Some(Token::Word(w)) = self.peek().cloned() {
                            if w.eq_ignore_ascii_case("CPERSQDIST") {
                                self.advance();
                                spec.capacitance_per_sq = Some(self.expect_number()?);
                                self.expect_semicolon()?;
                                continue;
                            }
                        }
                        spec.capacitance_per_sq = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "THICKNESS" => {
                        spec.thickness = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "MAXVIASTACK" => {
                        let n = self.expect_number()?;
                        spec.max_via_stack = Some(n.round() as u32);
                        self.skip_until_semicolon();
                    }
                    "ANTENNAAREARATIO" | "ANTENNADIFFAREARATIO" | "ANTENNAAREAFACTOR" => {
                        if let Ok(n) = self.expect_number() {
                            spec.antenna_metal_area_factor = Some(n);
                        }
                        self.skip_until_semicolon();
                    }
                    "ANTENNAGATEAREARATIO" => {
                        if let Ok(n) = self.expect_number() {
                            spec.antenna_diff_area_factor = Some(n);
                        }
                        self.skip_until_semicolon();
                    }
                    _ => {
                        self.skip_until_semicolon();
                    }
                }
            }
        }
    }

    fn parse_via(&mut self) -> Result<()> {
        let name = self.expect_word()?;
        let mut spec = ViaSpec {
            name: SmolStr::from(&name),
            ..Default::default()
        };
        // Optional DEFAULT keyword
        if let Some(Token::Word(w)) = self.peek().cloned() {
            if w.eq_ignore_ascii_case("DEFAULT") {
                self.advance();
                spec.default = true;
            }
        }
        let mut current_layer: Option<SmolStr> = None;
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            if let Token::Word(w) = t {
                let upper = w.to_ascii_uppercase();
                match upper.as_str() {
                    "END" => {
                        if let Some(Token::Word(n)) = self.peek().cloned() {
                            if n == name {
                                self.advance();
                            }
                        }
                        self.vias.push(spec);
                        return Ok(());
                    }
                    "RESISTANCE" => {
                        spec.resistance = Some(self.expect_number()?);
                        self.expect_semicolon()?;
                    }
                    "VIARULE" => {
                        spec.via_rule = Some(SmolStr::from(self.expect_word()?));
                        self.expect_semicolon()?;
                    }
                    "CUTSIZE" => {
                        let w = self.expect_number()?;
                        let h = self.expect_number()?;
                        spec.cut_size = Some((w, h));
                        self.expect_semicolon()?;
                    }
                    "LAYERS" => {
                        let a = self.expect_word()?;
                        let b = self.expect_word()?;
                        let c = self.expect_word()?;
                        self.expect_semicolon()?;
                        spec.layers = Some((SmolStr::from(a), SmolStr::from(b), SmolStr::from(c)));
                    }
                    "LAYER" => {
                        let lname = self.expect_word()?;
                        current_layer = Some(SmolStr::from(lname.clone()));
                        self.register_layer(&lname);
                        self.expect_semicolon()?;
                    }
                    "RECT" => {
                        let x0 = self.expect_number()?;
                        let y0 = self.expect_number()?;
                        let x1 = self.expect_number()?;
                        let y1 = self.expect_number()?;
                        self.expect_semicolon()?;
                        if let Some(layer) = &current_layer {
                            let dbu = self.lib.dbu() as f64;
                            let bbox = Bbox::new(
                                Point::new(
                                    (x0 * dbu).round() as i64,
                                    (y0 * dbu).round() as i64,
                                ),
                                Point::new(
                                    (x1 * dbu).round() as i64,
                                    (y1 * dbu).round() as i64,
                                ),
                            );
                            spec.shapes.push(ViaShape {
                                layer: layer.clone(),
                                bbox,
                            });
                        }
                    }
                    "ROWCOL" | "ORIGIN" | "OFFSET" | "PATTERN" | "ENCLOSURE" => {
                        self.skip_until_semicolon();
                    }
                    _ => self.skip_until_semicolon(),
                }
            }
        }
    }

    fn parse_site(&mut self) -> Result<()> {
        let name = self.expect_word()?;
        let mut spec = SiteSpec {
            name: SmolStr::from(&name),
            ..Default::default()
        };
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            if let Token::Word(w) = t {
                let upper = w.to_ascii_uppercase();
                match upper.as_str() {
                    "END" => {
                        if let Some(Token::Word(n)) = self.peek().cloned() {
                            if n == name {
                                self.advance();
                            }
                        }
                        self.sites.push(spec);
                        return Ok(());
                    }
                    "CLASS" => {
                        spec.class = Some(SmolStr::from(self.expect_word()?));
                        self.expect_semicolon()?;
                    }
                    "SIZE" => {
                        let w = self.expect_number()?;
                        let by = self.expect_word()?;
                        if by.eq_ignore_ascii_case("BY") {
                            let h = self.expect_number()?;
                            self.expect_semicolon()?;
                            spec.size = Some((w, h));
                        } else {
                            self.skip_until_semicolon();
                        }
                    }
                    "SYMMETRY" => {
                        // Up to ; collect tokens
                        loop {
                            match self.advance() {
                                Some(Token::Semicolon) => break,
                                Some(Token::Word(s)) => spec.symmetry.push(SmolStr::from(s)),
                                _ => {}
                            }
                        }
                    }
                    "ROWPATTERN" => {
                        // ROWPATTERN <site> <orient> [<site> <orient>]* ;
                        loop {
                            match self.advance() {
                                Some(Token::Semicolon) => break,
                                Some(Token::Word(s)) => {
                                    let orient = match self.advance() {
                                        Some(Token::Word(o)) => o,
                                        _ => break,
                                    };
                                    spec.row_pattern
                                        .push((SmolStr::from(s), SmolStr::from(orient)));
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => self.skip_until_semicolon(),
                }
            }
        }
    }

    fn parse_macro(&mut self) -> Result<()> {
        let macro_name = self.expect_word()?;
        let mut cb = CellBuilder::new(macro_name.clone());
        let mut spec = MacroSpec {
            name: SmolStr::from(&macro_name),
            ..Default::default()
        };
        let mut size: Option<(f64, f64)> = None;

        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) => {
                    let upper = w.to_ascii_uppercase();
                    match upper.as_str() {
                        "END" => {
                            if let Some(Token::Word(_)) = self.peek() {
                                self.advance();
                            }
                            if let Some((w, h)) = size {
                                let dbu = self.lib.dbu() as f64;
                                let bbox = Bbox::new(
                                    Point::new(0, 0),
                                    Point::new(
                                        (w * dbu).round() as i64,
                                        (h * dbu).round() as i64,
                                    ),
                                );
                                let outline = self.lib.layer(LayerInfo::named("OUTLINE", 0, 0));
                                cb.add_shape(outline, Rect::new(bbox));
                                spec.size = Some((w, h));
                            }
                            self.lib.insert(cb);
                            self.macros.push(spec);
                            return Ok(());
                        }
                        "CLASS" => {
                            // Class can be CORE, IO, BLOCK, COVER, RING, PAD, with subtypes.
                            let mut class_str = String::new();
                            loop {
                                match self.advance() {
                                    Some(Token::Semicolon) => break,
                                    Some(Token::Word(s)) => {
                                        if !class_str.is_empty() {
                                            class_str.push(' ');
                                        }
                                        class_str.push_str(&s);
                                    }
                                    _ => {}
                                }
                            }
                            spec.class = Some(SmolStr::from(class_str));
                        }
                        "SYMMETRY" => loop {
                            match self.advance() {
                                Some(Token::Semicolon) => break,
                                Some(Token::Word(s)) => spec.symmetry.push(SmolStr::from(s)),
                                _ => {}
                            }
                        },
                        "ORIGIN" => {
                            let x = self.expect_number()?;
                            let y = self.expect_number()?;
                            self.expect_semicolon()?;
                            spec.origin = Some((x, y));
                        }
                        "SOURCE" => {
                            self.skip_until_semicolon();
                        }
                        "SITE" => {
                            spec.site = Some(SmolStr::from(self.expect_word()?));
                            self.skip_until_semicolon();
                        }
                        "FOREIGN" => {
                            let name = self.expect_word()?;
                            // Optional offset
                            let x = match self.peek() {
                                Some(Token::Number(_)) => self.expect_number()?,
                                _ => 0.0,
                            };
                            let y = match self.peek() {
                                Some(Token::Number(_)) => self.expect_number()?,
                                _ => 0.0,
                            };
                            self.skip_until_semicolon();
                            spec.foreign = Some((SmolStr::from(name), x, y));
                        }
                        "EEQ" | "LEQ" | "POWER" | "DENSITY" => {
                            self.skip_until_semicolon();
                        }
                        "SIZE" => {
                            let w = self.expect_number()?;
                            let by = self.expect_word()?;
                            if !by.eq_ignore_ascii_case("BY") {
                                return Err(LefError::Expected {
                                    line: self.line,
                                    expected: "BY",
                                    got: by,
                                });
                            }
                            let h = self.expect_number()?;
                            self.expect_semicolon()?;
                            size = Some((w, h));
                        }
                        "PIN" => {
                            let pin = self.parse_pin(&mut cb)?;
                            // Bus expansion: if pin name matches A[hi:lo],
                            // generate one PinSpec per scalar bit; tag each
                            // with the original bus.
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
                                        spec.pins.push(p);
                                    }
                                } else {
                                    let mut p = pin;
                                    p.bus = Some(bus);
                                    spec.pins.push(p);
                                }
                            } else {
                                spec.pins.push(pin);
                            }
                        }
                        "OBS" => {
                            let obs_shapes = self.parse_obs(&mut cb)?;
                            spec.obs.extend(obs_shapes);
                        }
                        _ => {
                            self.skip_until_semicolon();
                        }
                    }
                }
                Token::Semicolon => {}
                _ => {}
            }
        }
    }

    fn parse_pin(&mut self, cb: &mut CellBuilder) -> Result<PinSpec> {
        let pin_name = self.expect_word()?;
        let mut spec = PinSpec {
            name: SmolStr::from(&pin_name),
            ..Default::default()
        };
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            match t {
                Token::Word(w) => {
                    let upper = w.to_ascii_uppercase();
                    match upper.as_str() {
                        "END" => {
                            if let Some(Token::Word(n)) = self.peek().cloned() {
                                if n == pin_name {
                                    self.advance();
                                }
                            }
                            return Ok(spec);
                        }
                        "DIRECTION" => {
                            let d = self.expect_word()?;
                            spec.direction = match d.to_ascii_uppercase().as_str() {
                                "INPUT" => Some(PinDirection::Input),
                                "OUTPUT" => Some(PinDirection::Output),
                                "INOUT" => Some(PinDirection::Inout),
                                "FEEDTHRU" => Some(PinDirection::Feedthru),
                                _ => None,
                            };
                            self.skip_until_semicolon();
                        }
                        "USE" => {
                            let u = self.expect_word()?;
                            spec.use_ = match u.to_ascii_uppercase().as_str() {
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
                            self.expect_semicolon()?;
                        }
                        "SHAPE" => {
                            let s = self.expect_word()?;
                            spec.shape = match s.to_ascii_uppercase().as_str() {
                                "ABUTMENT" => Some(PinShape::Abutment),
                                "RING" => Some(PinShape::Ring),
                                "FEEDTHRU" => Some(PinShape::Feedthru),
                                _ => None,
                            };
                            self.expect_semicolon()?;
                        }
                        "PORT" => {
                            let shapes = self.parse_port_geom(cb)?;
                            for s in shapes {
                                spec.geometry.shapes.push(s);
                            }
                        }
                        "ANTENNAGATEAREA" => {
                            spec.antenna_gate_area = Some(self.expect_number()?);
                            self.skip_until_semicolon();
                        }
                        "ANTENNADIFFAREA" => {
                            spec.antenna_diff_area = Some(self.expect_number()?);
                            self.skip_until_semicolon();
                        }
                        "TAPERRULE" | "ANTENNAPARTIALMETALAREA"
                        | "ANTENNAPARTIALMETALSIDEAREA" | "ANTENNAPARTIALCUTAREA"
                        | "ANTENNAPARTIALDIFFAREA" | "ANTENNAMETALAREA"
                        | "ANTENNAMETALLENGTH" | "ANTENNAMODEL" | "ANTENNAMAXAREACAR"
                        | "ANTENNAMAXSIDEAREACAR" | "ANTENNAMAXCUTCAR" | "PROPERTY" => {
                            self.skip_until_semicolon();
                        }
                        _ => {
                            self.skip_until_semicolon();
                        }
                    }
                }
                Token::Semicolon => {}
                _ => {}
            }
        }
    }

    /// Parse the body of a PORT block (multiple LAYER ... RECT/POLYGON ...)
    /// and add geometry to `cb`. Returns the per-shape (layer, shape) list
    /// for the parent PinSpec.
    fn parse_port_geom(
        &mut self,
        cb: &mut CellBuilder,
    ) -> Result<Vec<(SmolStr, PortShape)>> {
        let mut current_layer_name: Option<SmolStr> = None;
        let mut current_layer_idx: Option<u16> = None;
        let mut shapes_out: Vec<(SmolStr, PortShape)> = Vec::new();
        loop {
            let t = self.advance().ok_or(LefError::UnexpectedEof)?;
            if let Token::Word(w) = t {
                let upper = w.to_ascii_uppercase();
                match upper.as_str() {
                    "END" => return Ok(shapes_out),
                    "LAYER" => {
                        let name = self.expect_word()?;
                        current_layer_idx = Some(self.register_layer(&name));
                        current_layer_name = Some(SmolStr::from(name));
                        // Skip optional EXCEPTPGNET / SPACING / DESIGNRULEWIDTH
                        self.skip_until_semicolon();
                    }
                    "RECT" => {
                        let x0 = self.expect_number()?;
                        let y0 = self.expect_number()?;
                        let x1 = self.expect_number()?;
                        let y1 = self.expect_number()?;
                        self.expect_semicolon()?;
                        if let (Some(layer_idx), Some(layer_name)) =
                            (current_layer_idx, current_layer_name.clone())
                        {
                            let dbu = self.lib.dbu() as f64;
                            let li = self.lib.layer(LayerInfo::named(
                                layer_name.as_str(),
                                layer_idx,
                                0,
                            ));
                            let bbox = Bbox::new(
                                Point::new((x0 * dbu).round() as i64, (y0 * dbu).round() as i64),
                                Point::new((x1 * dbu).round() as i64, (y1 * dbu).round() as i64),
                            );
                            cb.add_shape(li, Rect::new(bbox));
                            shapes_out.push((layer_name, PortShape::Rect(bbox)));
                        }
                    }
                    "POLYGON" => {
                        // POLYGON x0 y0 x1 y1 x2 y2 ... ;
                        let mut pts: Vec<Point> = Vec::new();
                        let dbu = self.lib.dbu() as f64;
                        while matches!(self.peek(), Some(Token::Number(_))) {
                            let x = self.expect_number()?;
                            let y = self.expect_number()?;
                            pts.push(Point::new(
                                (x * dbu).round() as i64,
                                (y * dbu).round() as i64,
                            ));
                        }
                        self.expect_semicolon()?;
                        if let (Some(layer_idx), Some(layer_name)) =
                            (current_layer_idx, current_layer_name.clone())
                        {
                            let li = self.lib.layer(LayerInfo::named(
                                layer_name.as_str(),
                                layer_idx,
                                0,
                            ));
                            cb.add_shape(li, Polygon::from_hull(pts.clone()));
                            shapes_out.push((layer_name, PortShape::Polygon(pts)));
                        }
                    }
                    "VIA" | "PATH" | "CLASS" => {
                        self.skip_until_semicolon();
                    }
                    _ => self.skip_until_semicolon(),
                }
            }
        }
    }

    fn parse_obs(
        &mut self,
        cb: &mut CellBuilder,
    ) -> Result<Vec<(SmolStr, PortShape)>> {
        // OBS has the same vocabulary as PORT.
        self.parse_port_geom(cb)
    }
}
