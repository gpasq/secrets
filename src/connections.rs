//! Saved connections: AWS credentials managed in the app instead of ~/.aws files.
//!
//! The non-secret parts (name, access key id, region, role) are saved with the app's
//! settings. The secret access key and session token go to the OS credential store
//! (macOS Keychain / Windows Credential Manager), never to a plain file.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "net.pasq.secrets";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Stable id, also the key for the secret in the credential store.
    pub id: String,
    pub name: String,
    pub access_key_id: String,
    pub region: String,
    /// Optional role to assume with the keys above.
    pub role_arn: String,
    pub external_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSecret {
    pub secret_access_key: String,
    pub session_token: String,
}

impl Connection {
    pub fn new_id() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        format!("conn-{nanos:x}")
    }

    /// `AKIA…WXYZ`: enough to recognize a key without showing all of it in lists.
    pub fn key_hint(&self) -> String {
        let k = self.access_key_id.trim();
        if k.len() <= 8 {
            k.to_string()
        } else {
            format!("{}…{}", &k[..4], &k[k.len() - 4..])
        }
    }

    pub fn validate(&self, secret: &ConnectionSecret, others: &[Connection]) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() {
            bail!("Give the connection a name");
        }
        if others
            .iter()
            .any(|o| o.id != self.id && o.name.trim().eq_ignore_ascii_case(name))
        {
            bail!("Another connection is already called {name:?}");
        }
        let key = self.access_key_id.trim();
        if key.is_empty() {
            bail!("Enter an access key ID");
        }
        if key.len() < 16 || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("That doesn't look like an access key ID (e.g. AKIA… or ASIA…)");
        }
        if secret.secret_access_key.trim().is_empty() {
            bail!("Enter the secret access key");
        }
        if key.starts_with("ASIA") && secret.session_token.trim().is_empty() {
            bail!("Temporary keys (ASIA…) also need a session token");
        }
        if self.region.trim().is_empty() {
            bail!("Choose a region");
        }
        let role = self.role_arn.trim();
        if !role.is_empty() && !(role.starts_with("arn:aws") && role.contains(":role/")) {
            bail!("Role ARN should look like arn:aws:iam::123456789012:role/Name");
        }
        Ok(())
    }
}

/// Where connection secrets are kept.
pub trait Vault {
    fn get(&self, id: &str) -> Result<Option<ConnectionSecret>>;
    fn set(&self, id: &str, secret: &ConnectionSecret) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// The OS credential store.
pub struct KeyringVault;

impl Vault for KeyringVault {
    fn get(&self, id: &str) -> Result<Option<ConnectionSecret>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, id).map_err(store_err)?;
        match entry.get_password() {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    fn set(&self, id: &str, secret: &ConnectionSecret) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, id).map_err(store_err)?;
        entry
            .set_password(&serde_json::to_string(secret)?)
            .map_err(store_err)
    }

    fn delete(&self, id: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, id).map_err(store_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
    }
}

fn store_err(e: keyring::Error) -> anyhow::Error {
    anyhow!("credential store: {e}")
}

#[cfg(test)]
pub struct MemVault(pub std::sync::Mutex<std::collections::HashMap<String, ConnectionSecret>>);

#[cfg(test)]
impl Vault for MemVault {
    fn get(&self, id: &str) -> Result<Option<ConnectionSecret>> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }
    fn set(&self, id: &str, secret: &ConnectionSecret) -> Result<()> {
        self.0.lock().unwrap().insert(id.into(), secret.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<()> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        Connection {
            id: "c1".into(),
            name: "Prod".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            region: "us-east-1".into(),
            ..Default::default()
        }
    }

    fn secret() -> ConnectionSecret {
        ConnectionSecret {
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: String::new(),
        }
    }

    #[test]
    fn validation() {
        assert!(conn().validate(&secret(), &[]).is_ok());

        let mut c = conn();
        c.name = " ".into();
        assert!(c.validate(&secret(), &[]).is_err());

        let other = Connection {
            id: "c2".into(),
            name: "prod".into(),
            ..conn()
        };
        assert!(
            conn().validate(&secret(), &[other]).is_err(),
            "duplicate name"
        );

        let mut c = conn();
        c.access_key_id = "short".into();
        assert!(c.validate(&secret(), &[]).is_err());

        assert!(conn().validate(&ConnectionSecret::default(), &[]).is_err());

        let mut c = conn();
        c.access_key_id = "ASIAIOSFODNN7EXAMPLE".into();
        assert!(
            c.validate(&secret(), &[]).is_err(),
            "temporary keys need a token"
        );

        let mut c = conn();
        c.role_arn = "not-an-arn".into();
        assert!(c.validate(&secret(), &[]).is_err());
        c.role_arn = "arn:aws:iam::123456789012:role/Admin".into();
        assert!(c.validate(&secret(), &[]).is_ok());

        let mut c = conn();
        c.region.clear();
        assert!(c.validate(&secret(), &[]).is_err());
    }

    #[test]
    fn key_hint_hides_the_middle() {
        assert_eq!(conn().key_hint(), "AKIA…MPLE");
    }

    #[test]
    fn vault_round_trip() {
        let v = MemVault(Default::default());
        assert_eq!(v.get("x").unwrap(), None);
        v.set("x", &secret()).unwrap();
        assert_eq!(v.get("x").unwrap(), Some(secret()));
        v.delete("x").unwrap();
        assert_eq!(v.get("x").unwrap(), None);
    }
}
