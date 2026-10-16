//! Ограничение частоты запросов по IP (token bucket в памяти процесса).
//!
//! ```
//! use rustyapi::prelude::*;
//! use rustyapi::ratelimit::RateLimit;
//!
//! let app = App::new().rate_limit(RateLimit::per_minute(60).burst(10));
//! ```
//!
//! Превышение — `429 {"detail": "Too Many Requests"}` с заголовком `Retry-After`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::HTTPException;

/// Настройка лимита.
#[derive(Debug, Clone)]
pub struct RateLimit {
    requests: u32,
    period: Duration,
    burst: Option<u32>,
    trust_forwarded: bool,
}

impl RateLimit {
    /// `requests` запросов за `period` с одного IP.
    ///
    /// # Panics
    /// Если `requests == 0` или `period` нулевой.
    pub fn new(requests: u32, period: Duration) -> Self {
        assert!(requests > 0 && !period.is_zero(), "лимит должен быть > 0");
        Self {
            requests,
            period,
            burst: None,
            trust_forwarded: false,
        }
    }

    /// `requests` запросов в секунду.
    pub fn per_second(requests: u32) -> Self {
        Self::new(requests, Duration::from_secs(1))
    }

    /// `requests` запросов в минуту.
    pub fn per_minute(requests: u32) -> Self {
        Self::new(requests, Duration::from_secs(60))
    }

    /// Сколько запросов можно сделать подряд (по умолчанию — весь лимит периода).
    pub fn burst(mut self, burst: u32) -> Self {
        self.burst = Some(burst.max(1));
        self
    }

    /// Брать IP клиента из `X-Forwarded-For` (только за своим reverse proxy: иначе клиент
    /// подделает заголовок).
    pub fn trust_forwarded_for(mut self) -> Self {
        self.trust_forwarded = true;
        self
    }

    fn capacity(&self) -> f64 {
        f64::from(self.burst.unwrap_or(self.requests))
    }

    /// Запросов в секунду.
    fn rate(&self) -> f64 {
        f64::from(self.requests) / self.period.as_secs_f64()
    }
}

/// Состояние лимитера: корзина токенов на IP.
#[derive(Clone)]
pub(crate) struct Limiter {
    config: RateLimit,
    buckets: Arc<Mutex<HashMap<IpAddr, (f64, Instant)>>>,
}

impl Limiter {
    pub(crate) fn new(config: RateLimit) -> Self {
        Self {
            config,
            buckets: Arc::default(),
        }
    }

    /// `Ok(осталось)` или `Err(через сколько повторить)`.
    fn check(&self, ip: IpAddr, now: Instant) -> Result<u32, Duration> {
        let capacity = self.config.capacity();
        let rate = self.config.rate();
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Корзины, которые давно полны, не нужны.
        if buckets.len() > 10_000 {
            let full_after = Duration::from_secs_f64(capacity / rate);
            buckets.retain(|_, (_, at)| now.duration_since(*at) < full_after);
        }
        let (tokens, at) = buckets.entry(ip).or_insert((capacity, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * rate).min(capacity);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(*tokens as u32)
        } else {
            Err(Duration::from_secs_f64((1.0 - *tokens) / rate))
        }
    }

    fn client_ip(&self, request: &Request) -> Option<IpAddr> {
        if self.config.trust_forwarded {
            let forwarded = request
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .and_then(|ip| ip.trim().parse().ok());
            if forwarded.is_some() {
                return forwarded;
            }
        }
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip())
    }
}

pub(crate) async fn layer(limiter: Limiter, request: Request, next: Next) -> Response {
    let ip = limiter
        .client_ip(&request)
        .unwrap_or(IpAddr::from([0, 0, 0, 0]));
    match limiter.check(ip, Instant::now()) {
        Ok(remaining) => {
            let mut response = next.run(request).await;
            let headers = response.headers_mut();
            headers.insert(
                "x-ratelimit-limit",
                HeaderValue::from(limiter.config.requests),
            );
            headers.insert("x-ratelimit-remaining", HeaderValue::from(remaining));
            response
        }
        Err(retry) => {
            let seconds = retry.as_secs() + u64::from(retry.subsec_nanos() > 0);
            HTTPException::from_status(429)
                .with_header("retry-after", &seconds.max(1).to_string())
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket() {
        let limiter = Limiter::new(RateLimit::per_second(2));
        let ip = IpAddr::from([10, 0, 0, 1]);
        let t0 = Instant::now();
        assert_eq!(limiter.check(ip, t0), Ok(1));
        assert_eq!(limiter.check(ip, t0), Ok(0));
        let retry = limiter.check(ip, t0).unwrap_err();
        assert_eq!(retry, Duration::from_millis(500));
        // Другой IP — своя корзина.
        assert!(limiter.check(IpAddr::from([10, 0, 0, 2]), t0).is_ok());
        // Через полсекунды появился токен.
        assert!(limiter.check(ip, t0 + Duration::from_millis(500)).is_ok());
        assert!(limiter.check(ip, t0 + Duration::from_millis(500)).is_err());

        let burst = Limiter::new(RateLimit::per_minute(60).burst(1));
        assert!(burst.check(ip, t0).is_ok());
        assert!(burst.check(ip, t0).is_err());
        assert!(burst.check(ip, t0 + Duration::from_secs(1)).is_ok());
    }
}
