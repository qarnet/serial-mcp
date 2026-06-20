pub mod types;
pub use types::*;

pub const URI_PORTS: &str = "serial://ports";
pub const URI_CONNECTIONS: &str = "serial://connections";
pub const URI_CONNECTION_PREFIX: &str = "serial://connections/";
pub const URI_CONNECTION_TEMPLATE: &str = "serial://connections/{id}";
pub const URI_CONNECTION_RAW_TEMPLATE: &str = "serial://connections/{id}/raw";
pub const URI_CONNECTION_LOG_TEMPLATE: &str = "serial://connections/{id}/log";

#[derive(Debug, PartialEq, Eq)]
pub enum ResourceUriKind {
    Ports,
    ConnectionsList,
    ConnectionDetail(String),
    ConnectionDetailRaw(String),
    ConnectionLog(String),
    Unknown,
}

pub fn parse_resource_uri(uri: &str) -> ResourceUriKind {
    match uri {
        URI_PORTS => ResourceUriKind::Ports,
        URI_CONNECTIONS => ResourceUriKind::ConnectionsList,
        other => match other.strip_prefix(URI_CONNECTION_PREFIX) {
            Some(rest) if !rest.is_empty() => {
                if let Some(id) = rest.strip_suffix("/raw") {
                    if !id.is_empty() && !id.contains('/') {
                        return ResourceUriKind::ConnectionDetailRaw(id.to_string());
                    }
                }
                if let Some(id) = rest.strip_suffix("/log") {
                    if !id.is_empty() && !id.contains('/') {
                        return ResourceUriKind::ConnectionLog(id.to_string());
                    }
                }
                if rest.contains('/') {
                    ResourceUriKind::Unknown
                } else {
                    ResourceUriKind::ConnectionDetail(rest.to_string())
                }
            }
            _ => ResourceUriKind::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_uri_known_targets() {
        assert_eq!(parse_resource_uri("serial://ports"), ResourceUriKind::Ports);
        assert_eq!(
            parse_resource_uri("serial://connections"),
            ResourceUriKind::ConnectionsList
        );
        assert_eq!(
            parse_resource_uri("serial://connections/abc-123"),
            ResourceUriKind::ConnectionDetail("abc-123".into())
        );
        assert_eq!(
            parse_resource_uri("serial://connections/abc-123/raw"),
            ResourceUriKind::ConnectionDetailRaw("abc-123".into())
        );
    }

    #[test]
    fn resource_uri_unknown_targets() {
        assert_eq!(
            parse_resource_uri("serial://other"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("serial://connections/"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("serial://connections//raw"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("serial://connections/abc/extra"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("https://example.com"),
            ResourceUriKind::Unknown
        );
    }
}
