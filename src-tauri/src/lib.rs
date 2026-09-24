//! cs16browser — an external Counter-Strike 1.6 server browser as a Tauri app.
//!
//! The server list comes from Steam's `IGameServersService` Web API over
//! HTTPS, so the crate has no 32-bit constraint and no `steam_api.dll`
//! dependency. Pipeline logic lives in [`app`]; `gui` is only the Tauri
//! command surface over it.
//!
//! Reverse-engineering notes for the wire protocols live in the module docs of
//! `protocol::a2s` and `client::webapi`.
#[cfg(target_pointer_width = "32")]
compile_error!(
    "cs16browser is 64-bit only. The 32-bit requirement was dropped when the \
     `steam_api.dll` matchmaking path was replaced by Steam's Web API, so \
     build natively (e.g. `cargo build --release` on x86_64)."
);

pub mod app;
pub mod banlist;
pub mod client;
pub mod country;
pub mod detect;
pub mod gui;
pub mod history;
pub mod logging;
pub mod model;
pub mod protocol;
pub mod ratelimit;
pub mod scanner;
pub mod settings;
pub mod store;
