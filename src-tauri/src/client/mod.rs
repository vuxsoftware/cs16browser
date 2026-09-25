//! Server-list sources for the browser.
//!
//! `webapi` talks to Steam's `IGameServersService` over HTTPS with a Web API
//! key. That is the standalone replacement for the in-game browser's
//! matchmaking path — see the module docs in `webapi.rs` for the reverse
//! engineering that established it.

pub mod webapi;

pub use webapi::{ApiServer, WebApi, API_KEY_URL};
