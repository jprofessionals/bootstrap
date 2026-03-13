// ============================================================================
// Driver DB migrations — infrastructure tables owned by the driver kernel
// ============================================================================

pub const CREATE_AREA_DATABASES: &str = r#"
CREATE TABLE IF NOT EXISTS area_databases (
    namespace VARCHAR NOT NULL,
    area_name VARCHAR NOT NULL,
    db_name VARCHAR NOT NULL,
    db_user VARCHAR NOT NULL,
    db_password TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(namespace, area_name)
)
"#;

pub const CREATE_AREA_REGISTRY: &str = r#"
CREATE TABLE IF NOT EXISTS area_registry (
    namespace VARCHAR NOT NULL,
    area_name VARCHAR NOT NULL,
    owner VARCHAR NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(namespace, area_name)
)
"#;

pub const CREATE_MERGE_REQUESTS: &str = r#"
CREATE TABLE IF NOT EXISTS merge_requests (
    id SERIAL PRIMARY KEY,
    namespace VARCHAR NOT NULL,
    area_name VARCHAR NOT NULL,
    title VARCHAR NOT NULL,
    description TEXT,
    author VARCHAR NOT NULL,
    state VARCHAR DEFAULT 'open',
    source_branch VARCHAR NOT NULL DEFAULT 'develop',
    target_branch VARCHAR NOT NULL DEFAULT 'main',
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
)
"#;

pub const CREATE_MERGE_REQUEST_APPROVALS: &str = r#"
CREATE TABLE IF NOT EXISTS merge_request_approvals (
    id SERIAL PRIMARY KEY,
    merge_request_id INTEGER NOT NULL REFERENCES merge_requests(id),
    approver VARCHAR NOT NULL,
    comment TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
)
"#;

pub const CREATE_AI_API_KEYS: &str = "
CREATE TABLE IF NOT EXISTS ai_api_keys (
    account TEXT NOT NULL,
    encrypted_key BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account)
)";

pub const ALTER_AI_API_KEYS_ADD_PROVIDER: &str = "
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'ai_api_keys' AND column_name = 'provider'
    ) THEN
        ALTER TABLE ai_api_keys ADD COLUMN provider TEXT NOT NULL DEFAULT 'anthropic';
        ALTER TABLE ai_api_keys DROP CONSTRAINT ai_api_keys_pkey;
        ALTER TABLE ai_api_keys ADD PRIMARY KEY (account, provider);
    END IF;
END $$";

pub const CREATE_AI_PREFERENCES: &str = "
CREATE TABLE IF NOT EXISTS ai_preferences (
    account TEXT PRIMARY KEY,
    default_provider TEXT NOT NULL DEFAULT 'anthropic',
    default_model TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
)";

pub const ALTER_AI_API_KEYS_ADD_ENABLED: &str = "
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'ai_api_keys' AND column_name = 'enabled'
    ) THEN
        ALTER TABLE ai_api_keys ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT true;
    END IF;
END $$";

pub const CREATE_AI_CUSTOM_PROVIDERS: &str = "
CREATE TABLE IF NOT EXISTS ai_custom_providers (
    id SERIAL PRIMARY KEY,
    account TEXT NOT NULL,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_mode TEXT NOT NULL,
    encrypted_key BYTEA NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(account, name)
)";

/// Driver DB migrations — run against the driver database on boot.
pub const DRIVER_MIGRATIONS: &[&str] = &[
    CREATE_AREA_DATABASES,
    CREATE_AREA_REGISTRY,
    CREATE_MERGE_REQUESTS,
    CREATE_MERGE_REQUEST_APPROVALS,
    CREATE_AI_API_KEYS,
    ALTER_AI_API_KEYS_ADD_PROVIDER,
    CREATE_AI_PREFERENCES,
    ALTER_AI_API_KEYS_ADD_ENABLED,
    CREATE_AI_CUSTOM_PROVIDERS,
];

// ============================================================================
// NOTE: Stdlib table schemas (players, characters, access_tokens, sessions)
// are owned by the Ruby stdlib. The driver sends the stdlib DB URL to the
// adapter via a Configure message, and the adapter runs its own Sequel
// migrations. See bootstrap/ruby/stdlib/migrations/.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_migrations_count() {
        assert_eq!(DRIVER_MIGRATIONS.len(), 9);
    }

    #[test]
    fn area_databases_migration_creates_table() {
        assert!(CREATE_AREA_DATABASES.contains("CREATE TABLE IF NOT EXISTS area_databases"));
        assert!(CREATE_AREA_DATABASES.contains("namespace"));
        assert!(CREATE_AREA_DATABASES.contains("area_name"));
        assert!(CREATE_AREA_DATABASES.contains("db_name"));
        assert!(CREATE_AREA_DATABASES.contains("db_user"));
        assert!(CREATE_AREA_DATABASES.contains("db_password"));
        assert!(CREATE_AREA_DATABASES.contains("UNIQUE(namespace, area_name)"));
    }

    #[test]
    fn area_registry_migration_creates_table() {
        assert!(CREATE_AREA_REGISTRY.contains("CREATE TABLE IF NOT EXISTS area_registry"));
        assert!(CREATE_AREA_REGISTRY.contains("namespace"));
        assert!(CREATE_AREA_REGISTRY.contains("area_name"));
        assert!(CREATE_AREA_REGISTRY.contains("owner"));
        assert!(CREATE_AREA_REGISTRY.contains("UNIQUE(namespace, area_name)"));
    }

    #[test]
    fn merge_requests_migration_creates_table() {
        assert!(CREATE_MERGE_REQUESTS.contains("CREATE TABLE IF NOT EXISTS merge_requests"));
        assert!(CREATE_MERGE_REQUESTS.contains("SERIAL PRIMARY KEY"));
        assert!(CREATE_MERGE_REQUESTS.contains("source_branch"));
        assert!(CREATE_MERGE_REQUESTS.contains("target_branch"));
        assert!(CREATE_MERGE_REQUESTS.contains("state"));
        assert!(CREATE_MERGE_REQUESTS.contains("'open'"));
    }

    #[test]
    fn merge_request_approvals_migration_creates_table() {
        assert!(CREATE_MERGE_REQUEST_APPROVALS
            .contains("CREATE TABLE IF NOT EXISTS merge_request_approvals"));
        assert!(CREATE_MERGE_REQUEST_APPROVALS.contains("REFERENCES merge_requests(id)"));
        assert!(CREATE_MERGE_REQUEST_APPROVALS.contains("approver"));
    }

    #[test]
    fn all_migrations_are_idempotent() {
        for sql in DRIVER_MIGRATIONS {
            assert!(
                sql.contains("IF NOT EXISTS") || sql.contains("DO $$"),
                "Migration should be idempotent (contain IF NOT EXISTS or DO $$ block): {}",
                &sql[..60]
            );
        }
    }

    #[test]
    fn migrations_order() {
        assert!(DRIVER_MIGRATIONS[0].contains("area_databases"));
        assert!(DRIVER_MIGRATIONS[1].contains("area_registry"));
        assert!(DRIVER_MIGRATIONS[2].contains("merge_requests"));
        assert!(DRIVER_MIGRATIONS[3].contains("merge_request_approvals"));
        assert!(DRIVER_MIGRATIONS[4].contains("ai_api_keys"));
        assert!(DRIVER_MIGRATIONS[5].contains("ADD COLUMN provider"));
        assert!(DRIVER_MIGRATIONS[6].contains("ai_preferences"));
        assert!(DRIVER_MIGRATIONS[7].contains("ADD COLUMN enabled"));
        assert!(DRIVER_MIGRATIONS[8].contains("ai_custom_providers"));
    }
}
