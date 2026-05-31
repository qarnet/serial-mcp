use glob::Pattern;
use tracing::warn;

/// Security manager that handles port allowlist checks.
///
/// If the allowlist is empty, all ports are allowed.
#[derive(Clone)]
pub struct SecurityManager {
    allowlist: Vec<Pattern>,
}

impl SecurityManager {
    /// Create a security manager from explicit glob pattern strings.
    /// Invalid patterns are silently ignored (logged as warnings).
    pub fn from_patterns<I: IntoIterator>(patterns: I) -> Self
    where
        I::Item: AsRef<str>,
    {
        let allowlist = patterns
            .into_iter()
            .filter_map(|s| match Pattern::new(s.as_ref()) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!("Invalid allowlist pattern '{}': {e}", s.as_ref());
                    None
                }
            })
            .collect();
        Self { allowlist }
    }

    /// Check if a port matches the allowlist. Empty allowlist = allow all.
    pub fn is_port_allowed(&self, port: &str) -> bool {
        if self.allowlist.is_empty() {
            return true;
        }
        self.allowlist.iter().any(|pattern| pattern.matches(port))
    }

    /// Human-readable summary of allowlist patterns for error messages.
    pub fn allowlist_summary(&self) -> String {
        if self.allowlist.is_empty() {
            "(all ports allowed)".to_string()
        } else {
            self.allowlist
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_str(env_val: &str) -> SecurityManager {
        let patterns = env_val
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter_map(|s| match Pattern::new(s) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!("Invalid allowlist pattern '{s}': {e}");
                    None
                }
            })
            .collect();
        SecurityManager {
            allowlist: patterns,
        }
    }

    #[test]
    fn empty_allowlist_allows_all() {
        let mgr = from_str("");
        assert!(mgr.is_port_allowed("/dev/ttyUSB0"));
        assert!(mgr.is_port_allowed("COM3"));
        assert_eq!(mgr.allowlist_summary(), "(all ports allowed)");
    }

    #[test]
    fn pattern_in_allowlist_matches() {
        let mgr = from_str("/dev/ttyUSB*");
        assert!(mgr.is_port_allowed("/dev/ttyUSB0"));
        assert!(mgr.is_port_allowed("/dev/ttyUSB99"));
        assert!(!mgr.is_port_allowed("/dev/ttyS0"));
        assert!(!mgr.is_port_allowed("COM3"));
    }

    #[test]
    fn allowlist_summary_shows_patterns() {
        let mgr = from_str("/dev/ttyUSB*,COM*");
        let summary = mgr.allowlist_summary();
        assert!(summary.contains("/dev/ttyUSB*"));
        assert!(summary.contains("COM*"));
    }

    #[test]
    fn ignores_invalid_patterns() {
        let mgr = from_str("/dev/ttyUSB*,[invalid");
        assert_eq!(mgr.allowlist.len(), 1);
        assert!(mgr.is_port_allowed("/dev/ttyUSB0"));
    }

    #[test]
    fn all_invalid_patterns_yields_empty() {
        let mgr = from_str("[invalid,[also_broken");
        assert!(mgr.allowlist.is_empty());
        assert!(mgr.is_port_allowed("/dev/ttyUSB0"));
    }

    #[test]
    fn multiple_valid_and_invalid() {
        let mgr = from_str("COM3,[bad,/dev/tty[a-z]*");
        assert_eq!(mgr.allowlist.len(), 2);
        assert!(mgr.is_port_allowed("COM3"));
        assert!(mgr.is_port_allowed("/dev/ttyb"));
        assert!(!mgr.is_port_allowed("/dev/tty0"));
    }
}
