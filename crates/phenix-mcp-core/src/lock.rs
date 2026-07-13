use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct LockManager {
    locks: Mutex<HashMap<String, String>>,
}

impl LockManager {
    pub fn new() -> Self {
        Self { locks: Mutex::new(HashMap::new()) }
    }

    pub fn acquire(&self, resource: &str, owner: &str) -> Result<(), String> {
        let mut locks = self.locks.lock().map_err(|e| format!("Lock error: {}", e))?;
        if let Some(current_owner) = locks.get(resource) {
            if current_owner != owner {
                return Err(format!(
                    "Resource '{}' is locked by '{}'",
                    resource, current_owner
                ));
            }
        }
        locks.insert(resource.to_string(), owner.to_string());
        Ok(())
    }

    pub fn release(&self, resource: &str, owner: &str) -> Result<(), String> {
        let mut locks = self.locks.lock().map_err(|e| format!("Lock error: {}", e))?;
        match locks.get(resource) {
            Some(current) if current == owner => {
                locks.remove(resource);
                Ok(())
            }
            Some(current) => Err(format!(
                "Resource '{}' is locked by '{}', not '{}'",
                resource, current, owner
            )),
            None => Ok(()),
        }
    }

    pub fn is_locked(&self, resource: &str) -> bool {
        self.locks.lock().map(|l| l.contains_key(resource)).unwrap_or(false)
    }
}
