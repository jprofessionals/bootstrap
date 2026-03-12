use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

/// A player account record from the `players` table.
#[derive(Debug, Clone)]
pub struct PlayerRecord {
    pub id: String,
    pub password_hash: String,
    pub role: String,
    pub active_character: Option<String>,
    pub builder_character_id: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// A character record from the `characters` table.
#[derive(Debug, Clone)]
pub struct CharacterRecord {
    pub id: i32,
    pub player_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// A personal access token record (without the hash).
#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub id: i32,
    pub name: String,
    pub token_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// AuthResult
// ---------------------------------------------------------------------------

/// Result of an authentication attempt (password or token).
#[derive(Debug)]
pub enum AuthResult {
    /// Authentication succeeded; contains the player record.
    Success(PlayerRecord),
    /// The player exists but the password/token was wrong.
    WrongPassword,
    /// No player with that id exists.
    NotFound,
}

// ---------------------------------------------------------------------------
// PlayerStore
// ---------------------------------------------------------------------------

/// Manages player accounts, characters, access tokens, and sessions
/// in the driver PostgreSQL database.
pub struct PlayerStore {
    pool: PgPool,
}

impl PlayerStore {
    /// Create a new `PlayerStore` backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // -----------------------------------------------------------------------
    // Accounts
    // -----------------------------------------------------------------------

    /// Create a new player account with the given id and pre-hashed password.
    pub async fn create(&self, id: &str, password_hash: &str) -> Result<()> {
        sqlx::query("INSERT INTO players (id, password_hash) VALUES ($1, $2)")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await
            .context("creating player")?;
        Ok(())
    }

    /// Find a player by id. Returns `None` if no such player exists.
    pub async fn find(&self, id: &str) -> Result<Option<PlayerRecord>> {
        let row = sqlx::query(
            "SELECT id, password_hash, role, active_character, \
                    builder_character_id, created_at \
             FROM players WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("finding player")?;

        Ok(row.map(|r| PlayerRecord {
            id: r.get("id"),
            password_hash: r.get("password_hash"),
            role: r.get("role"),
            active_character: r.get("active_character"),
            builder_character_id: r.get("builder_character_id"),
            created_at: r.get("created_at"),
        }))
    }

    /// Authenticate a player by id and plaintext password.
    ///
    /// Returns `AuthResult::Success` with the player record if the password
    /// matches, `AuthResult::WrongPassword` if the password is wrong, or
    /// `AuthResult::NotFound` if no player with that id exists.
    pub async fn authenticate(&self, id: &str, password: &str) -> Result<AuthResult> {
        let player = match self.find(id).await? {
            Some(p) => p,
            None => return Ok(AuthResult::NotFound),
        };

        if bcrypt::verify(password, &player.password_hash)? {
            Ok(AuthResult::Success(player))
        } else {
            Ok(AuthResult::WrongPassword)
        }
    }

    /// Hash a plaintext password using bcrypt with the default cost.
    pub fn hash_password(password: &str) -> Result<String> {
        Ok(bcrypt::hash(password, bcrypt::DEFAULT_COST)?)
    }

    /// Return the role for the given player, or `None` if the player does
    /// not exist.
    pub async fn account_role(&self, id: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT role FROM players WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .context("fetching account role")?;

        Ok(row.map(|r| r.get("role")))
    }

    /// Set the role for the given player.
    pub async fn set_role(&self, id: &str, role: &str) -> Result<()> {
        let result = sqlx::query("UPDATE players SET role = $1 WHERE id = $2")
            .bind(role)
            .bind(id)
            .execute(&self.pool)
            .await
            .context("setting account role")?;

        if result.rows_affected() == 0 {
            bail!("player '{}' not found", id);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Characters
    // -----------------------------------------------------------------------

    /// Add a new character for the given player. Returns the character id.
    pub async fn add_character(&self, player_id: &str, name: &str) -> Result<i32> {
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO characters (player_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(player_id)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .context("adding character")?;

        Ok(id)
    }

    /// List all characters belonging to the given player.
    pub async fn list_characters(&self, player_id: &str) -> Result<Vec<CharacterRecord>> {
        let rows = sqlx::query(
            "SELECT id, player_id, name, created_at \
             FROM characters WHERE player_id = $1 \
             ORDER BY created_at",
        )
        .bind(player_id)
        .fetch_all(&self.pool)
        .await
        .context("listing characters")?;

        Ok(rows
            .into_iter()
            .map(|r| CharacterRecord {
                id: r.get("id"),
                player_id: r.get("player_id"),
                name: r.get("name"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Switch the active character for the given player.
    pub async fn switch_character(&self, player_id: &str, character_name: &str) -> Result<()> {
        let result = sqlx::query("UPDATE players SET active_character = $1 WHERE id = $2")
            .bind(character_name)
            .bind(player_id)
            .execute(&self.pool)
            .await
            .context("switching character")?;

        if result.rows_affected() == 0 {
            bail!("player '{}' not found", player_id);
        }
        Ok(())
    }

    /// Set the builder character for the given player.
    ///
    /// Only allowed if the player's role is not `"player"` (i.e. they must
    /// be a builder, admin, or other privileged role).
    pub async fn set_builder_character(&self, player_id: &str, character_id: i32) -> Result<()> {
        let role = self
            .account_role(player_id)
            .await?
            .context("player not found")?;

        if role == "player" {
            bail!(
                "cannot set builder character: player '{}' has role 'player'",
                player_id
            );
        }

        let result = sqlx::query("UPDATE players SET builder_character_id = $1 WHERE id = $2")
            .bind(character_id)
            .bind(player_id)
            .execute(&self.pool)
            .await
            .context("setting builder character")?;

        if result.rows_affected() == 0 {
            bail!("player '{}' not found", player_id);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Access Tokens (PATs)
    // -----------------------------------------------------------------------

    /// Create a new personal access token for the given player.
    ///
    /// Returns `(full_token, prefix)`. The full token is only shown once;
    /// only the bcrypt hash and 8-char prefix are stored.
    pub async fn create_access_token(
        &self,
        player_id: &str,
        name: &str,
    ) -> Result<(String, String)> {
        let mut random_bytes = [0u8; 16];
        rand::fill(&mut random_bytes);
        let hex_part = hex::encode(random_bytes);
        let token = format!("mud_{}", hex_part);
        let prefix = hex_part[..8].to_string();
        let token_hash = bcrypt::hash(&token, bcrypt::DEFAULT_COST)?;

        sqlx::query(
            "INSERT INTO access_tokens (player_id, name, token_prefix, token_hash) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(player_id)
        .bind(name)
        .bind(&prefix)
        .bind(&token_hash)
        .execute(&self.pool)
        .await
        .context("creating access token")?;

        Ok((token, prefix))
    }

    /// Authenticate a player using a personal access token.
    ///
    /// Fetches all tokens for the player and tries `bcrypt::verify` against
    /// each one. Updates `last_used_at` on the matching token.
    pub async fn authenticate_token(&self, player_id: &str, token: &str) -> Result<AuthResult> {
        // Check player exists first.
        let player = match self.find(player_id).await? {
            Some(p) => p,
            None => return Ok(AuthResult::NotFound),
        };

        // Extract the prefix from the token to narrow the search.
        let prefix = token
            .strip_prefix("mud_")
            .and_then(|hex_part| hex_part.get(..8))
            .unwrap_or("");

        let rows = sqlx::query(
            "SELECT id, token_hash FROM access_tokens \
             WHERE player_id = $1 AND token_prefix = $2",
        )
        .bind(player_id)
        .bind(prefix)
        .fetch_all(&self.pool)
        .await
        .context("fetching access tokens for authentication")?;

        for row in rows {
            let token_id: i32 = row.get("id");
            let token_hash: String = row.get("token_hash");

            if bcrypt::verify(token, &token_hash)? {
                // Update last_used_at timestamp.
                sqlx::query(
                    "UPDATE access_tokens SET last_used_at = CURRENT_TIMESTAMP \
                     WHERE id = $1",
                )
                .bind(token_id)
                .execute(&self.pool)
                .await
                .context("updating token last_used_at")?;

                return Ok(AuthResult::Success(player));
            }
        }

        Ok(AuthResult::WrongPassword)
    }

    /// List all access tokens for the given player (without hashes).
    pub async fn list_access_tokens(&self, player_id: &str) -> Result<Vec<TokenRecord>> {
        let rows = sqlx::query(
            "SELECT id, name, token_prefix, created_at, last_used_at \
             FROM access_tokens WHERE player_id = $1 \
             ORDER BY created_at",
        )
        .bind(player_id)
        .fetch_all(&self.pool)
        .await
        .context("listing access tokens")?;

        Ok(rows
            .into_iter()
            .map(|r| TokenRecord {
                id: r.get("id"),
                name: r.get("name"),
                token_prefix: r.get("token_prefix"),
                created_at: r.get("created_at"),
                last_used_at: r.get("last_used_at"),
            })
            .collect())
    }

    /// Revoke (delete) an access token by id, scoped to the given player.
    pub async fn revoke_access_token(&self, player_id: &str, token_id: i32) -> Result<()> {
        sqlx::query("DELETE FROM access_tokens WHERE id = $1 AND player_id = $2")
            .bind(token_id)
            .bind(player_id)
            .execute(&self.pool)
            .await
            .context("revoking access token")?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    /// Create a new session for the given player. Returns the session token.
    pub async fn create_session(&self, player_id: &str) -> Result<String> {
        let mut token_bytes = [0u8; 32];
        rand::fill(&mut token_bytes);
        let token = hex::encode(token_bytes);
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));

        sqlx::query("INSERT INTO sessions (player_id, token) VALUES ($1, $2)")
            .bind(player_id)
            .bind(&token_hash)
            .execute(&self.pool)
            .await
            .context("creating session")?;

        Ok(token)
    }

    /// Check whether a session token is valid for the given player.
    pub async fn valid_session(&self, player_id: &str, token: &str) -> Result<bool> {
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE player_id = $1 AND token = $2 \
             AND created_at > NOW() - INTERVAL '30 days'",
        )
        .bind(player_id)
        .bind(&token_hash)
        .fetch_one(&self.pool)
        .await
        .context("validating session")?;

        Ok(count > 0)
    }

    /// Destroy a session by token.
    pub async fn destroy_session(&self, token: &str) -> Result<()> {
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(&token_hash)
            .execute(&self.pool)
            .await
            .context("destroying session")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- hash_password / verify (pure bcrypt, no DB needed) --

    #[test]
    fn hash_password_produces_bcrypt_hash() {
        let hash = PlayerStore::hash_password("secret").unwrap();
        assert!(hash.starts_with("$2b$") || hash.starts_with("$2a$"));
    }

    #[test]
    fn hash_password_verifies_correctly() {
        let hash = PlayerStore::hash_password("my_password").unwrap();
        assert!(bcrypt::verify("my_password", &hash).unwrap());
    }

    #[test]
    fn hash_password_rejects_wrong_password() {
        let hash = PlayerStore::hash_password("correct").unwrap();
        assert!(!bcrypt::verify("wrong", &hash).unwrap());
    }

    #[test]
    fn hash_password_different_hashes_for_same_input() {
        let h1 = PlayerStore::hash_password("test").unwrap();
        let h2 = PlayerStore::hash_password("test").unwrap();
        assert_ne!(h1, h2); // different salts
    }

    #[test]
    fn hash_password_empty_string() {
        let hash = PlayerStore::hash_password("").unwrap();
        assert!(bcrypt::verify("", &hash).unwrap());
    }

    #[test]
    fn hash_password_unicode() {
        let hash = PlayerStore::hash_password("pässwörd").unwrap();
        assert!(bcrypt::verify("pässwörd", &hash).unwrap());
    }

    #[test]
    fn hash_password_long_password() {
        let long = "a".repeat(72); // bcrypt max is 72 bytes
        let hash = PlayerStore::hash_password(&long).unwrap();
        assert!(bcrypt::verify(&long, &hash).unwrap());
    }

    // -- AuthResult variants --

    #[test]
    fn auth_result_debug_format() {
        let success = AuthResult::NotFound;
        let debug = format!("{:?}", success);
        assert!(debug.contains("NotFound"));
    }

    // -- Record types --

    #[test]
    fn player_record_clone() {
        let record = PlayerRecord {
            id: "alice".into(),
            password_hash: "$2b$12$hash".into(),
            role: "player".into(),
            active_character: Some("warrior".into()),
            builder_character_id: None,
            created_at: Utc::now(),
        };
        let cloned = record.clone();
        assert_eq!(cloned.id, "alice");
        assert_eq!(cloned.role, "player");
        assert_eq!(cloned.active_character, Some("warrior".into()));
        assert_eq!(cloned.builder_character_id, None);
    }

    #[test]
    fn character_record_clone() {
        let record = CharacterRecord {
            id: 1,
            player_id: "alice".into(),
            name: "Warrior".into(),
            created_at: Utc::now(),
        };
        let cloned = record.clone();
        assert_eq!(cloned.id, 1);
        assert_eq!(cloned.player_id, "alice");
        assert_eq!(cloned.name, "Warrior");
    }

    #[test]
    fn token_record_clone() {
        let record = TokenRecord {
            id: 1,
            name: "CI Token".into(),
            token_prefix: "abcd1234".into(),
            created_at: Utc::now(),
            last_used_at: None,
        };
        let cloned = record.clone();
        assert_eq!(cloned.id, 1);
        assert_eq!(cloned.name, "CI Token");
        assert_eq!(cloned.token_prefix, "abcd1234");
        assert!(cloned.last_used_at.is_none());
    }

    #[test]
    fn token_record_with_last_used() {
        let now = Utc::now();
        let record = TokenRecord {
            id: 2,
            name: "Dev Token".into(),
            token_prefix: "efgh5678".into(),
            created_at: now,
            last_used_at: Some(now),
        };
        assert!(record.last_used_at.is_some());
    }
}
