use std::collections::HashMap;
use std::sync::RwLock;

/// Bearer token state for authentication enforcement.
/// Tracks valid tokens and their remaining uses.
pub struct AuthState {
    tokens: RwLock<HashMap<String, Option<u64>>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
        }
    }

    /// Add a token. `max_uses` of `None` = unlimited.
    pub fn add_token(&self, token: &str, max_uses: Option<u64>) {
        self.tokens
            .write()
            .unwrap()
            .insert(token.to_string(), max_uses);
    }

    /// Check token validity and decrement use count. Returns `true` if valid.
    pub fn check_and_use(&self, token: &str) -> bool {
        let mut tokens = self.tokens.write().unwrap();
        match tokens.get_mut(token) {
            Some(Some(remaining)) if *remaining > 0 => {
                *remaining -= 1;
                if *remaining == 0 {
                    tokens.remove(token);
                }
                true
            }
            Some(Some(_)) => {
                tokens.remove(token);
                false
            }
            Some(None) => true, // unlimited
            None => false,
        }
    }

    /// Revoke a token.
    pub fn revoke(&self, token: &str) {
        self.tokens.write().unwrap().remove(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_valid_token() {
        let state = AuthState::new();
        state.add_token("tok-1", None);
        assert!(state.check_and_use("tok-1"));
    }

    #[test]
    fn should_reject_unknown_token() {
        let state = AuthState::new();
        assert!(!state.check_and_use("unknown"));
    }

    #[test]
    fn should_expire_after_n_uses() {
        let state = AuthState::new();
        state.add_token("tok-1", Some(2));
        assert!(state.check_and_use("tok-1")); // use 1
        assert!(state.check_and_use("tok-1")); // use 2
        assert!(!state.check_and_use("tok-1")); // expired
    }

    #[test]
    fn should_remove_revoked_token() {
        let state = AuthState::new();
        state.add_token("tok-1", None);
        state.revoke("tok-1");
        assert!(!state.check_and_use("tok-1"));
    }

    #[test]
    fn should_accept_unlimited_token_many_times() {
        let state = AuthState::new();
        state.add_token("unlimited", None);
        for _ in 0..100 {
            assert!(state.check_and_use("unlimited"));
        }
    }
}
