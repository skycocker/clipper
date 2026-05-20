//! Reconnect helpers: backoff math and the retry-loop wrapper that calls
//! [`crate::session::run_session`] across (re)connect attempts.

use std::time::Duration;

/// Capped exponential backoff for reconnect attempts.
///
/// attempt=1 → 1s, 2 → 2s, 3 → 4s, 4 → 8s, ≥5 → 16s. attempt=0 is treated
/// as "first try" (no prior failure) and returns 0; useful for callers that
/// always sleep through this fn for symmetry.
pub fn backoff(attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let shift = attempt.saturating_sub(1).min(4);
    Duration::from_secs(1u64 << shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        let secs: Vec<u64> = (0..8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(secs, vec![0, 1, 2, 4, 8, 16, 16, 16]);
    }

    #[test]
    fn backoff_is_total_for_large_attempt() {
        // Pathological input shouldn't panic or overflow.
        assert_eq!(backoff(u32::MAX), Duration::from_secs(16));
    }
}
