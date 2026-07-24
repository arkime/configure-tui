//! Native-mode `.ini` generation. NOT used in docker mode (docker configures
//! purely via compose + `ARKIME__*` env — see `actions::docker`).

pub mod substitute;
pub mod templates;

pub use substitute::{
    basic_auth_value, inject_basic_auth, render, set_ini_key, BasicAuthEncoding, Substitutions,
};
pub use templates::{load_sample, write_if_absent, SampleKind, WriteOutcome};
