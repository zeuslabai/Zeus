//! Multi-identity auth store (#432): token → principal → scope.
//!
//! SQLite-backed. **Hashes only** — raw tokens/invite codes are shown exactly
//! once at mint time; only their SHA-256 digests are persisted.
//!
//! Schema managed via the shared `db::run_migrations` helper
//! (`PRAGMA user_version`).

use crate::db::run_migrations;
use ring::digest::{SHA256, digest};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Role ladder: readonly < member < admin < root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Readonly,
    Member,
    Admin,
    Root,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Readonly => "readonly",
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Root => "root",
        }
    }

    pub fn parse_role(s: &str) -> Option<Self> {
        match s {
            "readonly" => Some(Role::Readonly),
            "member" => Some(Role::Member),
            "admin" => Some(Role::Admin),
            "root" => Some(Role::Root),
            _ => None,
        }
    }
}

/// A resolved identity attached to a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub display_name: String,
    pub role: Role,
}

/// Public view of a principal — never includes token material.
#[derive(Debug, Clone, Serialize)]
pub struct MemberView {
    pub id: String,
    pub display_name: String,
    pub role: Role,
    pub created_at: u64,
    pub invited_by: Option<String>,
    pub disabled: bool,
    pub last_used_at: Option<u64>,
}

const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS principals (
        id TEXT PRIMARY KEY,
        display_name TEXT NOT NULL,
        role TEXT NOT NULL CHECK(role IN ('readonly','member','admin','root')),
        created_at INTEGER NOT NULL,
        invited_by TEXT,
        disabled INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS tokens (
        token_hash TEXT PRIMARY KEY,
        principal_id TEXT NOT NULL REFERENCES principals(id),
        label TEXT,
        created_at INTEGER NOT NULL,
        expires_at INTEGER,
        last_used_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_tokens_principal ON tokens(principal_id);

    CREATE TABLE IF NOT EXISTS invites (
        code_hash TEXT PRIMARY KEY,
        role TEXT NOT NULL CHECK(role IN ('readonly','member','admin')),
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        used_by TEXT,
        used_at INTEGER
    );
    "#,
];

/// SHA-256 hex of a raw secret (token or invite code). Never store raw values.
pub fn hash_secret(raw: &str) -> String {
    let d = digest(&SHA256, raw.as_bytes());
    hex::encode(d.as_ref())
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum IdentityError {
    Db(rusqlite::Error),
    NotFound,
    InviteUsed,
    InviteExpired,
    InvalidRole,
    PrincipalDisabled,
    TokenExpired,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Db(e) => write!(f, "db error: {e}"),
            IdentityError::NotFound => write!(f, "not found"),
            IdentityError::InviteUsed => write!(f, "invite already used"),
            IdentityError::InviteExpired => write!(f, "invite expired"),
            IdentityError::InvalidRole => write!(f, "invalid role"),
            IdentityError::PrincipalDisabled => write!(f, "principal disabled"),
            IdentityError::TokenExpired => write!(f, "token expired"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<rusqlite::Error> for IdentityError {
    fn from(e: rusqlite::Error) -> Self {
        IdentityError::Db(e)
    }
}

/// Thread-safe identity store. Clone-cheap (Arc<Mutex<Connection>>).
#[derive(Clone)]
pub struct IdentityStore {
    conn: Arc<Mutex<Connection>>,
}

impl IdentityStore {
    /// Open (or create) the identity database at `path` and run migrations.
    pub fn open(path: &Path) -> Result<Self, IdentityError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        run_migrations(&conn, MIGRATIONS)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// In-memory store (tests).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, IdentityError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn, MIGRATIONS)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Generate a random opaque secret (32 bytes, hex-encoded = 64 chars).
    fn generate_secret() -> String {
        use ring::rand::{SecureRandom, SystemRandom};
        let rng = SystemRandom::new();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes).expect("rng fill");
        hex::encode(bytes)
    }

    /// Create a principal and its first token. Returns (principal, raw_token).
    /// The raw token is returned ONCE — only its hash is stored.
    pub fn create_principal(
        &self,
        display_name: &str,
        role: Role,
        invited_by: Option<&str>,
    ) -> Result<(Principal, String), IdentityError> {
        let id = uuid::Uuid::new_v4().to_string();
        let raw_token = format!("zk_{}", Self::generate_secret());
        let token_hash = hash_secret(&raw_token);
        let now = now_epoch();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO principals (id, display_name, role, created_at, invited_by, disabled)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![id, display_name, role.as_str(), now as i64, invited_by],
        )?;
        conn.execute(
            "INSERT INTO tokens (token_hash, principal_id, label, created_at)
             VALUES (?1, ?2, 'initial', ?3)",
            params![token_hash, id, now as i64],
        )?;
        Ok((
            Principal {
                id,
                display_name: display_name.to_string(),
                role,
            },
            raw_token,
        ))
    }

    /// Mint an additional token for an existing principal.
    /// Returns the raw token (shown once).
    pub fn mint_token(&self, principal_id: &str, label: &str) -> Result<String, IdentityError> {
        let raw_token = format!("zk_{}", Self::generate_secret());
        let token_hash = hash_secret(&raw_token);
        let now = now_epoch();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        // Principal must exist and not be disabled
        let disabled: i64 = conn
            .query_row(
                "SELECT disabled FROM principals WHERE id = ?1",
                params![principal_id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => IdentityError::NotFound,
                other => IdentityError::Db(other),
            })?;
        if disabled != 0 {
            return Err(IdentityError::PrincipalDisabled);
        }
        conn.execute(
            "INSERT INTO tokens (token_hash, principal_id, label, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, principal_id, label, now as i64],
        )?;
        Ok(raw_token)
    }

    /// Resolve a raw Bearer token to its principal.
    /// Updates last_used_at on success. Returns None for unknown/expired/disabled.
    pub fn resolve_token(&self, raw_token: &str) -> Result<Option<Principal>, IdentityError> {
        let hash = hash_secret(raw_token);
        let now = now_epoch() as i64;
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let row = conn.query_row(
            "SELECT p.id, p.display_name, p.role, p.disabled, t.expires_at, t.principal_id
             FROM tokens t JOIN principals p ON p.id = t.principal_id
             WHERE t.token_hash = ?1",
            params![hash],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        );
        let (id, name, role_s, disabled, expires_at, pid) = match row {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(IdentityError::Db(e)),
        };
        if disabled != 0 {
            return Ok(None);
        }
        if let Some(exp) = expires_at
            && exp <= now
        {
            return Ok(None);
        }
        // Best-effort touch of last_used_at (non-fatal)
        let _ = conn.execute(
            "UPDATE tokens SET last_used_at = ?1 WHERE token_hash = ?2",
            params![now, hash],
        );
        let role = Role::parse_role(&role_s).ok_or(IdentityError::InvalidRole)?;
        let _ = pid;
        Ok(Some(Principal {
            id,
            display_name: name,
            role,
        }))
    }

    /// Create an invite code for a role. Returns the raw code (shown once).
    pub fn create_invite(&self, role: Role, ttl_secs: u64) -> Result<String, IdentityError> {
        if role == Role::Root {
            return Err(IdentityError::InvalidRole); // never invite to root
        }
        let raw_code = format!("zi_{}", Self::generate_secret());
        let code_hash = hash_secret(&raw_code);
        let now = now_epoch();
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO invites (code_hash, role, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![code_hash, role.as_str(), now as i64, (now + ttl_secs) as i64],
        )?;
        Ok(raw_code)
    }

    /// Redeem an invite code → creates principal + first token.
    /// Returns (principal, raw_token). One-time use.
    pub fn accept_invite(
        &self,
        raw_code: &str,
        display_name: &str,
    ) -> Result<(Principal, String), IdentityError> {
        let code_hash = hash_secret(raw_code);
        let now = now_epoch() as i64;
        let role_s: String;
        {
            let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            let row = conn.query_row(
                "SELECT role, expires_at, used_by FROM invites WHERE code_hash = ?1",
                params![code_hash],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            );
            let (role, expires_at, used_by) = match row {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Err(IdentityError::NotFound),
                Err(e) => return Err(IdentityError::Db(e)),
            };
            if used_by.is_some() {
                return Err(IdentityError::InviteUsed);
            }
            if expires_at <= now {
                return Err(IdentityError::InviteExpired);
            }
            role_s = role;
        }
        let role = Role::parse_role(&role_s).ok_or(IdentityError::InvalidRole)?;
        let (principal, raw_token) = self.create_principal(display_name, role, Some("invite"))?;
        {
            let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "UPDATE invites SET used_by = ?1, used_at = ?2 WHERE code_hash = ?3",
                params![principal.id, now, code_hash],
            )?;
        }
        Ok((principal, raw_token))
    }

    /// List all principals (public view, no token material).
    pub fn list_members(&self) -> Result<Vec<MemberView>, IdentityError> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT p.id, p.display_name, p.role, p.created_at, p.invited_by, p.disabled,
                    (SELECT MAX(t.last_used_at) FROM tokens t WHERE t.principal_id = p.id)
             FROM principals p ORDER BY p.created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MemberView {
                id: r.get(0)?,
                display_name: r.get(1)?,
                role: Role::parse_role(&r.get::<_, String>(2)?).unwrap_or(Role::Readonly),
                created_at: r.get::<_, i64>(3)? as u64,
                invited_by: r.get(4)?,
                disabled: r.get::<_, i64>(5)? != 0,
                last_used_at: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Disable a principal and revoke all its tokens.
    pub fn disable_principal(&self, principal_id: &str) -> Result<(), IdentityError> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let n = conn.execute(
            "UPDATE principals SET disabled = 1 WHERE id = ?1",
            params![principal_id],
        )?;
        if n == 0 {
            return Err(IdentityError::NotFound);
        }
        conn.execute(
            "DELETE FROM tokens WHERE principal_id = ?1",
            params![principal_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> IdentityStore {
        IdentityStore::in_memory().unwrap()
    }

    #[test]
    fn test_hash_secret_deterministic_and_full_len() {
        let h1 = hash_secret("secret-token");
        let h2 = hash_secret("secret-token");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // full SHA-256 hex
        assert_ne!(hash_secret("a"), hash_secret("b"));
    }

    #[test]
    fn test_role_ordering() {
        assert!(Role::Readonly < Role::Member);
        assert!(Role::Member < Role::Admin);
        assert!(Role::Admin < Role::Root);
        assert_eq!(Role::parse_role("admin"), Some(Role::Admin));
        assert_eq!(Role::parse_role("bogus"), None);
    }

    #[test]
    fn test_create_and_resolve_roundtrip() {
        let s = store();
        let (p, raw) = s.create_principal("alice", Role::Member, None).unwrap();
        let resolved = s.resolve_token(&raw).unwrap().expect("should resolve");
        assert_eq!(resolved.id, p.id);
        assert_eq!(resolved.display_name, "alice");
        assert_eq!(resolved.role, Role::Member);
        // Unknown token does not resolve
        assert!(s.resolve_token("zk_nope").unwrap().is_none());
    }

    #[test]
    fn test_no_raw_token_in_db() {
        let s = store();
        let (_p, raw) = s.create_principal("bob", Role::Admin, None).unwrap();
        let conn = s.conn.lock().unwrap();
        // The raw token string must NOT appear anywhere in the tokens table
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tokens WHERE token_hash = ?1 OR label = ?1",
                params![raw],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        // The hash SHOULD be there
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tokens WHERE token_hash = ?1",
                params![hash_secret(&raw)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_invite_lifecycle() {
        let s = store();
        let code = s.create_invite(Role::Member, 3600).unwrap();
        let (p, raw) = s.accept_invite(&code, "carol").unwrap();
        assert_eq!(p.role, Role::Member);
        assert!(s.resolve_token(&raw).unwrap().is_some());
        // One-time use
        let err = s.accept_invite(&code, "mallory").unwrap_err();
        assert!(matches!(err, IdentityError::InviteUsed));
    }

    #[test]
    fn test_invite_expired_and_unknown() {
        let s = store();
        // ttl 0 → immediately expired
        let code = s.create_invite(Role::Member, 0).unwrap();
        assert!(matches!(
            s.accept_invite(&code, "dave"),
            Err(IdentityError::InviteExpired)
        ));
        assert!(matches!(
            s.accept_invite("zi_bogus", "dave"),
            Err(IdentityError::NotFound)
        ));
    }

    #[test]
    fn test_no_root_invites() {
        let s = store();
        assert!(matches!(
            s.create_invite(Role::Root, 3600),
            Err(IdentityError::InvalidRole)
        ));
    }

    #[test]
    fn test_disable_revokes_tokens() {
        let s = store();
        let (p, raw) = s.create_principal("eve", Role::Member, None).unwrap();
        assert!(s.resolve_token(&raw).unwrap().is_some());
        s.disable_principal(&p.id).unwrap();
        // Token deleted → no resolution
        assert!(s.resolve_token(&raw).unwrap().is_none());
        // Mint for disabled principal fails
        assert!(matches!(
            s.mint_token(&p.id, "x"),
            Err(IdentityError::PrincipalDisabled)
        ));
        // Unknown principal disable → NotFound
        assert!(matches!(
            s.disable_principal("nonexistent"),
            Err(IdentityError::NotFound)
        ));
    }

    #[test]
    fn test_mint_additional_token() {
        let s = store();
        let (p, _raw1) = s.create_principal("frank", Role::Readonly, None).unwrap();
        let raw2 = s.mint_token(&p.id, "second").unwrap();
        let r = s.resolve_token(&raw2).unwrap().unwrap();
        assert_eq!(r.id, p.id);
        assert_eq!(r.role, Role::Readonly);
    }

    #[test]
    fn test_list_members_no_token_material() {
        let s = store();
        s.create_principal("gina", Role::Member, None).unwrap();
        s.create_principal("hank", Role::Admin, None).unwrap();
        let members = s.list_members().unwrap();
        assert_eq!(members.len(), 2);
        // MemberView has no token fields at all (compile-time guarantee),
        // but assert the JSON surface too:
        let json = serde_json::to_string(&members).unwrap();
        assert!(!json.contains("token"));
        assert!(!json.contains("hash"));
    }

    #[test]
    fn test_last_used_updates() {
        let s = store();
        let (p, raw) = s.create_principal("ivy", Role::Member, None).unwrap();
        assert!(s.list_members().unwrap()[0].last_used_at.is_none());
        s.resolve_token(&raw).unwrap();
        let members = s.list_members().unwrap();
        let m = members.iter().find(|m| m.id == p.id).unwrap();
        assert!(m.last_used_at.is_some());
    }
}
