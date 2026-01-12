use std::time::{Duration, Instant};

pub struct SendThrottle {
    delay: Duration,
    last_send: Instant,
}

impl SendThrottle {
    pub fn new(delay: f64) -> Self {
        let delay = if delay <= 0.0 {
            Duration::from_secs(0)
        } else {
            Duration::from_secs_f64(delay)
        };
        let now = Instant::now();
        let last_send = now.checked_sub(delay).unwrap_or(now);
        Self { delay, last_send }
    }

    pub async fn wait(&mut self) -> Duration {
        if self.delay.is_zero() {
            return Duration::from_secs(0);
        }

        let elapsed = self.last_send.elapsed();
        let wait_time = if elapsed < self.delay {
            self.delay - elapsed
        } else {
            Duration::from_secs(0)
        };

        if !wait_time.is_zero() {
            tokio::time::sleep(wait_time).await;
        }

        wait_time
    }

    pub fn mark_sent(&mut self) {
        self.last_send = Instant::now();
    }
}
