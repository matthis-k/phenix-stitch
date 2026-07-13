use crate::types::MutationLevel;

pub struct SafetyPolicy {
    pub allow_destructive: bool,
    pub allow_network: bool,
    pub allow_commit: bool,
    pub max_timeout_seconds: u64,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            allow_destructive: false,
            allow_network: false,
            allow_commit: false,
            max_timeout_seconds: 300,
        }
    }
}

impl SafetyPolicy {
    pub fn check_mutation(&self, level: &MutationLevel) -> Result<(), String> {
        match level {
            MutationLevel::Destructive if !self.allow_destructive => {
                Err("Destructive operations are not allowed by policy".to_string())
            }
            MutationLevel::Network if !self.allow_network => {
                Err("Network operations are not allowed by policy".to_string())
            }
            MutationLevel::CreatesCommit if !self.allow_commit => {
                Err("Commit operations are not allowed by policy".to_string())
            }
            _ => Ok(()),
        }
    }

    pub fn validate_timeout(&self, timeout_seconds: u64) -> Result<u64, String> {
        if timeout_seconds > self.max_timeout_seconds {
            return Err(format!(
                "Timeout {}s exceeds maximum allowed {}s",
                timeout_seconds, self.max_timeout_seconds
            ));
        }
        Ok(timeout_seconds)
    }
}
