//! The crate's one test harness (bmo-chat's docs/conventions.md, 「Rust builds」): each file of tests a
//! module of it, sharing the process.

mod catalogue;
mod commands;
mod external;
mod host;
mod personal;
mod plugin;
mod registry;
mod screens;
mod units;
