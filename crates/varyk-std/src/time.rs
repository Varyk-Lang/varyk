//! Time, for generated programs: `time::sleep`.

/// Waits `ms` milliseconds without holding up other tasks.
pub async fn sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    #[test]
    fn sleep_waits_at_least_as_long_as_asked() {
        let start = Instant::now();
        crate::run(async {
            super::sleep(20).await;
        });
        assert!(start.elapsed() >= Duration::from_millis(20));
    }
}
