//! aispice-core: the schematic, symbol and netlist engine behind aispice.
//!
//! Everything here is pure computation over files: no processes, no network.
//! Simulators live in `aispice-sim`, the language model side in `aispice-agent`.

pub mod encoding;
pub mod geometry;
pub mod render;
pub mod schematic;
pub mod symbol;
pub mod units;

pub use geometry::{Orient, Point};
pub use schematic::Schematic;
