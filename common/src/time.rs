use std::time::{Duration, Instant};

pub fn throttle_delay(delay_secs: f64) -> Duration {
    let millis = (delay_secs * 1000.0).clamp(0.0, 300000.0) as u64;
    Duration::from_millis(millis)
}

pub fn time_until(target: Instant) -> Duration {
    let now = Instant::now();
    if target > now {
        target.duration_since(now)
    } else {
        Duration::ZERO
    }
}

pub fn clamp_delay(min_secs: f64, value_secs: f64, max_secs: f64) -> f64 {
    value_secs.max(min_secs).min(max_secs)
}
