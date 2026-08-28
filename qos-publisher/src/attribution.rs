//! Whose fault was this query?
//!
//! The oracle wants `user_attributed_error_rate`, which the gateway does not publish. The split
//! below is read off `src/errors.rs` in the gateway fork. Whether Edge & Node draw the line in the
//! same place is **unverified**: if our figure differs from theirs on comparable traffic, this is
//! the first place to look.

/// Who a failed query is attributed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blame {
    /// The caller sent something we could not serve.
    User,
    /// The network could not serve a valid request.
    Network,
    /// The gateway itself broke.
    Gateway,
}

/// Classifies a gateway `result` string. `"success"` is not an error and returns `None`.
pub fn classify(result: &str) -> Option<Blame> {
    if result == "success" {
        return None;
    }
    Some(
        if result.starts_with("auth error:")
            || result.starts_with("bad query:")
            || result.starts_with("subgraph not found:")
        {
            Blame::User
        } else if result.starts_with("no indexers found") || result.starts_with("bad indexers:") {
            Blame::Network
        } else if result.starts_with("internal error:") {
            Blame::Gateway
        } else {
            // An unrecognised error is the gateway's problem until proven otherwise. Defaulting to
            // User would flatter our own success rate with every error string we have not seen yet,
            // which is the one direction a self-published metric must not be wrong in.
            Blame::Gateway
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_not_an_error() {
        assert_eq!(classify("success"), None);
    }

    #[test]
    fn classifies_each_documented_prefix() {
        assert_eq!(classify("auth error: missing API key"), Some(Blame::User));
        assert_eq!(classify("bad query: syntax"), Some(Blame::User));
        assert_eq!(classify("subgraph not found: Qm..."), Some(Blame::User));
        assert_eq!(classify("no indexers found"), Some(Blame::Network));
        assert_eq!(classify("bad indexers: all failed"), Some(Blame::Network));
        assert_eq!(classify("internal error: panic"), Some(Blame::Gateway));
    }

    /// The direction of this default is deliberate and load-bearing.
    #[test]
    fn an_unknown_error_blames_the_gateway_not_the_user() {
        assert_eq!(
            classify("something we have never seen"),
            Some(Blame::Gateway)
        );
        assert_ne!(classify("something we have never seen"), Some(Blame::User));
    }

    /// "successful" must not be mistaken for "success".
    #[test]
    fn only_the_exact_success_string_counts_as_success() {
        assert!(classify("successfully failed").is_some());
    }
}
