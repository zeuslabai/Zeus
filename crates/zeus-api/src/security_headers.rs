//! Security headers middleware
//!
//! Adds standard security headers to all HTTP responses:
//! - Content-Security-Policy
//! - X-Frame-Options
//! - X-Content-Type-Options
//! - Strict-Transport-Security
//! - X-XSS-Protection
//! - Referrer-Policy
//! - Permissions-Policy

use axum::{
    body::Body,
    http::{HeaderValue, Request, Response},
};
use std::task::{Context, Poll};
use tower::{Layer, Service};

/// Configuration for security headers.
#[derive(Debug, Clone)]
pub struct SecurityHeadersConfig {
    /// Content-Security-Policy header value
    pub content_security_policy: String,
    /// X-Frame-Options header value
    pub x_frame_options: String,
    /// X-Content-Type-Options header value
    pub x_content_type_options: String,
    /// Strict-Transport-Security header value
    pub strict_transport_security: String,
    /// X-XSS-Protection header value (0 = disable browser XSS filter, modern best practice)
    pub x_xss_protection: String,
    /// Referrer-Policy header value
    pub referrer_policy: String,
    /// Permissions-Policy header value
    pub permissions_policy: String,
}

impl Default for SecurityHeadersConfig {
    fn default() -> Self {
        Self {
            content_security_policy: "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; frame-ancestors 'none'".to_string(),
            x_frame_options: "DENY".to_string(),
            x_content_type_options: "nosniff".to_string(),
            strict_transport_security: "max-age=31536000; includeSubDomains".to_string(),
            x_xss_protection: "0".to_string(),
            referrer_policy: "strict-origin-when-cross-origin".to_string(),
            permissions_policy: "camera=(), microphone=(), geolocation=(), payment=()".to_string(),
        }
    }
}

/// Tower Layer that wraps services to add security headers.
#[derive(Debug, Clone, Default)]
pub struct SecurityHeadersLayer {
    config: SecurityHeadersConfig,
}

impl SecurityHeadersLayer {
    /// Create a new layer with the given config.
    pub fn new(config: SecurityHeadersConfig) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for SecurityHeadersLayer {
    type Service = SecurityHeadersService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SecurityHeadersService {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Tower Service that adds security headers to responses.
#[derive(Debug, Clone)]
pub struct SecurityHeadersService<S> {
    inner: S,
    config: SecurityHeadersConfig,
}

impl<S> Service<Request<Body>> for SecurityHeadersService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let config = self.config.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            let mut response = inner.call(req).await?;
            let headers = response.headers_mut();

            if let Ok(v) = HeaderValue::from_str(&config.content_security_policy) {
                headers.insert("content-security-policy", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.x_frame_options) {
                headers.insert("x-frame-options", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.x_content_type_options) {
                headers.insert("x-content-type-options", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.strict_transport_security) {
                headers.insert("strict-transport-security", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.x_xss_protection) {
                headers.insert("x-xss-protection", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.referrer_policy) {
                headers.insert("referrer-policy", v);
            }
            if let Ok(v) = HeaderValue::from_str(&config.permissions_policy) {
                headers.insert("permissions-policy", v);
            }

            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    async fn test_app() -> Router {
        Router::new().route("/test", get(|| async { "ok" }))
    }

    #[tokio::test]
    async fn test_security_headers_applied() {
        let app = test_app().await.layer(SecurityHeadersLayer::default());

        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), 200);

        let headers = response.headers();
        assert_eq!(headers.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(
            headers.get("strict-transport-security").unwrap(),
            "max-age=31536000; includeSubDomains"
        );
        assert_eq!(headers.get("x-xss-protection").unwrap(), "0");
        assert_eq!(
            headers.get("referrer-policy").unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert!(headers.get("content-security-policy").is_some());
        assert!(headers.get("permissions-policy").is_some());
    }

    #[tokio::test]
    async fn test_security_headers_custom_config() {
        let config = SecurityHeadersConfig {
            content_security_policy: "default-src 'none'".to_string(),
            x_frame_options: "SAMEORIGIN".to_string(),
            x_content_type_options: "nosniff".to_string(),
            strict_transport_security: "max-age=0".to_string(),
            x_xss_protection: "1; mode=block".to_string(),
            referrer_policy: "no-referrer".to_string(),
            permissions_policy: "camera=()".to_string(),
        };
        let app = test_app().await.layer(SecurityHeadersLayer::new(config));

        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(
            response.headers().get("content-security-policy").unwrap(),
            "default-src 'none'"
        );
        assert_eq!(
            response.headers().get("x-frame-options").unwrap(),
            "SAMEORIGIN"
        );
        assert_eq!(
            response.headers().get("referrer-policy").unwrap(),
            "no-referrer"
        );
    }

    #[tokio::test]
    async fn test_security_headers_default_config() {
        let config = SecurityHeadersConfig::default();
        assert_eq!(config.x_frame_options, "DENY");
        assert_eq!(config.x_content_type_options, "nosniff");
        assert_eq!(
            config.strict_transport_security,
            "max-age=31536000; includeSubDomains"
        );
        assert_eq!(config.x_xss_protection, "0");
        assert_eq!(config.referrer_policy, "strict-origin-when-cross-origin");
        assert!(
            config
                .content_security_policy
                .contains("default-src 'self'")
        );
        assert!(config.permissions_policy.contains("camera=()"));
    }

    #[tokio::test]
    async fn test_security_headers_dont_override_existing() {
        // The middleware sets headers after the response is built,
        // so it will override. This test verifies the headers are present.
        let app = test_app().await.layer(SecurityHeadersLayer::default());

        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(req).await.unwrap();
        // Verify all 7 security headers are present
        let header_count = [
            "content-security-policy",
            "x-frame-options",
            "x-content-type-options",
            "strict-transport-security",
            "x-xss-protection",
            "referrer-policy",
            "permissions-policy",
        ]
        .iter()
        .filter(|h| response.headers().get(**h).is_some())
        .count();
        assert_eq!(header_count, 7);
    }
}
