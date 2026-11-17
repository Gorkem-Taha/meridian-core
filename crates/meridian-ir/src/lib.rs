//! Meridian's intermediate representation: the document types, the YAML/JSON
//! parser that produces them, and the validation that every later stage relies
//! on having passed.
//!
//! Nothing in this crate panics on malformed input. Every failure is an
//! [`Error`] carrying the source position it was found at, and a parse or
//! validation pass reports every problem it found, not only the first.

mod error;
mod node;
mod parse;
mod types;
mod validate;

pub use error::{Error, ErrorList, Position};
pub use meridian_value::{canonical_map, Value};
pub use parse::{parse, parse_file, parse_value};
pub use types::{
    Dependency, Document, IfMissing, Infrastructure, InfrastructureSpec, Kind, Metadata,
    Notification, Output, Resource, ResourceSet, ResourceSetSpec, TargetHost, API_VERSION_V1,
    TARGET_HOST_FROM_INFRASTRUCTURE, TARGET_HOST_FROM_INLINE,
};
pub use validate::{load, validate, Bundle};
