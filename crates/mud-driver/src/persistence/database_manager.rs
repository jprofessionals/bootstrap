use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rand::Rng;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::config::DatabaseConfig;

// ---------------------------------------------------------------------------
// AreaMeta — metadata for a registered area
// ---------------------------------------------------------------------------

/// Metadata for a registered area from the `area_registry` table.
#[derive(Debug, Clone)]
pub struct AreaMeta {
    pub namespace: String,
    pub area_name: String,
    pub owner: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DatabaseManager
// ---------------------------------------------------------------------------

/// Manages PostgreSQL databases: the driver DB (infrastructure) and the
/// stdlib DB (player accounts, characters, sessions).
///
/// This mirrors the Ruby project's two-database architecture:
/// - **Driver DB**: area_databases, area_registry, merge_requests, merge_request_approvals
/// - **Stdlib DB**: players, characters, access_tokens, sessions
pub struct DatabaseManager {
    admin_pool: PgPool,
    driver_pool: PgPool,
    stdlib_pool: PgPool,
    driver_db: String,
    stdlib_db: String,
    db_host: String,
    db_port: u16,
}

impl DatabaseManager {
    /// Connect to admin (`postgres`), create both driver and stdlib databases
    /// if they don't exist, then connect to each.
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        let admin_password = config
            .admin_password
            .as_deref()
            .unwrap_or("");

        // 1. Connect to the default `postgres` database for admin operations.
        let admin_url = format!(
            "postgres://{}:{}@{}:{}/postgres",
            config.admin_user, admin_password, config.host, config.port
        );
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&admin_url)
            .await
            .context("connecting to admin database")?;

        // 2. Create the driver database if it does not exist.
        Self::ensure_database(&admin_pool, &config.driver_db).await?;

        // 3. Create the stdlib database if it does not exist.
        Self::ensure_database(&admin_pool, &config.stdlib_db).await?;

        // 4. Connect to both databases.
        let driver_url = format!(
            "postgres://{}:{}@{}:{}/{}",
            config.admin_user, admin_password, config.host, config.port, config.driver_db
        );
        let driver_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&driver_url)
            .await
            .context("connecting to driver database")?;

        let stdlib_url = format!(
            "postgres://{}:{}@{}:{}/{}",
            config.admin_user, admin_password, config.host, config.port, config.stdlib_db
        );
        let stdlib_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&stdlib_url)
            .await
            .context("connecting to stdlib database")?;

        Ok(Self {
            admin_pool,
            driver_pool,
            stdlib_pool,
            driver_db: config.driver_db.clone(),
            stdlib_db: config.stdlib_db.clone(),
            db_host: config.host.clone(),
            db_port: config.port,
        })
    }

    /// Create a database if it does not already exist.
    async fn ensure_database(admin_pool: &PgPool, db_name: &str) -> Result<()> {
        Self::validate_name(db_name)?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
        )
        .bind(db_name)
        .fetch_one(admin_pool)
        .await?;

        if !exists {
            sqlx::query(&format!("CREATE DATABASE \"{}\"", db_name))
                .execute(admin_pool)
                .await
                .with_context(|| format!("creating database '{}'", db_name))?;
        }
        Ok(())
    }

    /// Run driver schema migrations on boot.
    ///
    /// Only driver tables (area_databases, area_registry, merge_requests, etc.)
    /// are migrated here. Stdlib tables (players, characters, etc.) are owned
    /// by the Ruby adapter, which receives the stdlib DB URL via a Configure
    /// message and runs its own Sequel migrations.
    ///
    /// Each statement is idempotent, safe to re-run on every boot.
    pub async fn setup(&self) -> Result<()> {
        for sql in super::migrations::DRIVER_MIGRATIONS {
            sqlx::query(sql)
                .execute(&self.driver_pool)
                .await
                .with_context(|| {
                    let preview: String = sql.chars().take(60).collect();
                    format!("running driver migration: {preview}")
                })?;
        }
        Ok(())
    }

    /// Return a reference to the driver database connection pool.
    pub fn driver_pool(&self) -> &PgPool {
        &self.driver_pool
    }

    /// Return a reference to the stdlib database connection pool.
    ///
    /// Used by `PlayerStore` for account/character operations.
    pub fn stdlib_pool(&self) -> &PgPool {
        &self.stdlib_pool
    }

    /// Return the driver database name.
    pub fn driver_db(&self) -> &str {
        &self.driver_db
    }

    /// Return the stdlib database name.
    pub fn stdlib_db(&self) -> &str {
        &self.stdlib_db
    }

    /// Build a connection URL for an area's dedicated database.
    ///
    /// Reads credentials from the `area_databases` table and combines them
    /// with the configured database host/port to produce a `postgres://` URL.
    /// Returns `None` if no credentials exist for this area.
    pub async fn get_area_db_url(&self, ns: &str, area: &str) -> Result<Option<String>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT db_name, db_user, db_password FROM area_databases \
             WHERE namespace = $1 AND area_name = $2",
        )
        .bind(ns)
        .bind(area)
        .fetch_optional(&self.driver_pool)
        .await?;

        match row {
            Some((db_name, db_user, db_password)) => {
                let url = format!(
                    "postgres://{}:{}@{}:{}/{}",
                    urlencoding::encode(&db_user),
                    urlencoding::encode(&db_password),
                    self.db_host,
                    self.db_port,
                    db_name,
                );
                Ok(Some(url))
            }
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Area registry
    // -----------------------------------------------------------------------

    /// Register an area in the `area_registry` table.
    ///
    /// Uses `ON CONFLICT DO NOTHING` so calling this twice for the same
    /// namespace/area is harmless.
    pub async fn register_area(&self, ns: &str, area: &str, owner: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO area_registry (namespace, area_name, owner) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (namespace, area_name) DO NOTHING",
        )
        .bind(ns)
        .bind(area)
        .bind(owner)
        .execute(&self.driver_pool)
        .await
        .context("registering area")?;
        Ok(())
    }

    /// Look up metadata for a single area. Returns `None` if the area is not
    /// registered.
    pub async fn find_area_meta(&self, ns: &str, area: &str) -> Result<Option<AreaMeta>> {
        let row = sqlx::query(
            "SELECT namespace, area_name, owner, created_at \
             FROM area_registry \
             WHERE namespace = $1 AND area_name = $2",
        )
        .bind(ns)
        .bind(area)
        .fetch_optional(&self.driver_pool)
        .await
        .context("finding area metadata")?;

        Ok(row.map(|r| AreaMeta {
            namespace: r.get("namespace"),
            area_name: r.get("area_name"),
            owner: r.get("owner"),
            created_at: r.get("created_at"),
        }))
    }

    /// List all registered areas ordered by namespace, then area name.
    pub async fn list_areas(&self) -> Result<Vec<AreaMeta>> {
        let rows = sqlx::query(
            "SELECT namespace, area_name, owner, created_at \
             FROM area_registry \
             ORDER BY namespace, area_name",
        )
        .fetch_all(&self.driver_pool)
        .await
        .context("listing areas")?;

        Ok(rows
            .into_iter()
            .map(|r| AreaMeta {
                namespace: r.get("namespace"),
                area_name: r.get("area_name"),
                owner: r.get("owner"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Remove an area from the registry.
    pub async fn unregister_area(&self, ns: &str, area: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM area_registry \
             WHERE namespace = $1 AND area_name = $2",
        )
        .bind(ns)
        .bind(area)
        .execute(&self.driver_pool)
        .await
        .context("unregistering area")?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Per-area database provisioning
    // -----------------------------------------------------------------------

    /// Provision a dedicated database and PostgreSQL role for an area.
    ///
    /// The database and role names are derived from the namespace and area
    /// name (`mud_area_<ns>_<area>`). A random password is generated and
    /// stored in the `area_databases` table.
    ///
    /// NOTE: Passwords are stored as plaintext for now. Task 2 will add
    /// `CredentialEncryptor` for AES-256-GCM encryption.
    pub async fn provision_area_db(&self, ns: &str, area: &str) -> Result<()> {
        Self::validate_name(ns)?;
        Self::validate_name(area)?;

        // If credentials already exist, skip — the DB and role are already set up.
        let already_provisioned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM area_databases WHERE namespace = $1 AND area_name = $2)",
        )
        .bind(ns)
        .bind(area)
        .fetch_one(&self.driver_pool)
        .await?;

        if already_provisioned {
            return Ok(());
        }

        let db_name = format!("mud_area_{}_{}", ns, area);
        let db_user = format!("mud_area_{}_{}", ns, area);
        let db_password = generate_password(32);

        // Create the role if it does not exist.
        let role_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)",
        )
        .bind(&db_user)
        .fetch_one(&self.admin_pool)
        .await?;

        if !role_exists {
            // Role names are validated above, safe for interpolation.
            sqlx::query(&format!(
                "CREATE ROLE \"{}\" LOGIN PASSWORD '{}'",
                db_user,
                db_password.replace('\'', "''")
            ))
            .execute(&self.admin_pool)
            .await
            .context("creating area database role")?;
        }

        // Create the database if it does not exist.
        let db_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
        )
        .bind(&db_name)
        .fetch_one(&self.admin_pool)
        .await?;

        if !db_exists {
            sqlx::query(&format!(
                "CREATE DATABASE \"{}\" OWNER \"{}\"",
                db_name, db_user
            ))
            .execute(&self.admin_pool)
            .await
            .context("creating area database")?;
        }

        // Store credentials in the driver database.
        sqlx::query(
            "INSERT INTO area_databases (namespace, area_name, db_name, db_user, db_password) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (namespace, area_name) DO NOTHING",
        )
        .bind(ns)
        .bind(area)
        .bind(&db_name)
        .bind(&db_user)
        .bind(&db_password) // plaintext for now; Task 2 adds encryption
        .execute(&self.driver_pool)
        .await
        .context("storing area database credentials")?;

        Ok(())
    }

    /// Drop a per-area database and its role, and remove the credentials
    /// record from the driver database.
    pub async fn drop_area_db(&self, ns: &str, area: &str) -> Result<()> {
        Self::validate_name(ns)?;
        Self::validate_name(area)?;

        let db_name = format!("mud_area_{}_{}", ns, area);
        let db_user = format!("mud_area_{}_{}", ns, area);

        // Drop the database if it exists.
        let db_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
        )
        .bind(&db_name)
        .fetch_one(&self.admin_pool)
        .await?;

        if db_exists {
            sqlx::query(&format!("DROP DATABASE \"{}\"", db_name))
                .execute(&self.admin_pool)
                .await
                .context("dropping area database")?;
        }

        // Drop the role if it exists.
        let role_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)",
        )
        .bind(&db_user)
        .fetch_one(&self.admin_pool)
        .await?;

        if role_exists {
            sqlx::query(&format!("DROP ROLE \"{}\"", db_user))
                .execute(&self.admin_pool)
                .await
                .context("dropping area database role")?;
        }

        // Remove the credentials record.
        sqlx::query(
            "DELETE FROM area_databases \
             WHERE namespace = $1 AND area_name = $2",
        )
        .bind(ns)
        .bind(area)
        .execute(&self.driver_pool)
        .await
        .context("removing area database credentials")?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Name validation
    // -----------------------------------------------------------------------

    /// Validate that a name contains only `[a-z0-9_]` characters.
    ///
    /// This is a security measure to prevent SQL injection in DDL statements
    /// where parameter binding is not available (e.g. `CREATE DATABASE`).
    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("invalid name '{}': must match [a-z0-9_]+", name);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_name tests --

    #[test]
    fn validate_name_accepts_lowercase() {
        assert!(DatabaseManager::validate_name("hello").is_ok());
    }

    #[test]
    fn validate_name_accepts_digits() {
        assert!(DatabaseManager::validate_name("area42").is_ok());
    }

    #[test]
    fn validate_name_accepts_underscores() {
        assert!(DatabaseManager::validate_name("my_area_name").is_ok());
    }

    #[test]
    fn validate_name_accepts_mixed() {
        assert!(DatabaseManager::validate_name("mud_area_123_test").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(DatabaseManager::validate_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_uppercase() {
        assert!(DatabaseManager::validate_name("Hello").is_err());
    }

    #[test]
    fn validate_name_rejects_dash() {
        assert!(DatabaseManager::validate_name("my-area").is_err());
    }

    #[test]
    fn validate_name_rejects_space() {
        assert!(DatabaseManager::validate_name("my area").is_err());
    }

    #[test]
    fn validate_name_rejects_dot() {
        assert!(DatabaseManager::validate_name("my.area").is_err());
    }

    #[test]
    fn validate_name_rejects_at_sign() {
        assert!(DatabaseManager::validate_name("area@dev").is_err());
    }

    #[test]
    fn validate_name_rejects_semicolon() {
        assert!(DatabaseManager::validate_name("area;drop").is_err());
    }

    #[test]
    fn validate_name_rejects_unicode() {
        assert!(DatabaseManager::validate_name("ärëa").is_err());
    }

    // -- generate_password tests --

    #[test]
    fn generate_password_correct_length() {
        assert_eq!(generate_password(32).len(), 32);
        assert_eq!(generate_password(0).len(), 0);
        assert_eq!(generate_password(1).len(), 1);
        assert_eq!(generate_password(100).len(), 100);
    }

    #[test]
    fn generate_password_only_alphanumeric() {
        let pw = generate_password(1000);
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn generate_password_produces_different_values() {
        let pw1 = generate_password(32);
        let pw2 = generate_password(32);
        // Extremely unlikely to be equal with 32 random chars
        assert_ne!(pw1, pw2);
    }

    #[test]
    fn generate_password_zero_length() {
        assert_eq!(generate_password(0), "");
    }
}

/// Generate a random alphanumeric password of the given length.
fn generate_password(len: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
