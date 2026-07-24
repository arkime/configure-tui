//! Arkime setup TUI — library crate holding all testable logic. The `main.rs`
//! binary is a thin shell around `app::run`.

pub mod actions;
pub mod app;
pub mod config;
pub mod domain;
pub mod guards;
pub mod interfaces;
pub mod log;
pub mod steps;
pub mod ui;
