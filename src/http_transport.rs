//! HTTP routing that preserves the product-owned MCP lifecycle policy.

use axum::{extract::Request, response::IntoResponse, routing::any, Router};
use rmcp::{
    transport::streamable_http_server::{session::SessionManager, StreamableHttpService},
    ServerHandler,
};

use crate::mcp_protocol::{policy_for_str, ProtocolLifecycle};

/// Mount an rmcp service with exact-version lifecycle selection.
///
/// rmcp 3.5 treats `initialize` as legacy regardless of its protocol version.
/// Force stateless transport for a declared discovery-only product version so
/// the handler can reject `initialize` before any session is allocated. This
/// does not parse or rewrite messages; rmcp still validates headers and bodies.
/// Legacy, absent, and unknown version headers retain rmcp's normal routing.
pub fn router<S, M>(mount_path: &str, service: StreamableHttpService<S, M>) -> Router
where
    S: ServerHandler + Send + 'static,
    M: SessionManager + 'static,
{
    Router::new().nest_service(
        mount_path,
        any(move |request: Request| {
            let mut service = service.clone();
            async move {
                let policy = request
                    .headers()
                    .get("MCP-Protocol-Version")
                    .and_then(|value| value.to_str().ok())
                    .and_then(policy_for_str);
                if policy
                    .is_some_and(|policy| policy.lifecycle == ProtocolLifecycle::DiscoverStateless)
                {
                    service.config.legacy_session_mode = false;
                }
                service.handle(request).await.into_response()
            }
        }),
    )
}
