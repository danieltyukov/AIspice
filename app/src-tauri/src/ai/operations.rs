use serde::{Deserialize, Serialize};

/// A single edit operation the AI wants to apply to a schematic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum EditOperation {
    #[serde(rename = "set_value")]
    SetValue { target: String, value: String },

    #[serde(rename = "add_component")]
    AddComponent {
        #[serde(rename = "type")]
        comp_type: String,
        position: [i32; 2],
        rotation: String,
        name: String,
        value: String,
    },

    #[serde(rename = "add_wire")]
    AddWire { from: [i32; 2], to: [i32; 2] },

    #[serde(rename = "remove")]
    Remove { target: String },

    #[serde(rename = "move")]
    MoveComponent { target: String, position: [i32; 2] },

    #[serde(rename = "add_flag")]
    AddFlag { position: [i32; 2], label: String },

    #[serde(rename = "add_directive")]
    AddDirective { position: [i32; 2], text: String },

    #[serde(rename = "remove_text")]
    RemoveText { text: String },

    #[serde(rename = "remove_wire")]
    RemoveWire { from: [i32; 2], to: [i32; 2] },
}

/// The top-level response the AI returns when it wants to edit a schematic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditResponse {
    pub operations: Vec<EditOperation>,
    pub explanation: String,
    pub changes: Vec<ChangeEntry>,
}

/// A human-readable summary of one change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub component: Option<String>,
    pub description: String,
    pub filename: String,
}
