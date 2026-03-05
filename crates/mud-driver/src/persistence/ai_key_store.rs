use anyhow::Result;
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::sync::Arc;

use super::credential_encryptor::CredentialEncryptor;

/// Status info for a built-in provider (returned by list_providers).
#[derive(Debug, Serialize)]
pub struct ProviderStatus {
    pub configured: bool,
    pub enabled: bool,
}

/// A custom (self-hosted) LLM provider entry.
#[derive(Debug, Serialize)]
pub struct CustomProvider {
    pub id: i32,
    pub name: String,
    pub base_url: String,
    pub api_mode: String,
    pub enabled: bool,
}

pub struct AiKeyStore {
    pool: PgPool,
    encryptor: Arc<CredentialEncryptor>,
}

impl AiKeyStore {
    pub fn new(pool: PgPool, encryptor: Arc<CredentialEncryptor>) -> Self {
        Self { pool, encryptor }
    }

    /// Store an encrypted API key for a specific provider.
    pub async fn store_ai_api_key(
        &self,
        account: &str,
        provider: &str,
        api_key: &str,
    ) -> Result<()> {
        let encrypted = self.encryptor.encrypt(api_key)?;
        sqlx::query(
            "INSERT INTO ai_api_keys (account, provider, encrypted_key, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (account, provider) DO UPDATE \
             SET encrypted_key = EXCLUDED.encrypted_key, updated_at = EXCLUDED.updated_at",
        )
        .bind(account)
        .bind(provider)
        .bind(encrypted.as_bytes())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retrieve and decrypt an API key for a specific provider.
    pub async fn get_ai_api_key(
        &self,
        account: &str,
        provider: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT encrypted_key FROM ai_api_keys WHERE account = $1 AND provider = $2",
        )
        .bind(account)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let encrypted: Vec<u8> = row.get("encrypted_key");
                let encrypted_str =
                    String::from_utf8(encrypted).map_err(|e| anyhow::anyhow!("{}", e))?;
                let decrypted = self.encryptor.decrypt(&encrypted_str)?;
                Ok(Some(decrypted))
            }
            None => Ok(None),
        }
    }

    /// Delete the API key for a specific provider.
    pub async fn delete_ai_api_key(&self, account: &str, provider: &str) -> Result<()> {
        sqlx::query("DELETE FROM ai_api_keys WHERE account = $1 AND provider = $2")
            .bind(account)
            .bind(provider)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List which providers have stored keys for this account (legacy, returns names only).
    pub async fn list_providers(&self, account: &str) -> Result<Vec<String>> {
        let rows =
            sqlx::query("SELECT provider FROM ai_api_keys WHERE account = $1 ORDER BY provider")
                .bind(account)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(|r| r.get("provider")).collect())
    }

    /// List built-in providers with their configured/enabled status.
    pub async fn list_provider_statuses(
        &self,
        account: &str,
    ) -> Result<Vec<(String, ProviderStatus)>> {
        let rows = sqlx::query(
            "SELECT provider, enabled FROM ai_api_keys WHERE account = $1 ORDER BY provider",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;

        let configured: std::collections::HashMap<String, bool> = rows
            .iter()
            .map(|r| (r.get("provider"), r.get("enabled")))
            .collect();

        let mut result = Vec::new();
        for name in &["anthropic", "openai", "gemini"] {
            let status = match configured.get(*name) {
                Some(&enabled) => ProviderStatus {
                    configured: true,
                    enabled,
                },
                None => ProviderStatus {
                    configured: false,
                    enabled: false,
                },
            };
            result.push((name.to_string(), status));
        }
        Ok(result)
    }

    /// Toggle a built-in provider's enabled flag.
    pub async fn toggle_provider_enabled(
        &self,
        account: &str,
        provider: &str,
        enabled: bool,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE ai_api_keys SET enabled = $3, updated_at = NOW() \
             WHERE account = $1 AND provider = $2",
        )
        .bind(account)
        .bind(provider)
        .bind(enabled)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            anyhow::bail!("No API key configured for provider: {}", provider);
        }
        Ok(())
    }

    /// Check if a provider is enabled for this account.
    pub async fn is_provider_enabled(&self, account: &str, provider: &str) -> Result<bool> {
        let row = sqlx::query(
            "SELECT enabled FROM ai_api_keys WHERE account = $1 AND provider = $2",
        )
        .bind(account)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get("enabled")).unwrap_or(false))
    }

    // -----------------------------------------------------------------------
    // Custom providers (self-hosted LLMs)
    // -----------------------------------------------------------------------

    /// Create a custom provider entry.
    pub async fn create_custom_provider(
        &self,
        account: &str,
        name: &str,
        base_url: &str,
        api_mode: &str,
        api_key: &str,
    ) -> Result<CustomProvider> {
        // Enforce max 5 custom providers per account
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ai_custom_providers WHERE account = $1",
        )
        .bind(account)
        .fetch_one(&self.pool)
        .await?;

        if count >= 5 {
            anyhow::bail!("Maximum of 5 custom providers reached");
        }

        let encrypted = self.encryptor.encrypt(api_key)?;
        let row = sqlx::query(
            "INSERT INTO ai_custom_providers (account, name, base_url, api_mode, encrypted_key) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, name, base_url, api_mode, enabled",
        )
        .bind(account)
        .bind(name)
        .bind(base_url)
        .bind(api_mode)
        .bind(encrypted.as_bytes())
        .fetch_one(&self.pool)
        .await?;

        Ok(CustomProvider {
            id: row.get("id"),
            name: row.get("name"),
            base_url: row.get("base_url"),
            api_mode: row.get("api_mode"),
            enabled: row.get("enabled"),
        })
    }

    /// List custom providers for an account (does not return keys).
    pub async fn list_custom_providers(&self, account: &str) -> Result<Vec<CustomProvider>> {
        let rows = sqlx::query(
            "SELECT id, name, base_url, api_mode, enabled \
             FROM ai_custom_providers WHERE account = $1 ORDER BY id",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| CustomProvider {
                id: r.get("id"),
                name: r.get("name"),
                base_url: r.get("base_url"),
                api_mode: r.get("api_mode"),
                enabled: r.get("enabled"),
            })
            .collect())
    }

    /// Update a custom provider. Only updates fields that are Some.
    pub async fn update_custom_provider(
        &self,
        account: &str,
        id: i32,
        name: Option<&str>,
        base_url: Option<&str>,
        api_mode: Option<&str>,
        api_key: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<()> {
        // Build dynamic UPDATE query
        let mut sets = Vec::new();
        let mut bind_idx = 3u32; // $1 = account, $2 = id

        if name.is_some() {
            sets.push(format!("name = ${bind_idx}"));
            bind_idx += 1;
        }
        if base_url.is_some() {
            sets.push(format!("base_url = ${bind_idx}"));
            bind_idx += 1;
        }
        if api_mode.is_some() {
            sets.push(format!("api_mode = ${bind_idx}"));
            bind_idx += 1;
        }
        if api_key.is_some() {
            sets.push(format!("encrypted_key = ${bind_idx}"));
            bind_idx += 1;
        }
        if enabled.is_some() {
            sets.push(format!("enabled = ${bind_idx}"));
            // bind_idx not needed after this
        }

        if sets.is_empty() {
            return Ok(());
        }

        sets.push("updated_at = NOW()".to_string());
        let sql = format!(
            "UPDATE ai_custom_providers SET {} WHERE account = $1 AND id = $2",
            sets.join(", ")
        );

        let mut query = sqlx::query(&sql).bind(account).bind(id);

        if let Some(v) = name {
            query = query.bind(v);
        }
        if let Some(v) = base_url {
            query = query.bind(v);
        }
        if let Some(v) = api_mode {
            query = query.bind(v);
        }
        if let Some(v) = api_key {
            let encrypted = self.encryptor.encrypt(v)?;
            query = query.bind(encrypted.into_bytes());
        }
        if let Some(v) = enabled {
            query = query.bind(v);
        }

        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            anyhow::bail!("Custom provider not found");
        }
        Ok(())
    }

    /// Delete a custom provider.
    pub async fn delete_custom_provider(&self, account: &str, id: i32) -> Result<()> {
        let result =
            sqlx::query("DELETE FROM ai_custom_providers WHERE account = $1 AND id = $2")
                .bind(account)
                .bind(id)
                .execute(&self.pool)
                .await?;

        if result.rows_affected() == 0 {
            anyhow::bail!("Custom provider not found");
        }
        Ok(())
    }

    /// Get a custom provider's decrypted API key (for streaming).
    pub async fn get_custom_provider_key(
        &self,
        account: &str,
        id: i32,
    ) -> Result<Option<(String, String, String)>> {
        let row = sqlx::query(
            "SELECT base_url, api_mode, encrypted_key FROM ai_custom_providers \
             WHERE account = $1 AND id = $2 AND enabled = true",
        )
        .bind(account)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let base_url: String = row.get("base_url");
                let api_mode: String = row.get("api_mode");
                let encrypted: Vec<u8> = row.get("encrypted_key");
                let encrypted_str =
                    String::from_utf8(encrypted).map_err(|e| anyhow::anyhow!("{}", e))?;
                let decrypted = self.encryptor.decrypt(&encrypted_str)?;
                Ok(Some((base_url, api_mode, decrypted)))
            }
            None => Ok(None),
        }
    }

    /// Get the builder's default provider and model preferences.
    /// Returns ("anthropic", None) if no preferences are set.
    pub async fn get_preferences(&self, account: &str) -> Result<(String, Option<String>)> {
        let row = sqlx::query(
            "SELECT default_provider, default_model FROM ai_preferences WHERE account = $1",
        )
        .bind(account)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => Ok((r.get("default_provider"), r.get("default_model"))),
            None => Ok(("anthropic".to_string(), None)),
        }
    }

    /// Set the builder's default provider and model preferences.
    pub async fn set_preferences(
        &self,
        account: &str,
        provider: &str,
        model: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO ai_preferences (account, default_provider, default_model, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (account) DO UPDATE \
             SET default_provider = EXCLUDED.default_provider, \
                 default_model = EXCLUDED.default_model, \
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(account)
        .bind(provider)
        .bind(model)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
