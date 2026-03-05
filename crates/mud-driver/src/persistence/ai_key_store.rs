use anyhow::Result;
use sqlx::{PgPool, Row};
use std::sync::Arc;

use super::credential_encryptor::CredentialEncryptor;

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

    /// List which providers have stored keys for this account.
    pub async fn list_providers(&self, account: &str) -> Result<Vec<String>> {
        let rows =
            sqlx::query("SELECT provider FROM ai_api_keys WHERE account = $1 ORDER BY provider")
                .bind(account)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(|r| r.get("provider")).collect())
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
