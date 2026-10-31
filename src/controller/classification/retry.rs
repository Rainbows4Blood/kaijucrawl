//! Per-book exponential delays and a shared run cooldown; no attempt budget.
use crate::decisions::Retry;
use std::time::Duration;
use tokio::time::Instant;

pub(super) fn delay(failures: u32, server: Option<Duration>, jitter_ms: u64) -> Duration {
    let exponential = Duration::from_secs(2_u64.pow(failures.clamp(1, 9)).min(300));
    exponential
        .max(server.unwrap_or_default())
        .saturating_add(Duration::from_millis(jitter_ms.min(1000)))
}

pub(super) struct Cooldown {
    duration: Duration,
    since: Instant,
    pub cause: Option<Retry>,
}
impl Default for Cooldown {
    fn default() -> Self {
        Self {
            duration: Duration::ZERO,
            since: Instant::now(),
            cause: None,
        }
    }
}
impl Cooldown {
    pub fn remaining(&self) -> Duration {
        self.duration.saturating_sub(self.since.elapsed())
    }
    pub fn extend(&mut self, duration: Duration, cause: Retry) {
        // A sibling's shorter delay or successful request cannot shorten an existing wait.
        if duration >= self.remaining() {
            self.duration = duration;
            self.since = Instant::now();
            self.cause = Some(cause);
        }
    }
    pub fn seconds(&self) -> u64 {
        let remaining = self.remaining();
        remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delays_increase_and_cap_without_exhausting_retries_or_shortening_server_hints() {
        for (attempt, secs) in [
            (1, 2),
            (2, 4),
            (3, 8),
            (8, 256),
            (9, 300),
            (1000, 300),
            (u32::MAX, 300),
        ] {
            assert_eq!(delay(attempt, None, 0), Duration::from_secs(secs));
        }
        assert_eq!(delay(1, None, 125), Duration::from_millis(2125));
        assert_eq!(
            delay(1, Some(Duration::from_secs(7200)), 500),
            Duration::from_millis(7_200_500)
        );
        assert_eq!(
            delay(9, Some(Duration::from_secs(1)), 0),
            Duration::from_secs(300)
        );
        let mut cooldown = Cooldown::default();
        let capacity = crate::decisions::Error::Capacity {
            retry_after_secs: Some(7200),
        }
        .retry_hint()
        .unwrap();
        let connection = crate::decisions::Error::Transport.retry_hint().unwrap();
        cooldown.extend(Duration::from_secs(7200), capacity);
        cooldown.extend(Duration::from_secs(2), connection);
        assert!(cooldown.remaining() > Duration::from_secs(7199));
        assert_eq!(cooldown.cause, Some(capacity));
        // Huge, valid server seconds never overflow Instant or become an early retry.
        cooldown.extend(Duration::from_secs(u64::MAX), connection);
        assert!(cooldown.seconds() > u64::MAX - 2);
        assert_eq!(cooldown.cause, Some(connection));
    }
}
