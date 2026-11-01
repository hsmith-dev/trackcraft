//! `server.json` (next to the workspaces) and API tokens.
//!
//! Tokens are random (32 bytes from the OS), shown once when minted, and stored only as their
//! SHA-256 hash. A token authenticates as one workspace user id; the user must also be a member of
//! the workspace a request touches.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trackcraft_api::{Credential, Identity, IdentityProvider};

use crate::ServerError;

pub const CONFIG_FILE: &str = "server.json";
/// Largest config file read.
const MAX_CONFIG_BYTES: u64 = 16 * 1024 * 1024;
/// Prefix of every minted token (lets secret scanners recognise them).
pub const TOKEN_PREFIX: &str = "tct_";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Workspace served at the root paths; `None` = the only one, when there is exactly one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    /// Public origin used in `self` links (`https://track.example.com`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub tokens: Vec<TokenRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenRecord {
    /// Short public id (for `token list` / `token revoke`).
    pub id: String,
    /// The workspace user id this token acts as.
    pub user: String,
    /// Accepted as the Basic-auth user name besides the user id (Jira clients send an email).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Hex SHA-256 of the token.
    pub sha256: String,
    pub created: String,
}

pub fn config_path(data: &Path) -> PathBuf {
    data.join(CONFIG_FILE)
}

impl Config {
    /// Reads `server.json`; a missing file is an empty config.
    pub fn load(data: &Path) -> Result<Config, ServerError> {
        let path = config_path(data);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(ServerError::io(&path, e)),
        };
        if meta.len() > MAX_CONFIG_BYTES {
            return Err(ServerError::Config(format!("{} is larger than 16 MiB", path.display())));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| ServerError::io(&path, e))?;
        serde_json::from_str(&text).map_err(|e| ServerError::Config(format!("{}: {e}", path.display())))
    }

    /// Writes `server.json` atomically (temp file + rename), readable only by the owner on Unix.
    pub fn save(&self, data: &Path) -> Result<(), ServerError> {
        let path = config_path(data);
        let text = serde_json::to_string_pretty(self).map_err(|e| ServerError::Config(e.to_string()))?;
        let tmp = data.join(format!(".{CONFIG_FILE}.saving"));
        std::fs::write(&tmp, text).map_err(|e| ServerError::io(&tmp, e))?;
        restrict(&tmp);
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ServerError::io(&path, e)
        })
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

pub fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn random_hex(bytes: usize) -> Result<String, ServerError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| ServerError::Config(format!("no OS randomness: {e}")))?;
    Ok(hex::encode(buf))
}

/// Mints a token for `user`, records its hash in `server.json`, and returns `(record, token)`.
/// The token itself is not stored anywhere: show it once.
pub fn mint_token(data: &Path, user: &str, email: Option<&str>, label: &str) -> Result<(TokenRecord, String), ServerError> {
    let user = user.trim();
    if user.is_empty() || user.len() > 128 || user.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ServerError::Config("--user is a workspace user id (1–128 characters, no spaces)".into()));
    }
    let mut cfg = Config::load(data)?;
    let token = format!("{TOKEN_PREFIX}{}", random_hex(32)?);
    let rec = TokenRecord {
        id: random_hex(4)?,
        user: user.to_string(),
        email: email.map(str::trim).filter(|e| !e.is_empty()).map(str::to_string),
        label: label.chars().take(200).collect(),
        sha256: hash(&token),
        created: trackcraft_engine::system_now().to_rfc3339(),
    };
    cfg.tokens.push(rec.clone());
    cfg.save(data)?;
    Ok((rec, token))
}

/// Removes a token by id; `false` if there was none.
pub fn revoke_token(data: &Path, id: &str) -> Result<bool, ServerError> {
    let mut cfg = Config::load(data)?;
    let before = cfg.tokens.len();
    cfg.tokens.retain(|t| t.id != id);
    let gone = cfg.tokens.len() != before;
    if gone {
        cfg.save(data)?;
    }
    Ok(gone)
}

/// The standalone [`IdentityProvider`]: the token hashes in `server.json`. The file is re-read
/// when its modification time changes (one `stat` per request), so tokens minted or revoked
/// while the server runs take effect without a restart.
pub struct TokenIdentity {
    data: Option<PathBuf>,
    state: RwLock<(Vec<TokenRecord>, Option<SystemTime>)>,
}

impl TokenIdentity {
    pub fn new(tokens: Vec<TokenRecord>) -> Self {
        TokenIdentity { data: None, state: RwLock::new((tokens, None)) }
    }

    pub fn from_dir(data: &Path) -> Result<Self, ServerError> {
        let cfg = Config::load(data)?;
        let mtime = std::fs::metadata(config_path(data)).and_then(|m| m.modified()).ok();
        Ok(TokenIdentity { data: Some(data.to_path_buf()), state: RwLock::new((cfg.tokens, mtime)) })
    }

    fn lookup(&self, credential: &Credential<'_>) -> Option<Identity> {
        let st = self.state.read().unwrap_or_else(|p| p.into_inner());
        let (secret, name) = match credential {
            Credential::Bearer(t) => (*t, None),
            Credential::Basic { user, secret } => (*secret, Some(*user)),
        };
        if !secret.starts_with(TOKEN_PREFIX) || secret.len() > 256 {
            return None;
        }
        let h = hash(secret);
        let rec = st.0.iter().find(|t| t.sha256 == h)?;
        if let Some(n) = name {
            let matches = n == rec.user || rec.email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(n));
            if !matches {
                return None;
            }
        }
        Some(Identity { user: rec.user.clone() })
    }

    fn reload_if_changed(&self) -> bool {
        let Some(data) = &self.data else { return false };
        let mtime = std::fs::metadata(config_path(data)).and_then(|m| m.modified()).ok();
        if self.state.read().unwrap_or_else(|p| p.into_inner()).1 == mtime {
            return false;
        }
        let mut st = self.state.write().unwrap_or_else(|p| p.into_inner());
        match Config::load(data) {
            Ok(cfg) => {
                *st = (cfg.tokens, mtime);
                true
            }
            Err(_) => false,
        }
    }
}

impl IdentityProvider for TokenIdentity {
    fn authenticate(&self, credential: &Credential<'_>) -> Option<Identity> {
        self.reload_if_changed();
        self.lookup(credential)
    }
}
