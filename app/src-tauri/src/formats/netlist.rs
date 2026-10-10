use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Netlist {
    pub title: String,
    pub components: Vec<NetlistComponent>,
    pub directives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetlistComponent {
    /// The full reference designator, e.g. "R1", "C2", "V1".
    pub name: String,
    /// Component kind inferred from the first character of the name.
    pub kind: ComponentKind,
    /// Node connections (typically 2 for passives, 3 for transistors, etc.).
    pub nodes: Vec<String>,
    /// The value / model string that follows the node list.
    pub value: String,
    /// Auto-generated layout coordinates (since netlists carry none).
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ComponentKind {
    Resistor,
    Capacitor,
    Inductor,
    VoltageSource,
    CurrentSource,
    Diode,
    Transistor,
    Other,
}

impl ComponentKind {
    /// Infer the component kind from the first character of the SPICE name.
    pub fn from_prefix(ch: char) -> Self {
        match ch.to_ascii_uppercase() {
            'R' => Self::Resistor,
            'C' => Self::Capacitor,
            'L' => Self::Inductor,
            'V' => Self::VoltageSource,
            'I' => Self::CurrentSource,
            'D' => Self::Diode,
            'Q' => Self::Transistor,
            _ => Self::Other,
        }
    }

    /// How many node connections a component of this kind typically has.
    pub fn expected_nodes(self) -> usize {
        match self {
            Self::Resistor | Self::Capacitor | Self::Inductor => 2,
            Self::VoltageSource | Self::CurrentSource => 2,
            Self::Diode => 2,
            Self::Transistor => 3,
            Self::Other => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a SPICE netlist (.cir / .spice / .net) from its text content.
///
/// Conventions:
/// - The first non-comment, non-blank line is the circuit title.
/// - Lines starting with `*` are comments and are skipped.
/// - Lines starting with `.` are SPICE directives.
/// - `.end` (case-insensitive) terminates parsing.
/// - Continuation lines (starting with `+`) are appended to the previous
///   logical line.
/// - Everything else is treated as a component instance line.
pub fn parse_netlist(content: &str) -> Result<Netlist, String> {
    let raw_lines: Vec<&str> = content.lines().collect();

    // ---- Merge continuation lines ----
    let mut logical_lines: Vec<String> = Vec::new();
    for raw in &raw_lines {
        let trimmed = raw.trim();
        if trimmed.starts_with('+') {
            // Append to previous logical line
            if let Some(prev) = logical_lines.last_mut() {
                prev.push(' ');
                prev.push_str(trimmed[1..].trim());
            }
        } else {
            logical_lines.push(trimmed.to_string());
        }
    }

    // ---- Extract title (first non-empty, non-comment line) ----
    let mut title = String::new();
    let mut start_idx = 0;
    for (idx, line) in logical_lines.iter().enumerate() {
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        // The very first non-comment line is the title per SPICE convention.
        title = line.clone();
        start_idx = idx + 1;
        break;
    }

    let mut components: Vec<NetlistComponent> = Vec::new();
    let mut directives: Vec<String> = Vec::new();

    // Vertical spacing for auto-layout
    let x_base = 100;
    let y_spacing = 120;
    let mut comp_index: i32 = 0;

    for line in &logical_lines[start_idx..] {
        if line.is_empty() || line.starts_with('*') {
            continue;
        }

        // .end terminates the netlist
        if line.eq_ignore_ascii_case(".end") {
            break;
        }

        // Directives
        if line.starts_with('.') {
            directives.push(line.clone());
            continue;
        }

        // Component instance line
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let name = parts[0].to_string();
        let first_char = name.chars().next().unwrap_or('X');
        let kind = ComponentKind::from_prefix(first_char);
        let n_nodes = kind.expected_nodes();

        // After the name come the nodes, then the value/model
        let (nodes, value) = if parts.len() > n_nodes + 1 {
            let nodes: Vec<String> = parts[1..=n_nodes].iter().map(|s| s.to_string()).collect();
            let value = parts[n_nodes + 1..].join(" ");
            (nodes, value)
        } else if parts.len() > 1 {
            // Not enough tokens for the expected node count — best-effort
            let nodes: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
            (nodes, String::new())
        } else {
            (Vec::new(), String::new())
        };

        components.push(NetlistComponent {
            name,
            kind,
            nodes,
            value,
            x: x_base,
            y: comp_index * y_spacing,
        });
        comp_index += 1;
    }

    Ok(Netlist {
        title,
        components,
        directives,
    })
}

// ---------------------------------------------------------------------------
// Writer (for completeness / round-trip)
// ---------------------------------------------------------------------------

/// Serialise a `Netlist` back to SPICE text.
#[allow(dead_code)]
pub fn write_netlist(netlist: &Netlist) -> String {
    let mut out = String::new();

    out.push_str(&netlist.title);
    out.push('\n');

    for comp in &netlist.components {
        out.push_str(&comp.name);
        for node in &comp.nodes {
            out.push(' ');
            out.push_str(node);
        }
        if !comp.value.is_empty() {
            out.push(' ');
            out.push_str(&comp.value);
        }
        out.push('\n');
    }

    for dir in &netlist.directives {
        out.push_str(dir);
        out.push('\n');
    }

    out.push_str(".end\n");
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
RC Low-Pass Filter
* Simple first-order RC filter
R1 in out 1k
C1 out 0 100n
V1 in 0 AC 1
.ac dec 10 1 1meg
.end
";

    #[test]
    fn test_parse_basic() {
        let n = parse_netlist(SAMPLE).unwrap();
        assert_eq!(n.title, "RC Low-Pass Filter");
        assert_eq!(n.components.len(), 3);
        assert_eq!(n.directives.len(), 1);

        assert_eq!(n.components[0].name, "R1");
        assert_eq!(n.components[0].kind, ComponentKind::Resistor);
        assert_eq!(n.components[0].nodes, vec!["in", "out"]);
        assert_eq!(n.components[0].value, "1k");

        assert_eq!(n.components[1].kind, ComponentKind::Capacitor);
        assert_eq!(n.components[2].kind, ComponentKind::VoltageSource);
    }

    #[test]
    fn test_auto_layout_coordinates() {
        let n = parse_netlist(SAMPLE).unwrap();
        assert_eq!(n.components[0].y, 0);
        assert_eq!(n.components[1].y, 120);
        assert_eq!(n.components[2].y, 240);
    }

    #[test]
    fn test_continuation_lines() {
        let input = "\
Test
R1 a b
+ 1k
.end
";
        let n = parse_netlist(input).unwrap();
        // R1 with continuation should merge: "R1 a b 1k"
        assert_eq!(n.components.len(), 1);
        assert_eq!(n.components[0].name, "R1");
    }
}
