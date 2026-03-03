//! Secret management for the pince supervisor.
//!
//! Secrets are stored as plain files under a protected directory
//! (`~/.config/pince/secrets/` by default). The supervisor reads them on
//! demand; sub-agents never see their values.

pub mod store;
pub mod resolver;
pub mod path_guard;

pub use store::{SecretStore, SecretValue};
pub use resolver::{resolve_secret_refs, has_secret_refs};
pub use path_guard::PathGuard;
