//! The runtime that generated Varyk programs call into. Generated Rust
//! names only this crate (`::varyk_std::...`), never serde or tracing.

mod dotenv;
pub mod env;
mod error;
pub mod json;
mod log;
mod parse;

pub use error::Error;
pub use log::start;
pub use parse::{Parse, parse};
pub use serde;
pub use tracing;
