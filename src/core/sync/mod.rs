//! Shared sync coordinator and HUD rendering.

pub mod coordinator;
pub mod renderer;
pub mod state;

pub use coordinator::SyncCoordinator;
pub use state::{Stage, SyncMode, SyncState};
