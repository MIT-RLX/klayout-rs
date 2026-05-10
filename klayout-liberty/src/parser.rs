//! Recursive-descent parser for Liberty (.lib).

use crate::tokenizer::{Token, Tokenizer};
use crate::types::*;
use crate::{LibertyError, Result};
use smol_str::SmolStr;
use std::path::Path;

pub fn parse_liberty_path(path: impl AsRef<Path>) -> Result<Library> {
    let bytes = std::fs::read(path.as_ref())?;
    parse_liberty(&bytes)
}

pub fn parse_liberty(bytes: &[u8]) -> Result<Library> {
    let mut p = Parser::new(bytes);
    p.parse_library()
}

struct Parser<'a> {
    tk: Tokenizer<'a>,
    peeked: Option<Token>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self {
            tk: Tokenizer::new(src),
            peeked: None,
        }
    }

    fn next(&mut self) -> Result<Option<Token>> {
        if let Some(t) = self.peeked.take() {
            return Ok(Some(t));
        }
        self.tk.next_token()
    }

    fn peek(&mut self) -> Result<Option<&Token>> {
        if self.peeked.is_none() {
            self.peeked = self.tk.next_token()?;
        }
        Ok(self.peeked.as_ref())
    }

    fn expect(&mut self, expected: Token) -> Result<()> {
        match self.next()? {
            Some(t) if t == expected => Ok(()),
            other => Err(LibertyError::UnexpectedToken {
                pos: self.tk.pos(),
                msg: format!("expected {expected:?}, got {other:?}"),
            }),
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        match self.next()? {
            Some(Token::Ident(s)) => Ok(s),
            Some(Token::String(s)) => Ok(s),
            other => Err(LibertyError::UnexpectedToken {
                pos: self.tk.pos(),
                msg: format!("expected identifier, got {other:?}"),
            }),
        }
    }

    fn parse_library(&mut self) -> Result<Library> {
        // Optional preamble — find `library` keyword.
        let head = match self.next()? {
            Some(Token::Ident(s)) => s,
            other => {
                return Err(LibertyError::UnexpectedToken {
                    pos: self.tk.pos(),
                    msg: format!("expected `library`, got {other:?}"),
                })
            }
        };
        if head != "library" {
            return Err(LibertyError::UnexpectedToken {
                pos: self.tk.pos(),
                msg: format!("expected `library`, got `{head}`"),
            });
        }
        self.expect(Token::LParen)?;
        let name = self.expect_ident()?;
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;

        let mut lib = Library {
            name: SmolStr::from(name),
            ..Library::default()
        };
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(lib),
                Some(Token::Ident(name)) => self.parse_library_item(&mut lib, &name)?,
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("at library top, got {other:?}"),
                    })
                }
            }
        }
    }

    fn parse_library_item(&mut self, lib: &mut Library, name: &str) -> Result<()> {
        match name {
            "cell" => {
                let cell = self.parse_cell()?;
                lib.cells.push(cell);
                Ok(())
            }
            _ => self.parse_attribute_into(name, &mut lib.attributes),
        }
    }

    fn parse_cell(&mut self) -> Result<Cell> {
        self.expect(Token::LParen)?;
        let name = self.expect_ident()?;
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut cell = Cell {
            name: SmolStr::from(name),
            ..Cell::default()
        };
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(cell),
                Some(Token::Ident(n)) => self.parse_cell_item(&mut cell, &n)?,
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in cell {}: {other:?}", cell.name),
                    })
                }
            }
        }
    }

    fn parse_cell_item(&mut self, cell: &mut Cell, name: &str) -> Result<()> {
        match name {
            "pin" => {
                let pin = self.parse_pin()?;
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
                            cell.pins.push(p);
                        }
                    } else {
                        let mut p = pin;
                        p.bus = Some(bus);
                        cell.pins.push(p);
                    }
                } else {
                    cell.pins.push(pin);
                }
                Ok(())
            }
            "area" => {
                let v = self.parse_simple_value_number()?;
                cell.area = Some(v);
                Ok(())
            }
            "cell_leakage_power" => {
                let v = self.parse_simple_value_number()?;
                cell.leakage_power = Some(v);
                Ok(())
            }
            "function" => {
                let s = self.parse_simple_value_string()?;
                cell.function = Some(SmolStr::from(s));
                Ok(())
            }
            "leakage_power" => {
                let group = self.parse_leakage_power_group()?;
                cell.leakage_groups.push(group);
                Ok(())
            }
            "statetable" => {
                let st = self.parse_statetable_group()?;
                cell.statetable = Some(st);
                Ok(())
            }
            _ => self.parse_attribute_into(name, &mut cell.attributes),
        }
    }

    fn parse_leakage_power_group(&mut self) -> Result<LeakageGroup> {
        // `leakage_power () { when : "..."; value : ...; related_pg_pin : "..."; }`
        self.expect(Token::LParen)?;
        // Skip optional inner content.
        loop {
            match self.peek()? {
                Some(Token::RParen) => break,
                _ => {
                    let _ = self.next()?;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut g = LeakageGroup::default();
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(g),
                Some(Token::Ident(n)) => match n.as_str() {
                    "when" => {
                        let s = self.parse_simple_value_string()?;
                        g.when = SmolStr::from(s.trim_matches('"').to_string());
                    }
                    "value" => {
                        g.value = self.parse_simple_value_number()?;
                    }
                    "related_pg_pin" => {
                        let s = self.parse_simple_value_string()?;
                        g.related_pg_pin =
                            Some(SmolStr::from(s.trim_matches('"').to_string()));
                    }
                    _ => self.skip_attribute_or_group()?,
                },
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in leakage_power: {other:?}"),
                    })
                }
            }
        }
    }

    fn parse_statetable_group(&mut self) -> Result<StateTable> {
        // `statetable ("input_pins", "internal_pins") { table : "..."; }`
        self.expect(Token::LParen)?;
        let inputs = match self.next()? {
            Some(Token::String(s)) => s,
            Some(Token::Ident(s)) => s,
            _ => String::new(),
        };
        // Optional comma + second arg.
        let internals = match self.peek()? {
            Some(Token::Comma) => {
                self.next()?;
                match self.next()? {
                    Some(Token::String(s)) => s,
                    Some(Token::Ident(s)) => s,
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut st = StateTable {
            input_pins: SmolStr::from(inputs),
            internal_pins: SmolStr::from(internals),
            rows: Vec::new(),
        };
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(st),
                Some(Token::Ident(n)) if n == "table" => {
                    let s = self.parse_simple_value_string()?;
                    st.rows.push(SmolStr::from(s.trim_matches('"').to_string()));
                }
                Some(Token::Ident(_)) => self.skip_attribute_or_group()?,
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in statetable: {other:?}"),
                    })
                }
            }
        }
    }

    fn parse_pin(&mut self) -> Result<Pin> {
        self.expect(Token::LParen)?;
        let name = self.expect_ident()?;
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut pin = Pin {
            name: SmolStr::from(name),
            ..Pin::default()
        };
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(pin),
                Some(Token::Ident(n)) => self.parse_pin_item(&mut pin, &n)?,
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in pin {}: {other:?}", pin.name),
                    })
                }
            }
        }
    }

    fn parse_pin_item(&mut self, pin: &mut Pin, name: &str) -> Result<()> {
        match name {
            "direction" => {
                let s = self.parse_simple_value_string()?;
                pin.direction = match s.as_str() {
                    "input" => Some(PinDirection::Input),
                    "output" => Some(PinDirection::Output),
                    "inout" => Some(PinDirection::Inout),
                    "internal" => Some(PinDirection::Internal),
                    _ => None,
                };
                Ok(())
            }
            "capacitance" => {
                pin.capacitance = Some(self.parse_simple_value_number()?);
                Ok(())
            }
            "max_capacitance" => {
                pin.max_capacitance = Some(self.parse_simple_value_number()?);
                Ok(())
            }
            "max_transition" => {
                pin.max_transition = Some(self.parse_simple_value_number()?);
                Ok(())
            }
            "function" => {
                pin.function = Some(SmolStr::from(self.parse_simple_value_string()?));
                Ok(())
            }
            "clock" => {
                let s = self.parse_simple_value_string()?;
                pin.clock = matches!(s.as_str(), "true");
                Ok(())
            }
            "timing" => {
                let arc = self.parse_timing()?;
                pin.timing.push(arc);
                Ok(())
            }
            "internal_power" => {
                let p = self.parse_internal_power_group()?;
                pin.internal_power.push(p);
                Ok(())
            }
            _ => {
                // Skip whatever this attribute is — could be a simple
                // attribute, complex attribute, or a nested group.
                self.skip_attribute_or_group()?;
                Ok(())
            }
        }
    }

    fn parse_internal_power_group(&mut self) -> Result<InternalPower> {
        // `internal_power () { when : "..."; related_pin : "..."; rise_power(template){...} fall_power(template){...} }`
        self.expect(Token::LParen)?;
        loop {
            match self.peek()? {
                Some(Token::RParen) => break,
                _ => {
                    let _ = self.next()?;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut p = InternalPower::default();
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(p),
                Some(Token::Ident(n)) => match n.as_str() {
                    "when" => {
                        let s = self.parse_simple_value_string()?;
                        p.when = SmolStr::from(s.trim_matches('"').to_string());
                    }
                    "related_pin" => {
                        let s = self.parse_simple_value_string()?;
                        p.related_pin = Some(SmolStr::from(s.trim_matches('"').to_string()));
                    }
                    "rise_power" | "fall_power" => {
                        let body = self.capture_group_body()?;
                        if n == "rise_power" {
                            p.rise_power = Some(SmolStr::from(body));
                        } else {
                            p.fall_power = Some(SmolStr::from(body));
                        }
                    }
                    _ => self.skip_attribute_or_group()?,
                },
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in internal_power: {other:?}"),
                    })
                }
            }
        }
    }

    fn parse_timing(&mut self) -> Result<TimingArc> {
        // `timing () { ... }` or sometimes `timing (label) { ... }`.
        self.expect(Token::LParen)?;
        // Skip optional inner identifier.
        loop {
            match self.peek()? {
                Some(Token::RParen) => break,
                _ => {
                    let _ = self.next()?;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.expect(Token::LBrace)?;
        let mut arc = TimingArc::default();
        loop {
            match self.next()? {
                Some(Token::RBrace) => return Ok(arc),
                Some(Token::Ident(n)) => self.parse_timing_item(&mut arc, &n)?,
                None => return Err(LibertyError::UnexpectedEof),
                other => {
                    return Err(LibertyError::UnexpectedToken {
                        pos: self.tk.pos(),
                        msg: format!("in timing: {other:?}"),
                    })
                }
            }
        }
    }

    fn parse_timing_item(&mut self, arc: &mut TimingArc, name: &str) -> Result<()> {
        match name {
            "related_pin" => {
                arc.related_pin = Some(SmolStr::from(self.parse_simple_value_string()?));
                Ok(())
            }
            "timing_sense" => {
                let s = self.parse_simple_value_string()?;
                arc.timing_sense = match s.as_str() {
                    "positive_unate" => Some(TimingSense::Positive),
                    "negative_unate" => Some(TimingSense::Negative),
                    "non_unate" => Some(TimingSense::NonUnate),
                    _ => None,
                };
                Ok(())
            }
            "timing_type" => {
                arc.timing_type = Some(SmolStr::from(self.parse_simple_value_string()?));
                Ok(())
            }
            "cell_rise" | "cell_fall" | "rise_transition" | "fall_transition" => {
                // These are nested groups: `cell_rise (template_name) { values("..."); }`.
                let body = self.capture_group_body()?;
                let smol = SmolStr::from(body);
                match name {
                    "cell_rise" => arc.cell_rise = Some(smol),
                    "cell_fall" => arc.cell_fall = Some(smol),
                    "rise_transition" => arc.rise_transition = Some(smol),
                    "fall_transition" => arc.fall_transition = Some(smol),
                    _ => unreachable!(),
                }
                Ok(())
            }
            _ => {
                self.skip_attribute_or_group()?;
                Ok(())
            }
        }
    }

    /// Capture a `(args) { body }` block as a single string for raw
    /// retention (used by lookup tables).
    fn capture_group_body(&mut self) -> Result<String> {
        // Optional `(args)` — capture if present.
        let mut out = String::new();
        if let Some(Token::LParen) = self.peek()? {
            self.next()?;
            out.push('(');
            loop {
                match self.next()? {
                    Some(Token::RParen) => {
                        out.push(')');
                        break;
                    }
                    Some(t) => append_token(&mut out, &t),
                    None => return Err(LibertyError::UnexpectedEof),
                }
            }
        }
        self.expect(Token::LBrace)?;
        out.push('{');
        let mut depth = 1;
        loop {
            match self.next()? {
                Some(Token::LBrace) => {
                    depth += 1;
                    out.push('{');
                }
                Some(Token::RBrace) => {
                    depth -= 1;
                    out.push('}');
                    if depth == 0 {
                        break;
                    }
                }
                Some(t) => append_token(&mut out, &t),
                None => return Err(LibertyError::UnexpectedEof),
            }
        }
        Ok(out)
    }

    fn parse_attribute_into(
        &mut self,
        name: &str,
        out: &mut std::collections::HashMap<SmolStr, SmolStr>,
    ) -> Result<()> {
        // Three attribute shapes:
        //   key : value ;            (simple)
        //   key (v1, v2, ...) ;      (complex)
        //   key (args) { body }      (group)
        match self.peek()? {
            Some(Token::Colon) => {
                self.next()?; // consume `:`
                let v = self.parse_value_until_semicolon()?;
                out.insert(SmolStr::from(name), SmolStr::from(v));
                Ok(())
            }
            Some(Token::LParen) => {
                self.next()?; // consume `(`
                // Read until `)`.
                let mut acc = String::new();
                let mut first = true;
                loop {
                    match self.next()? {
                        Some(Token::RParen) => break,
                        Some(Token::Comma) => {
                            acc.push(',');
                            first = true;
                        }
                        Some(t) => {
                            if !first {
                                acc.push(' ');
                            }
                            first = false;
                            append_token(&mut acc, &t);
                        }
                        None => return Err(LibertyError::UnexpectedEof),
                    }
                }
                // Check whether the next token is `{` — group form.
                match self.peek()? {
                    Some(Token::LBrace) => {
                        // Skip the body.
                        self.next()?;
                        let mut depth = 1;
                        loop {
                            match self.next()? {
                                Some(Token::LBrace) => depth += 1,
                                Some(Token::RBrace) => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                Some(_) => {}
                                None => return Err(LibertyError::UnexpectedEof),
                            }
                        }
                    }
                    Some(Token::Semicolon) => {
                        self.next()?;
                    }
                    _ => {}
                }
                out.insert(SmolStr::from(name), SmolStr::from(acc));
                Ok(())
            }
            _ => {
                // Bare keyword? Skip until `;`.
                self.parse_value_until_semicolon()?;
                Ok(())
            }
        }
    }

    fn parse_value_until_semicolon(&mut self) -> Result<String> {
        let mut acc = String::new();
        let mut first = true;
        loop {
            match self.next()? {
                Some(Token::Semicolon) => return Ok(acc),
                Some(t) => {
                    if !first {
                        acc.push(' ');
                    }
                    first = false;
                    append_token(&mut acc, &t);
                }
                None => return Err(LibertyError::UnexpectedEof),
            }
        }
    }

    fn parse_simple_value_string(&mut self) -> Result<String> {
        self.expect(Token::Colon)?;
        self.parse_value_until_semicolon()
    }

    fn parse_simple_value_number(&mut self) -> Result<f64> {
        let s = self.parse_simple_value_string()?;
        let trimmed = s.trim().trim_matches('"');
        trimmed
            .parse::<f64>()
            .map_err(|_| LibertyError::InvalidNumber(s.clone()))
    }

    fn skip_attribute_or_group(&mut self) -> Result<()> {
        match self.peek()? {
            Some(Token::Colon) => {
                self.next()?;
                self.parse_value_until_semicolon()?;
                Ok(())
            }
            Some(Token::LParen) => {
                self.next()?;
                let mut depth = 1;
                loop {
                    match self.next()? {
                        Some(Token::LParen) => depth += 1,
                        Some(Token::RParen) => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        Some(_) => {}
                        None => return Err(LibertyError::UnexpectedEof),
                    }
                }
                if let Some(Token::LBrace) = self.peek()? {
                    self.next()?;
                    let mut depth = 1;
                    loop {
                        match self.next()? {
                            Some(Token::LBrace) => depth += 1,
                            Some(Token::RBrace) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(_) => {}
                            None => return Err(LibertyError::UnexpectedEof),
                        }
                    }
                } else if let Some(Token::Semicolon) = self.peek()? {
                    self.next()?;
                }
                Ok(())
            }
            _ => {
                // Try value-until-semicolon as a fallback.
                self.parse_value_until_semicolon()?;
                Ok(())
            }
        }
    }
}

fn append_token(out: &mut String, t: &Token) {
    match t {
        Token::Ident(s) => out.push_str(s),
        Token::String(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        Token::LParen => out.push('('),
        Token::RParen => out.push(')'),
        Token::LBrace => out.push('{'),
        Token::RBrace => out.push('}'),
        Token::Comma => out.push(','),
        Token::Colon => out.push(':'),
        Token::Semicolon => out.push(';'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
library (foundry_lib) {
  delay_model : table_lookup;
  time_unit : "1ns";
  capacitive_load_unit (1, ff);

  cell (INV1) {
    area : 4.5;
    cell_leakage_power : 0.001;

    pin (A) {
      direction : input;
      capacitance : 0.005;
    }

    pin (Y) {
      direction : output;
      function : "!A";
      max_capacitance : 0.5;
      timing () {
        related_pin : "A";
        timing_sense : negative_unate;
        cell_rise (delay_template_5x5) {
          values("0.01, 0.02, 0.03, 0.04, 0.05");
        }
        cell_fall (delay_template_5x5) {
          values("0.01, 0.02, 0.03, 0.04, 0.05");
        }
      }
    }
  }

  cell (BUF1) {
    area : 8.0;
    pin (A) { direction : input; capacitance : 0.005; }
    pin (Y) { direction : output; function : "A"; }
  }
}
"#;

    #[test]
    fn parses_library_header() {
        let lib = parse_liberty(SAMPLE.as_bytes()).unwrap();
        assert_eq!(lib.name.as_str(), "foundry_lib");
        assert_eq!(
            lib.attributes
                .get("delay_model")
                .map(|s| s.as_str()),
            Some("table_lookup")
        );
        assert!(lib
            .attributes
            .get("time_unit")
            .map(|s| s.as_str())
            .is_some());
    }

    #[test]
    fn parses_cells_with_area() {
        let lib = parse_liberty(SAMPLE.as_bytes()).unwrap();
        assert_eq!(lib.cells.len(), 2);
        let inv = lib.cells.iter().find(|c| c.name == "INV1").unwrap();
        assert_eq!(inv.area, Some(4.5));
        assert_eq!(inv.leakage_power, Some(0.001));
    }

    #[test]
    fn parses_pins_with_direction() {
        let lib = parse_liberty(SAMPLE.as_bytes()).unwrap();
        let inv = lib.cells.iter().find(|c| c.name == "INV1").unwrap();
        assert_eq!(inv.pins.len(), 2);
        let a = inv.pins.iter().find(|p| p.name == "A").unwrap();
        assert_eq!(a.direction, Some(PinDirection::Input));
        assert_eq!(a.capacitance, Some(0.005));
        let y = inv.pins.iter().find(|p| p.name == "Y").unwrap();
        assert_eq!(y.direction, Some(PinDirection::Output));
        assert_eq!(y.max_capacitance, Some(0.5));
    }

    #[test]
    fn parses_timing_arc_with_lookup_tables() {
        let lib = parse_liberty(SAMPLE.as_bytes()).unwrap();
        let inv = lib.cells.iter().find(|c| c.name == "INV1").unwrap();
        let y = inv.pins.iter().find(|p| p.name == "Y").unwrap();
        assert_eq!(y.timing.len(), 1);
        let arc = &y.timing[0];
        assert_eq!(arc.related_pin.as_ref().map(|s| s.as_str()), Some("\"A\""));
        assert_eq!(arc.timing_sense, Some(TimingSense::Negative));
        assert!(arc.cell_rise.is_some());
        assert!(arc.cell_fall.is_some());
    }

    #[test]
    fn comments_are_skipped() {
        let src = r#"
            /* outer comment */
            library (lib) {
                /* nested */ time_unit : "1ns";
                cell (X) { area : 1.0; }
            }
        "#;
        let lib = parse_liberty(src.as_bytes()).unwrap();
        assert_eq!(lib.cells.len(), 1);
    }

    #[test]
    fn parses_leakage_power_groups() {
        let src = r#"
        library (lib) {
            cell (X) {
                area : 1.0;
                leakage_power () {
                    when : "!A & !B";
                    value : 0.001;
                    related_pg_pin : "VDD";
                }
                leakage_power () {
                    when : "A & !B";
                    value : 0.002;
                }
            }
        }
        "#;
        let lib = parse_liberty(src.as_bytes()).unwrap();
        let cell = &lib.cells[0];
        assert_eq!(cell.leakage_groups.len(), 2);
        assert_eq!(cell.leakage_groups[0].when.as_str(), "!A & !B");
        assert!((cell.leakage_groups[0].value - 0.001).abs() < 1e-9);
        assert_eq!(
            cell.leakage_groups[0].related_pg_pin.as_ref().map(|s| s.as_str()),
            Some("VDD")
        );
    }

    #[test]
    fn parses_statetable() {
        let src = r#"
        library (lib) {
            cell (DFF) {
                statetable ("CLK D", "Q") {
                    table : "L L : - : N";
                    table : "L H : - : N";
                    table : "H D : - : D";
                }
            }
        }
        "#;
        let lib = parse_liberty(src.as_bytes()).unwrap();
        let cell = &lib.cells[0];
        let st = cell.statetable.as_ref().unwrap();
        assert_eq!(st.input_pins.as_str(), "CLK D");
        assert_eq!(st.internal_pins.as_str(), "Q");
        assert_eq!(st.rows.len(), 3);
    }

    #[test]
    fn parses_internal_power_on_pin() {
        let src = r#"
        library (lib) {
            cell (BUF) {
                pin (Y) {
                    direction : output;
                    internal_power () {
                        when : "A";
                        related_pin : "A";
                        rise_power (template) {
                            values ("0.01, 0.02");
                        }
                        fall_power (template) {
                            values ("0.01, 0.02");
                        }
                    }
                }
            }
        }
        "#;
        let lib = parse_liberty(src.as_bytes()).unwrap();
        let pin = &lib.cells[0].pins[0];
        assert_eq!(pin.internal_power.len(), 1);
        let ip = &pin.internal_power[0];
        assert_eq!(ip.when.as_str(), "A");
        assert!(ip.rise_power.is_some());
        assert!(ip.fall_power.is_some());
    }
}
