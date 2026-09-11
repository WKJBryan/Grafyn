//! Multi-LLM canvas: the `#[tauri::command]` surface for canvas sessions,
//! prompt streaming, model debates, and twin/note context assembly.
//!
//! Split into focused submodules (Task 4.1):
//! - `shared` — model-routing helpers and the trace-append helper used by
//!   every command handler below.
//! - `context` — note/twin context-prompt assembly, including the twin
//!   Simulation identity gate.
//! - `streaming` — `send_prompt`, `add_models_to_tile`, `regenerate_response`
//!   and their per-model stream task helpers.
//! - `debate` — `start_debate`, `continue_debate`, and debate streaming.
//! - `session` — session CRUD, tile/node positions, viewport, auto-arrange,
//!   and note export.
//!
//! Command names and signatures are unchanged from the pre-split
//! `canvas.rs` — `main.rs`'s `generate_handler!` list still resolves them
//! via the glob re-exports below.

mod shared;

mod prediction_terminality;

mod working_memory;

mod context;

mod streaming;
pub use streaming::*;

#[cfg(desktop)]
mod debate;
#[cfg(desktop)]
pub use debate::*;

mod session;
pub use session::*;

#[cfg(test)]
mod test_support;
