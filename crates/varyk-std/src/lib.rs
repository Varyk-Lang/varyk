//! The runtime that generated Varyk programs call into. Generated Rust
//! names only this crate (`::varyk_std::...`), never serde or tracing.

mod buffer;
mod dotenv;
pub mod env;
mod error;
mod ids;
pub mod json;
mod log;
mod parse;
mod runtime;
mod task;
pub mod time;
mod timestamp;
mod value;

pub use buffer::Bytes;
pub use error::Error;
pub use ids::Uuid;
pub use log::start;
pub use parse::{Parse, parse};
pub use runtime::run;
pub use serde;
pub use task::Task;
pub use timestamp::Time;
pub use tracing;
pub use value::Value;

/// Test helper: runs `f` with a silent panic hook so expected panics keep the
/// test output clean. One lock covers every panic test, since the hook is
/// process-wide.
#[cfg(test)]
pub(crate) fn quietly<R>(f: impl FnOnce() -> R) -> R {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(previous);
    out
}
