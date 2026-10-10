//! SPICE netlists: reading and writing them, and deriving one from a schematic.

pub mod connect;
pub mod spice;

pub use connect::{Connectivity, Net, PinRef, connect};
pub use spice::{Element, Line, Netlist, Subckt, parse, write};
