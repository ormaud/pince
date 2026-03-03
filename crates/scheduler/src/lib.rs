//! Cron scheduler for the pince supervisor.
//!
//! Parses cron job definitions from config and fires a `DueJob` on a channel
//! whenever a job's schedule matches the current time.

pub mod config;
pub mod runner;

pub use config::CronJob;
pub use runner::{DueJob, Scheduler};
