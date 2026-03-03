pub mod pince {
    pub mod agent {
        include!("pince.agent.rs");
    }
    pub mod frontend {
        include!("pince.frontend.rs");
    }
}

// Backward-compatible alias so existing code using `generated::pince_agent` still works.
pub mod pince_agent {
    pub use super::pince::agent::*;
}

// Convenience alias for frontend types.
pub mod pince_frontend {
    pub use super::pince::frontend::*;
}
