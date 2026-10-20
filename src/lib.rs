//! zwallet: a local, login-free wallet CLI for agents.
//!
//! Layout: `core` (chain-agnostic custody, protocol, amounts), `chains` (registry + family
//! boundary; `chains::evm` today), `daemon` (the key holder), `cli` + `commands` (one-shot CLI).

pub mod chains;
pub mod cli;
pub mod commands;
pub mod core;
pub mod daemon;
