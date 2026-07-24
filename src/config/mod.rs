//! Native-mode `.ini` generation. NOT used in docker mode (docker configures
//! purely via compose + `ARKIME__*` env — see `actions::docker`).

pub mod substitute;
pub mod templates;

pub use substitute::{
    basic_auth_value, get_ini_key, inject_basic_auth, render, set_ini_key, set_ini_key_opt,
    BasicAuthEncoding, Substitutions,
};
pub use templates::{load_sample, write_if_absent, SampleKind, WriteOutcome};
