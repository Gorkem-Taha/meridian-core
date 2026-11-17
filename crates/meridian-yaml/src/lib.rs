//! YAML as Meridian reads and writes it.
//!
//! [`scalar`] decides what an unquoted scalar means when a document is read.
//! [`write`] turns a tree of values into block-style YAML.
//!
//! The writer's output is byte-for-byte what gopkg.in/yaml.v3 produces for
//! the same tree, because that is what the first Meridian targets were
//! written with and their golden files record. It decides, as that library
//! does, when a string can be written plainly, when it must be quoted so it
//! is not read back as a number, a boolean, a date or nothing at all, and
//! when it is better as a literal block.

pub mod scalar;
mod writer;

pub use writer::{write, Yaml};
