//! # sr-model
//!
//! Loads a scene-render 1.1 document into a typed, fully resolved model or
//! returns every problem found, each with a stable code and a source position.
//!
//! Loading runs four stages over one parse of the file:
//!
//! 1. **Structure** ([`xsd`]): the XSD 1.0 contract — element content models,
//!    attribute declarations, simple-type facets, `xs:ID` uniqueness and
//!    `xs:IDREF` resolution. The rules are tables generated from
//!    `schema/scene-render-1.1.xsd` at build time.
//! 2. **Rules** ([`rules`]): the 41 Schematron patterns of
//!    `schema/scene-render-1.1.sch`, including the 1.0/1.1 version gate,
//!    evaluated with XPath 1.0 semantics.
//! 3. **Assets** ([`assets`]): existence of every referenced input file and
//!    SHA-256 verification of every declared hash, including generated media
//!    and transcription caches.
//! 4. **Model** ([`model`], [`document`]): construction of the typed model and
//!    the ID index that later stages use to resolve references.
//!
//! ```no_run
//! use sr_model::{load_file, LoadOptions};
//!
//! match load_file("promo.scene.xml", &LoadOptions::default()) {
//!     Ok(doc) => println!("{} frames", doc.frame_count()),
//!     Err(e) => eprintln!("{e}"),
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod assets;
pub mod codes;
pub mod diag;
pub mod document;
pub mod effect_attrs;
pub mod element;
pub mod foam;
pub mod inert;
pub mod model;
pub mod parse;
pub mod program;
mod program_ids;
pub mod rules;
pub mod values;
pub mod xsd;

pub use diag::{Diagnostic, Loc, Measure, Report, Severity};
pub use document::{
    load_file, load_str, validate_file, validate_str, Document, Index, LoadError, LoadOptions, NodePath, NodeRoot,
    ResolvedPaint, Target, Version,
};

/// Schema version implemented by this crate: the newest document version it accepts, that of the vendored sr-core
/// schema 1.7.0 (schema/UPSTREAM). Version 1.7 adds no syntax (SREP 87 has no version gate), so a 1.7 document means
/// what it means as 1.6; a 1.6 document with no 1.6 syntax holds 1.5 content and means what it means as 1.5.
pub const SCHEMA_VERSION: &str = "1.7";
