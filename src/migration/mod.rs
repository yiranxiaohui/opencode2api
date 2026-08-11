use std::path::Path;

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectOptions, Database};

use crate::error::ApiError;

mod m20260811_000001_initial {
    use sea_orm_migration::prelude::*;

    const SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS api_keys (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  api_key_enc TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '[]',
  notes TEXT NOT NULL DEFAULT '',
  model_cache TEXT NOT NULL DEFAULT '[]',
  is_default INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_keys_name ON api_keys(name COLLATE NOCASE);
CREATE UNIQUE INDEX IF NOT EXISTS idx_keys_default ON api_keys(is_default) WHERE is_default = 1;
CREATE TABLE IF NOT EXISTS proxies (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  url_enc TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS client_api_keys (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  key_hash TEXT NOT NULL UNIQUE,
  prefix TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER
);
CREATE TABLE IF NOT EXISTS request_logs (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  client_key_id TEXT,
  client_key_name TEXT NOT NULL,
  route_key_id TEXT,
  route_key_name TEXT,
  method TEXT NOT NULL,
  path TEXT NOT NULL,
  model TEXT,
  stream INTEGER NOT NULL DEFAULT 0,
  status INTEGER NOT NULL,
  latency_ms INTEGER NOT NULL DEFAULT 0,
  prompt_tokens INTEGER,
  completion_tokens INTEGER,
  error TEXT
);
CREATE INDEX IF NOT EXISTS idx_request_logs_created ON request_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_client ON request_logs(client_key_id);
CREATE INDEX IF NOT EXISTS idx_request_logs_route ON request_logs(route_key_id);
"#;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m20260811_000001_initial"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.get_connection().execute_unprepared(SQL).await?;
            Ok(())
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Err(DbErr::Migration(
                "initial migration cannot be rolled back".into(),
            ))
        }
    }
}

mod m20260811_000002_add_api_key_proxy {
    use sea_orm_migration::prelude::*;
    use sea_orm_migration::sea_orm::{DbBackend, Statement};

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m20260811_000002_add_api_key_proxy"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            let connection = manager.get_connection();
            let columns = connection
                .query_all(Statement::from_string(
                    DbBackend::Sqlite,
                    "PRAGMA table_info('api_keys')".to_owned(),
                ))
                .await?;
            let has_proxy_id = columns
                .iter()
                .any(|row| row.try_get::<String>("", "name").as_deref() == Ok("proxy_id"));
            if !has_proxy_id {
                connection
                    .execute_unprepared("ALTER TABLE api_keys ADD COLUMN proxy_id TEXT")
                    .await?;
            }
            connection
                .execute_unprepared(
                    "CREATE INDEX IF NOT EXISTS idx_keys_proxy ON api_keys(proxy_id)",
                )
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared("DROP INDEX IF EXISTS idx_keys_proxy")
                .await?;
            Ok(())
        }
    }
}

mod m20260811_000003_add_first_token_timing {
    use sea_orm_migration::prelude::*;

    pub struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            "m20260811_000003_add_first_token_timing"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE request_logs ADD COLUMN first_token_ms INTEGER")
                .await?;
            Ok(())
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE request_logs DROP COLUMN first_token_ms")
                .await?;
            Ok(())
        }
    }
}

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260811_000001_initial::Migration),
            Box::new(m20260811_000002_add_api_key_proxy::Migration),
            Box::new(m20260811_000003_add_first_token_timing::Migration),
        ]
    }
}

pub async fn run(path: &Path) -> Result<(), ApiError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
    let mut options = ConnectOptions::new(url);
    options.sqlx_logging(false);
    let connection = Database::connect(options)
        .await
        .map_err(|error| ApiError::Internal(format!("database connection: {error}")))?;
    Migrator::up(&connection, None)
        .await
        .map_err(|error| ApiError::Internal(format!("database migration: {error}")))?;
    connection
        .close()
        .await
        .map_err(|error| ApiError::Internal(format!("database close: {error}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rusqlite::Connection;
    use uuid::Uuid;

    fn temporary_database() -> PathBuf {
        std::env::temp_dir().join(format!("opencode2api-migration-{}.db", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn creates_a_fresh_database() {
        let path = temporary_database();
        super::run(&path).await.unwrap();

        let connection = Connection::open(&path).unwrap();
        let applied: i64 = connection
            .query_row("SELECT COUNT(*) FROM seaql_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let has_proxy_id: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('api_keys') WHERE name = 'proxy_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(applied, 3);
        assert_eq!(has_proxy_id, 1);

        drop(connection);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn upgrades_an_existing_database_without_losing_data() {
        let path = temporary_database();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE api_keys (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    base_url TEXT NOT NULL,
                    api_key_enc TEXT NOT NULL,
                    tags TEXT NOT NULL DEFAULT '[]',
                    notes TEXT NOT NULL DEFAULT '',
                    model_cache TEXT NOT NULL DEFAULT '[]',
                    is_default INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                INSERT INTO api_keys
                    (id, name, base_url, api_key_enc, created_at, updated_at)
                VALUES ('existing', 'Existing', 'https://example.com', 'secret', 1, 1);",
            )
            .unwrap();
        drop(connection);

        super::run(&path).await.unwrap();

        let connection = Connection::open(&path).unwrap();
        let name: String = connection
            .query_row(
                "SELECT name FROM api_keys WHERE id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let has_proxy_id: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('api_keys') WHERE name = 'proxy_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Existing");
        assert_eq!(has_proxy_id, 1);

        drop(connection);
        std::fs::remove_file(path).unwrap();
    }
}
