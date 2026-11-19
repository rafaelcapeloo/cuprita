//! Cuprita core: KiCad file parsing and the board model. No GUI or GPU dependencies.

pub mod analysis;
pub mod board;
mod body;
pub mod bom;
pub mod diff;
#[cfg(not(target_arch = "wasm32"))]
pub mod digikey;
pub mod drawing;
pub mod drc;
mod fab;
mod fill;
pub mod font;
mod gerber;
pub mod history;
pub mod json;
pub mod library;
pub mod models;
pub mod new;
pub mod newpart;
#[cfg(not(target_arch = "wasm32"))]
pub mod ngspice;
#[cfg(not(target_arch = "wasm32"))]
pub mod ollama;
mod outline;
pub mod pair;
mod parse;
pub mod parts;
mod pdf;
mod picked;
mod polygon;
pub mod ratsnest;
pub mod route;
mod save;
pub mod schematic;
pub mod sexpr;
pub mod shove;
pub mod step;
mod threads;
pub mod tidy;
pub mod tr;
mod tracks;
mod tune;
pub mod units;
pub mod update;
pub mod zip;
mod zone;
