//! A count of requests for a key in the current minute or hour, kept in Postgres so every server instance shares it.

use crate::service::ServiceError;
use crate::state::SharedState;

/// How long a count lasts. A window is a whole minute or a whole hour of the clock.
#[derive(Debug, Clone, Copy)]
pub enum Window {
    Minute,
    Hour,
}

impl Window {
    fn unit(self) -> &'static str {
        match self {
            Window::Minute => "minute",
            Window::Hour => "hour",
        }
    }
}

/// Counts one request against `key` in the current window. The request that passes `limit` is refused with `rate_limited`.
pub fn hit(state: &SharedState, key: String, limit: i32, window: Window) -> Result<(), ServiceError> {
    let unit = window.unit();
    let count: i32 = state
        .repository
        .with_client(move |c| {
            let n: i32 = c
                .query_one(
                    &format!(
                        "INSERT INTO rate_limits (key, window_start, count) VALUES ($1, date_trunc('{unit}', NOW()), 1) \
                         ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1 RETURNING count"
                    ),
                    &[&key],
                )?
                .get(0);
            // A key's first request of a window also clears what is over a day old.
            if n == 1 {
                c.execute("DELETE FROM rate_limits WHERE window_start < NOW() - INTERVAL '1 day'", &[])?;
            }
            Ok(n)
        })
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    if count > limit {
        return Err(ServiceError::RateLimited("Too many requests. Wait a while and try again.".to_string()));
    }
    Ok(())
}
