use super::*;

/// Parse the text of an `.asy` file. Unknown lines are ignored: a symbol is
/// only ever read, never written back.
pub fn parse_asy(text: &str) -> SymbolDef {
    let mut def = SymbolDef::default();
    for raw in text.lines() {
        let line = raw.trim();
        let (keyword, rest) = split_first(line);
        let nums = |n: usize| -> Option<Vec<i32>> {
            let v: Vec<i32> = rest
                .split_whitespace()
                .skip(1)
                .take(n)
                .map_while(|t| t.parse().ok())
                .collect();
            (v.len() == n).then_some(v)
        };
        match keyword {
            "SymbolType" => {
                def.kind = if rest.trim().eq_ignore_ascii_case("BLOCK") {
                    SymbolType::Block
                } else {
                    SymbolType::Cell
                };
            }
            "LINE" => {
                if let Some(v) = nums(4) {
                    def.graphics.push(Graphic::Line {
                        a: Point::new(v[0], v[1]),
                        b: Point::new(v[2], v[3]),
                    });
                }
            }
            "RECTANGLE" => {
                if let Some(v) = nums(4) {
                    def.graphics.push(Graphic::Rect {
                        a: Point::new(v[0], v[1]),
                        b: Point::new(v[2], v[3]),
                    });
                }
            }
            "CIRCLE" => {
                if let Some(v) = nums(4) {
                    def.graphics.push(Graphic::Circle {
                        a: Point::new(v[0], v[1]),
                        b: Point::new(v[2], v[3]),
                    });
                }
            }
            "ARC" => {
                if let Some(v) = nums(8) {
                    def.graphics.push(Graphic::Arc {
                        a: Point::new(v[0], v[1]),
                        b: Point::new(v[2], v[3]),
                        start: Point::new(v[4], v[5]),
                        end: Point::new(v[6], v[7]),
                    });
                }
            }
            "TEXT" => {
                // TEXT x y align size text
                let mut parts = rest.splitn(5, char::is_whitespace);
                let parsed = (|| {
                    let x = parts.next()?.parse().ok()?;
                    let y = parts.next()?.parse().ok()?;
                    let align = parts.next()?.to_string();
                    let size = parts.next()?.parse().ok()?;
                    let text = parts.next().unwrap_or("").trim().to_string();
                    Some(Graphic::Text {
                        at: Point::new(x, y),
                        align,
                        size,
                        text,
                    })
                })();
                if let Some(g) = parsed {
                    def.graphics.push(g);
                }
            }
            "WINDOW" => {
                let mut parts = rest.split_whitespace();
                let parsed = (|| {
                    let index = parts.next()?.parse().ok()?;
                    let x = parts.next()?.parse().ok()?;
                    let y = parts.next()?.parse().ok()?;
                    let align = parts.next()?.to_string();
                    let size = parts.next().and_then(|s| s.parse().ok()).unwrap_or(2);
                    Some(WindowDef {
                        index,
                        at: Point::new(x, y),
                        align,
                        size,
                    })
                })();
                if let Some(w) = parsed {
                    def.windows.push(w);
                }
            }
            "SYMATTR" => {
                let (key, value) = split_first(rest);
                def.attrs.push((key.to_string(), value.to_string()));
            }
            "PIN" => {
                let mut parts = rest.split_whitespace();
                let parsed = (|| {
                    let x = parts.next()?.parse().ok()?;
                    let y = parts.next()?.parse().ok()?;
                    let justification = parts.next().unwrap_or("NONE").to_string();
                    let label_offset = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    Some(PinDef {
                        at: Point::new(x, y),
                        name: String::new(),
                        spice_order: def.pins.len() as u32 + 1,
                        justification,
                        label_offset,
                    })
                })();
                if let Some(p) = parsed {
                    def.pins.push(p);
                }
            }
            "PINATTR" => {
                let (key, value) = split_first(rest);
                if let Some(pin) = def.pins.last_mut() {
                    if key.eq_ignore_ascii_case("PinName") {
                        pin.name = value.to_string();
                    } else if key.eq_ignore_ascii_case("SpiceOrder")
                        && let Ok(order) = value.trim().parse()
                    {
                        pin.spice_order = order;
                    }
                }
            }
            _ => {}
        }
    }
    for (i, pin) in def.pins.iter_mut().enumerate() {
        if pin.name.is_empty() {
            pin.name = format!("{}", i + 1);
        }
    }
    def
}

fn split_first(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = "Version 4\r\nSymbolType CELL\r\nLINE Normal 16 88 16 96\r\nWINDOW 0 36 40 Left 2\r\nWINDOW 3 36 76 Left 2\r\nSYMATTR Value R\r\nSYMATTR Prefix R\r\nSYMATTR Description A resistor\r\nPIN 16 16 NONE 0\r\nPINATTR PinName A\r\nPINATTR SpiceOrder 1\r\nPIN 16 96 NONE 0\r\nPINATTR PinName B\r\nPINATTR SpiceOrder 2\r\n";

    #[test]
    fn reads_pins_and_attributes() {
        let def = parse_asy(RES);
        assert_eq!(def.kind, SymbolType::Cell);
        assert_eq!(def.prefix(), "R");
        assert_eq!(def.description(), Some("A resistor"));
        assert_eq!(def.pins.len(), 2);
        assert_eq!(def.pins[0].at, Point::new(16, 16));
        assert_eq!(def.pins[1].name, "B");
        assert_eq!(def.windows.len(), 2);
        assert_eq!(def.graphics.len(), 1);
    }

    #[test]
    fn spice_order_can_differ_from_file_order() {
        let text = "SymbolType CELL\nPIN 0 96 NONE 0\nPINATTR PinName +\nPINATTR SpiceOrder 2\nPIN 0 16 NONE 0\nPINATTR PinName -\nPINATTR SpiceOrder 1\n";
        let def = parse_asy(text);
        let order: Vec<&str> = def
            .pins_in_spice_order()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(order, vec!["-", "+"]);
    }

    #[test]
    fn placed_pin_positions() {
        let def = parse_asy(RES);
        let origin = Point::new(192, 80);
        let a = def.pin_position(&def.pins[0], origin, Orient::R90);
        let b = def.pin_position(&def.pins[1], origin, Orient::R90);
        assert_eq!(a, Point::new(176, 96));
        assert_eq!(b, Point::new(96, 96));
    }
}
