use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Initialize database connection and create schema
    pub async fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_url = format!("sqlite://{}?mode=rwc", db_path.as_ref().display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        let db = Self { pool };
        db.create_schema().await?;

        Ok(db)
    }

    /// Create database schema if it doesn't exist
    async fn create_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS video_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                device_name TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS frames (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                video_chunk_id INTEGER NOT NULL,
                offset_index INTEGER NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                device_name TEXT,
                FOREIGN KEY (video_chunk_id) REFERENCES video_chunks(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_frames_video_chunk_id
            ON frames(video_chunk_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert a new video chunk and return its ID
    pub async fn insert_video_chunk(&self, file_path: &str, device_name: &str) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO video_chunks (file_path, device_name) VALUES (?1, ?2)",
        )
        .bind(file_path)
        .bind(device_name)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Insert a frame with auto-incrementing offset_index
    pub async fn insert_frame(
        &self,
        device_name: &str,
        timestamp: Option<DateTime<Utc>>,
    ) -> Result<i64> {
        let mut tx = self.pool.begin().await?;

        // 1. Get latest video chunk for this device
        let video_chunk_id: i64 = sqlx::query_scalar(
            "SELECT id FROM video_chunks
             WHERE device_name = ?1
             ORDER BY id DESC LIMIT 1",
        )
        .bind(device_name)
        .fetch_one(&mut *tx)
        .await?;

        // 2. Calculate next offset_index (KEY LOGIC)
        let offset_index: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(offset_index), -1) + 1
             FROM frames
             WHERE video_chunk_id = ?1",
        )
        .bind(video_chunk_id)
        .fetch_one(&mut *tx)
        .await?;

        // 3. Insert frame with auto-incremented offset_index
        let result = sqlx::query(
            "INSERT INTO frames (video_chunk_id, offset_index, timestamp, device_name)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(video_chunk_id)
        .bind(offset_index)
        .bind(timestamp.unwrap_or_else(Utc::now))
        .bind(device_name)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(result.last_insert_rowid())
    }

    /// Get frame information by frame ID
    pub async fn get_frame(&self, frame_id: i64) -> Result<FrameInfo> {
        let row = sqlx::query_as::<_, FrameInfo>(
            r#"
            SELECT
                f.id,
                f.video_chunk_id,
                f.offset_index,
                f.timestamp,
                f.device_name,
                vc.file_path
            FROM frames f
            JOIN video_chunks vc ON f.video_chunk_id = vc.id
            WHERE f.id = ?1
            "#,
        )
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get all frames for a specific video chunk
    pub async fn get_frames_by_chunk(&self, video_chunk_id: i64) -> Result<Vec<FrameInfo>> {
        let rows = sqlx::query_as::<_, FrameInfo>(
            r#"
            SELECT
                f.id,
                f.video_chunk_id,
                f.offset_index,
                f.timestamp,
                f.device_name,
                vc.file_path
            FROM frames f
            JOIN video_chunks vc ON f.video_chunk_id = vc.id
            WHERE f.video_chunk_id = ?1
            ORDER BY f.offset_index ASC
            "#,
        )
        .bind(video_chunk_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get the current video chunk ID for a device
    pub async fn get_current_chunk_id(&self, device_name: &str) -> Result<Option<i64>> {
        let id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM video_chunks
             WHERE device_name = ?1
             ORDER BY id DESC LIMIT 1",
        )
        .bind(device_name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(id)
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct FrameInfo {
    pub id: i64,
    pub video_chunk_id: i64,
    pub offset_index: i64,
    pub timestamp: DateTime<Utc>,
    pub device_name: Option<String>,
    pub file_path: String,
}
