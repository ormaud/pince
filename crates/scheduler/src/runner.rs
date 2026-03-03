//! Scheduler loop: evaluates cron expressions and emits due jobs.

use std::str::FromStr;

use chrono::Utc;
use cron::Schedule;
use tokio::{sync::mpsc, time};
use tracing::{error, info, warn};

use crate::config::CronJob;

/// Sent on the channel when a scheduled job is due to run.
#[derive(Debug, Clone)]
pub struct DueJob {
    /// Name of the cron job.
    pub name: String,
    /// Agent profile to spawn.
    pub agent: String,
    /// Prompt to send to the agent.
    pub prompt: String,
    /// Max runtime before kill (seconds).
    pub timeout_secs: u64,
}

/// Internal: a parsed job with its last-fire timestamp.
struct ParsedJob {
    config: CronJob,
    schedule: Schedule,
    /// The wall-clock minute at which we last fired this job (used to deduplicate).
    last_fired_minute: Option<i64>,
}

/// Evaluates cron schedules and sends `DueJob` values on a channel.
///
/// The scheduler checks every 60 seconds. Missed ticks (e.g., due to supervisor
/// downtime) are silently skipped — no catch-up.
pub struct Scheduler {
    jobs: Vec<ParsedJob>,
    tx: mpsc::Sender<DueJob>,
}

impl Scheduler {
    /// Build a scheduler from config. Jobs with invalid cron expressions are
    /// logged and skipped.
    pub fn new(jobs: Vec<CronJob>, tx: mpsc::Sender<DueJob>) -> Self {
        let mut parsed = Vec::with_capacity(jobs.len());
        for job in jobs {
            if !job.enabled {
                info!(job = %job.name, "cron job disabled, skipping");
                continue;
            }
            match Schedule::from_str(&job.schedule) {
                Ok(schedule) => {
                    info!(job = %job.name, schedule = %job.schedule, "cron job registered");
                    parsed.push(ParsedJob {
                        config: job,
                        schedule,
                        last_fired_minute: None,
                    });
                }
                Err(e) => {
                    error!(job = %job.name, schedule = %job.schedule, "invalid cron expression: {e}");
                }
            }
        }
        Self { jobs: parsed, tx }
    }

    /// Run the scheduler loop forever. Ticks every 60 seconds.
    pub async fn run(&mut self) {
        let mut interval = time::interval(std::time::Duration::from_secs(60));
        // The first tick fires immediately; skip it so we don't fire all jobs
        // on startup.
        interval.tick().await;

        loop {
            interval.tick().await;
            self.check_and_fire().await;
        }
    }

    /// Check all jobs against the current time, fire any that are due.
    pub(crate) async fn check_and_fire(&mut self) {
        let now = Utc::now();
        // Truncate to the current minute for deduplication.
        let current_minute = now.timestamp() / 60;

        for job in &mut self.jobs {
            // Skip if already fired this minute.
            if job.last_fired_minute == Some(current_minute) {
                continue;
            }

            // Check if the next occurrence is within the last 60 seconds.
            let next = job.schedule.upcoming(Utc).next();
            let Some(next_time) = next else { continue };

            // If the next fire time is in the past (≤ now + 1s grace) it means
            // the schedule is due.  We also accept it if it's within the next
            // second (clocks can disagree slightly).
            let delta = (next_time - now).num_seconds();
            if delta <= 1 {
                info!(job = %job.config.name, "cron job due, triggering");
                job.last_fired_minute = Some(current_minute);

                let due = DueJob {
                    name: job.config.name.clone(),
                    agent: job.config.agent.clone(),
                    prompt: job.config.prompt.clone(),
                    timeout_secs: job.config.timeout_secs.unwrap_or(300),
                };
                if self.tx.send(due).await.is_err() {
                    warn!(job = %job.config.name, "scheduler channel closed");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_job(name: &str, schedule: &str, enabled: bool) -> CronJob {
        CronJob {
            name: name.to_string(),
            schedule: schedule.to_string(),
            agent: "default".to_string(),
            prompt: "test prompt".to_string(),
            timeout_secs: Some(60),
            enabled,
        }
    }

    #[test]
    fn parse_valid_expression() {
        let (tx, _rx) = mpsc::channel(4);
        let jobs = vec![make_job("test", "0 * * * * * *", true)];
        let scheduler = Scheduler::new(jobs, tx);
        assert_eq!(scheduler.jobs.len(), 1);
    }

    #[test]
    fn skip_invalid_expression() {
        let (tx, _rx) = mpsc::channel(4);
        let jobs = vec![make_job("bad", "not-a-cron", true)];
        let scheduler = Scheduler::new(jobs, tx);
        assert_eq!(scheduler.jobs.len(), 0);
    }

    #[test]
    fn skip_disabled_job() {
        let (tx, _rx) = mpsc::channel(4);
        let jobs = vec![make_job("disabled", "0 * * * * * *", false)];
        let scheduler = Scheduler::new(jobs, tx);
        assert_eq!(scheduler.jobs.len(), 0);
    }

    #[tokio::test]
    async fn no_double_fire_in_same_minute() {
        let (tx, mut rx) = mpsc::channel(4);
        // "every second" — fires at 0 seconds every minute
        let jobs = vec![make_job("fast", "* * * * * * *", true)];
        let mut scheduler = Scheduler::new(jobs, tx);

        // Set last_fired to a past minute so first check fires.
        // We check twice in the same minute and expect only one event.
        let current_minute = Utc::now().timestamp() / 60;
        scheduler.jobs[0].last_fired_minute = Some(current_minute - 1);

        scheduler.check_and_fire().await;
        let first = rx.try_recv();
        assert!(first.is_ok(), "expected first fire");

        // Second check in the same minute should not fire again.
        scheduler.check_and_fire().await;
        let second = rx.try_recv();
        assert!(second.is_err(), "should not double-fire");
    }
}
