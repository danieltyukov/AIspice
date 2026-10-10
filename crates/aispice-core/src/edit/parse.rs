//! Reading the edit list a model sent, one edit at a time.
//!
//! Deserializing the whole list at once gives serde's bare message ("missing
//! field `net`") with no word on which edit has the mistake. Here each edit
//! is read on its own, and an error names the edit by its position in the
//! list (counting from 0, as the apply-time errors do) and its op, and says
//! which fields that op takes. A few harmless aliases that models often send
//! are accepted, only where they cannot mean anything else for that op.

use super::{EditError, EditOp};
use serde_json::{Map, Value};

/// Every op with its fields and the form shown in errors.
pub const OPS: &[(&str, &[&str], &str)] = &[
    (
        "add_component",
        &["symbol", "name", "value", "at", "orient", "near", "attrs"],
        "{symbol; optional name, value, orient, near, at: [x, y], attrs}",
    ),
    ("remove", &["name"], "{name}"),
    ("replace_symbol", &["name", "symbol"], "{name, symbol}"),
    ("move", &["name", "to"], "{name, to: [x, y]}"),
    ("rotate", &["name", "orient"], "{name; optional orient}"),
    ("set_value", &["name", "value"], "{name, value}"),
    ("set_attr", &["name", "key", "value"], "{name, key, value}"),
    ("rename", &["name", "new_name"], "{name, new_name}"),
    ("connect", &["from", "to"], "{from: PIN, to: PIN}"),
    ("connect_to_net", &["pin", "net"], "{pin, net}"),
    ("disconnect", &["pin"], "{pin}"),
    ("add_wire", &["from", "to"], "{from: [x, y], to: [x, y]}"),
    ("remove_wire", &["from", "to"], "{from: [x, y], to: [x, y]}"),
    ("add_label", &["at", "label"], "{at: [x, y], label}"),
    (
        "remove_label",
        &["label", "at"],
        "{label; optional at: [x, y]}",
    ),
    (
        "add_directive",
        &["text", "at"],
        "{text; optional at: [x, y]}",
    ),
    ("remove_directive", &["matching"], "{matching}"),
    (
        "replace_directive",
        &["matching", "text"],
        "{matching, text}",
    ),
    (
        "add_comment",
        &["text", "at"],
        "{text; optional at: [x, y]}",
    ),
];

/// Op names models use for the real ones.
const OP_ALIASES: &[(&str, &str)] = &[
    ("set_attribute", "set_attr"),
    ("remove_component", "remove"),
    ("delete_component", "remove"),
    ("delete", "remove"),
    ("add_part", "add_component"),
];

/// Field aliases per op: `(op, alias, field)`. The alias is used only when
/// the real field is absent.
pub const FIELD_ALIASES: &[(&str, &str, &str)] = &[
    ("connect_to_net", "to", "net"),
    ("connect_to_net", "net_name", "net"),
    ("connect_to_net", "from", "pin"),
    ("connect", "pin", "from"),
    ("set_attr", "attr", "key"),
    ("set_attr", "attribute", "key"),
    ("move", "at", "to"),
    ("rename", "to", "new_name"),
    ("add_label", "net", "label"),
    ("remove_label", "net", "label"),
    ("add_directive", "directive", "text"),
    ("replace_directive", "directive", "text"),
    ("remove_directive", "text", "matching"),
];

/// Ops that name a component in `name`, which also accept `component` or
/// `part` for it.
const NAMED: &[&str] = &[
    "add_component",
    "remove",
    "replace_symbol",
    "move",
    "rotate",
    "set_value",
    "set_attr",
    "rename",
];

/// Fields that hold a point `[x, y]`.
const POINTS: &[&str] = &["at", "to", "from"];

fn op_names() -> String {
    OPS.iter()
        .map(|(n, _, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A point given as `{"x": .., "y": ..}`, or with whole numbers written as
/// floats, becomes `[x, y]`. Anything else is left for serde to report.
fn normalize_point(v: &mut Value) {
    let pair = match v {
        Value::Object(m) => match (m.get("x"), m.get("y")) {
            (Some(x), Some(y)) => Some((x.clone(), y.clone())),
            _ => None,
        },
        Value::Array(a) if a.len() == 2 => Some((a[0].clone(), a[1].clone())),
        _ => None,
    };
    let whole = |n: &Value| -> Option<i64> {
        let f = n.as_f64()?;
        (f.fract() == 0.0 && f.abs() < 1e12).then_some(f as i64)
    };
    if let Some((x, y)) = pair
        && let (Some(x), Some(y)) = (whole(&x), whole(&y))
    {
        *v = Value::Array(vec![x.into(), y.into()]);
    }
}

/// Bring one edit object into the shape [`EditOp`] reads: real op name, real
/// field names, values as text. Returns the op name and the fields it did
/// not recognise.
fn normalize(obj: &mut Map<String, Value>) -> Option<(String, Vec<String>)> {
    if !obj.contains_key("op")
        && let Some(a) = obj.remove("action")
    {
        obj.insert("op".into(), a);
    }
    let op = obj.get("op")?.as_str()?.trim().to_ascii_lowercase();
    let op = OP_ALIASES
        .iter()
        .find(|(alias, _)| *alias == op)
        .map(|(_, real)| real.to_string())
        .unwrap_or(op);
    obj.insert("op".into(), Value::String(op.clone()));
    for (o, alias, field) in FIELD_ALIASES {
        if *o == op
            && !obj.contains_key(*field)
            && let Some(v) = obj.remove(*alias)
        {
            obj.insert(field.to_string(), v);
        }
    }
    if NAMED.contains(&op.as_str()) && !obj.contains_key("name") {
        for alias in ["component", "part"] {
            if let Some(v) = obj.remove(alias) {
                obj.insert("name".into(), v);
                break;
            }
        }
    }
    // A value or text given as a number is meant as that number.
    for key in ["value", "text", "net", "label", "new_name", "name"] {
        if let Some(n) = obj.get(key).filter(|v| v.is_number()).cloned() {
            obj.insert(key.into(), Value::String(n.to_string()));
        }
    }
    for key in POINTS {
        if let Some(v) = obj.get_mut(*key) {
            normalize_point(v);
        }
    }
    let known: &[&str] = OPS
        .iter()
        .find(|(n, _, _)| *n == op)
        .map(|(_, f, _)| *f)
        .unwrap_or(&[]);
    let ignored = obj
        .keys()
        .filter(|k| k.as_str() != "op" && !known.contains(&k.as_str()))
        .cloned()
        .collect();
    Some((op, ignored))
}

/// Read a list of edits. Returns the edits and notes about fields that were
/// ignored, or the first mistake, naming the edit.
pub fn parse_edits(values: &[Value]) -> Result<(Vec<EditOp>, Vec<String>), EditError> {
    let mut ops = Vec::with_capacity(values.len());
    let mut notes = Vec::new();
    for (index, v) in values.iter().enumerate() {
        let Value::Object(obj) = v else {
            return Err(EditError::Malformed {
                index,
                message: format!(
                    "each edit is an object with an `op` and its fields, found {}",
                    kind_of(v)
                ),
            });
        };
        let mut obj = obj.clone();
        let Some((op, ignored)) = normalize(&mut obj) else {
            return Err(EditError::Malformed {
                index,
                message: format!("no `op`; give one of: {}", op_names()),
            });
        };
        let Some((_, _, form)) = OPS.iter().find(|(n, _, _)| *n == op) else {
            return Err(EditError::InBatch {
                index,
                op: op.clone(),
                source: Box::new(EditError::Schema(format!(
                    "unknown op; the ops are: {}",
                    op_names()
                ))),
            });
        };
        match serde_json::from_value::<EditOp>(Value::Object(obj)) {
            Ok(e) => ops.push(e),
            Err(e) => {
                return Err(EditError::InBatch {
                    index,
                    op,
                    source: Box::new(EditError::Schema(format!("{e}; this op takes {form}"))),
                });
            }
        }
        if !ignored.is_empty() {
            notes.push(format!(
                "edit {index} ({op}): ignored {} {}; this op takes {form}",
                if ignored.len() == 1 {
                    "field"
                } else {
                    "fields"
                },
                ignored
                    .iter()
                    .map(|k| format!("`{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok((ops, notes))
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> Result<(Vec<EditOp>, Vec<String>), String> {
        parse_edits(v.as_array().unwrap()).map_err(|e| e.to_string())
    }

    #[test]
    fn errors_name_the_edit_and_its_fields() {
        let e = parse(json!([{"op": "set_value", "name": "R1", "value": "1k"}, {"op": "connect_to_net", "pin": "R1.A"}])).unwrap_err();
        assert_eq!(
            e,
            "edit 1 (connect_to_net): missing field `net`; this op takes {pin, net}"
        );
        let e = parse(json!([{"op": "add_wire", "from": [0, 0], "to": [1, 2, 3]}])).unwrap_err();
        assert!(e.starts_with("edit 0 (add_wire): invalid length 3"), "{e}");
        assert!(
            e.ends_with("this op takes {from: [x, y], to: [x, y]}"),
            "{e}"
        );
        assert!(
            parse(json!(["R1"]))
                .unwrap_err()
                .starts_with("edit 0: each edit is an object")
        );
    }

    #[test]
    fn harmless_aliases_are_accepted() {
        let (ops, notes) = parse(json!([
            {"op": "connect_to_net", "pin": "R1.A", "to": "out"},
            {"op": "set_attribute", "component": "U1", "attr": "SpiceLine2", "value": "GBW=1Meg"},
            {"op": "set_value", "name": "R1", "value": 4700},
            {"op": "add_label", "at": {"x": 16, "y": 32.0}, "label": "x"},
            {"action": "remove", "part": "R9"}
        ]))
        .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            ops[0],
            EditOp::ConnectToNet {
                pin: "R1.A".into(),
                net: "out".into()
            }
        );
        assert_eq!(
            ops[1],
            EditOp::SetAttr {
                name: "U1".into(),
                key: "SpiceLine2".into(),
                value: "GBW=1Meg".into()
            }
        );
        assert_eq!(
            ops[2],
            EditOp::SetValue {
                name: "R1".into(),
                value: "4700".into()
            }
        );
        assert_eq!(
            ops[3],
            EditOp::AddLabel {
                at: [16, 32],
                label: "x".into()
            }
        );
        assert_eq!(ops[4], EditOp::Remove { name: "R9".into() });
    }

    #[test]
    fn unknown_fields_are_noted() {
        let (_, notes) =
            parse(json!([{"op": "connect", "from": "R1.A", "to": "R2.B", "net": "x"}])).unwrap();
        assert_eq!(
            notes,
            vec!["edit 0 (connect): ignored field `net`; this op takes {from: PIN, to: PIN}"]
        );
    }

    #[test]
    fn the_table_covers_every_op() {
        let schema = super::super::flat_edit_schema();
        let listed: Vec<&str> = schema["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let table: Vec<&str> = OPS.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(listed, table);
    }
}
