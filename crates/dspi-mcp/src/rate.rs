//! The rate limiter: a token bucket per client and class, on the caller's
//! clock (`Server::handle`'s `now_ms`), so tests drive it with numbers.

/// A bucket's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    /// The most calls in a burst.
    pub burst: u32,
    /// Calls regained per second.
    pub per_s: f64,
}

/// The default for read tools: 20 a second, bursts of 40.
pub const READ_LIMIT: RateLimit = RateLimit {
    burst: 40,
    per_s: 20.0,
};

/// The default for write tools: 4 a second, bursts of 10. MCP2a may tighten
/// it per tool.
pub const WRITE_LIMIT: RateLimit = RateLimit {
    burst: 10,
    per_s: 4.0,
};

/// One bucket.
#[derive(Debug, Clone)]
pub struct Bucket {
    limit: RateLimit,
    tokens: f64,
    at_ms: Option<u64>,
}

impl Bucket {
    /// A full bucket.
    pub fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            tokens: f64::from(limit.burst),
            at_ms: None,
        }
    }

    /// Take one call at `now_ms`: `Ok`, or how many ms until one is free. A
    /// clock that goes back refills nothing.
    pub fn take(&mut self, now_ms: u64) -> Result<(), u64> {
        let burst = f64::from(self.limit.burst);
        if let Some(at) = self.at_ms {
            let dt = now_ms.saturating_sub(at) as f64 / 1000.0;
            self.tokens = (self.tokens + dt * self.limit.per_s).min(burst);
        }
        self.at_ms = Some(self.at_ms.map_or(now_ms, |at| at.max(now_ms)));
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Ok(());
        }
        if self.limit.per_s <= 0.0 {
            return Err(u64::MAX);
        }
        let wait = ((1.0 - self.tokens) / self.limit.per_s * 1000.0).ceil();
        Err(if wait.is_finite() && wait >= 1.0 {
            wait as u64
        } else {
            1
        })
    }
}
