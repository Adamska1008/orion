//! Local server discovery and lifecycle. Independent of Tauri and the scan engine.
mod discovery;
mod http_client;
mod process;
mod supervisor;
pub use discovery::Connection;
pub use supervisor::{Backend, BackendStatus, ExitError};
