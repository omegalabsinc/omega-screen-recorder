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
                recording_type TEXT,
                task_id TEXT,
                chunk_index INTEGER,
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
                is_keyframe INTEGER DEFAULT 0,
                pts INTEGER,
                dts INTEGER,
                display_index INTEGER DEFAULT 0,
                display_width INTEGER,
                display_height INTEGER,
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

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_frames_keyframe
            ON frames(is_keyframe)
            WHERE is_keyframe = 1
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: Add display columns if they don't exist
        // SQLite doesn't support "IF NOT EXISTS" for ALTER TABLE, so we check first
        // PRAGMA table_info returns: (cid, name, type, notnull, dflt_value, pk)
        let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(frames)")
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

        let column_names: Vec<String> = columns.iter().map(|(_, name, _, _, _, _)| name.clone()).collect();
        log::debug!("Existing frames table columns: {:?}", column_names);

        if !column_names.contains(&"display_index".to_string()) {
            log::info!("Adding display_index column to frames table");
            sqlx::query("ALTER TABLE frames ADD COLUMN display_index INTEGER DEFAULT 0")
                .execute(&self.pool)
                .await?;
        }

        if !column_names.contains(&"display_width".to_string()) {
            log::info!("Adding display_width column to frames table");
            sqlx::query("ALTER TABLE frames ADD COLUMN display_width INTEGER")
                .execute(&self.pool)
                .await?;
        }

        if !column_names.contains(&"display_height".to_string()) {
            log::info!("Adding display_height column to frames table");
            sqlx::query("ALTER TABLE frames ADD COLUMN display_height INTEGER")
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Insert a new video chunk and return its ID
    pub async fn insert_video_chunk(
        &self,
        file_path: &str,
        device_name: &str,
        recording_type: Option<&str>,
        task_id: Option<&str>,
        chunk_index: Option<i64>,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO video_chunks (file_path, device_name, recording_type, task_id, chunk_index) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(file_path)
        .bind(device_name)
        .bind(recording_type)
        .bind(task_id)
        .bind(chunk_index)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Insert a frame with auto-incrementing offset_index
    pub async fn insert_frame(
        &self,
        device_name: &str,
        timestamp: Option<DateTime<Utc>>,
        is_keyframe: bool,
        pts: Option<i64>,
        dts: Option<i64>,
        display_index: Option<i64>,
        display_width: Option<i64>,
        display_height: Option<i64>,
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
            "INSERT INTO frames (video_chunk_id, offset_index, timestamp, device_name, is_keyframe, pts, dts, display_index, display_width, display_height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(video_chunk_id)
        .bind(offset_index)
        .bind(timestamp.unwrap_or_else(Utc::now))
        .bind(device_name)
        .bind(is_keyframe as i32)
        .bind(pts)
        .bind(dts)
        .bind(display_index)
        .bind(display_width)
        .bind(display_height)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(result.last_insert_rowid())
    }

    /// Get frame information by frame ID
    #[allow(dead_code)]
    pub async fn get_frame(&self, frame_id: i64) -> Result<FrameInfo> {
        let row = sqlx::query_as::<_, FrameInfo>(
            r#"
            SELECT
                f.id,
                f.video_chunk_id,
                f.offset_index,
                f.timestamp,
                f.device_name,
                vc.file_path,
                f.is_keyframe,
                f.pts,
                f.dts,
                f.display_index,
                f.display_width,
                f.display_height
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
    #[allow(dead_code)]
    pub async fn get_frames_by_chunk(&self, video_chunk_id: i64) -> Result<Vec<FrameInfo>> {
        let rows = sqlx::query_as::<_, FrameInfo>(
            r#"
            SELECT
                f.id,
                f.video_chunk_id,
                f.offset_index,
                f.timestamp,
                f.device_name,
                vc.file_path,
                f.is_keyframe,
                f.pts,
                f.dts,
                f.display_index,
                f.display_width,
                f.display_height
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
    #[allow(dead_code)]
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

    /// Get all video chunks for a specific task_id, ordered by created_at
    pub async fn get_chunks_by_task_id(&self, task_id: &str) -> Result<Vec<VideoChunkInfo>> {
        let rows = sqlx::query_as::<_, VideoChunkInfo>(
            r#"
            SELECT id, file_path, device_name, recording_type, task_id, chunk_index, created_at
            FROM video_chunks
            WHERE task_id = ?1
            ORDER BY created_at ASC
            "#,
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a video chunk by ID (also deletes associated frames due to foreign key cascade)
    pub async fn delete_chunk(&self, chunk_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM video_chunks
            WHERE id = ?1
            "#,
        )
        .bind(chunk_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all frames for a specific task_id across all chunks
    pub async fn get_frames_by_task_id(&self, task_id: &str) -> Result<Vec<FrameInfo>> {
        let rows = sqlx::query_as::<_, FrameInfo>(
            r#"
            SELECT
                f.id,
                f.video_chunk_id,
                f.offset_index,
                f.timestamp,
                f.device_name,
                vc.file_path,
                f.is_keyframe,
                f.pts,
                f.dts,
                f.display_index,
                f.display_width,
                f.display_height
            FROM frames f
            JOIN video_chunks vc ON f.video_chunk_id = vc.id
            WHERE vc.task_id = ?1
            ORDER BY vc.created_at ASC, f.offset_index ASC
            "#,
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct FrameInfo {
    pub id: i64,
    pub video_chunk_id: i64,
    pub offset_index: i64,
    pub timestamp: DateTime<Utc>,
    pub device_name: Option<String>,
    pub file_path: String,
    pub is_keyframe: i32,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub display_index: Option<i64>,
    pub display_width: Option<i64>,
    pub display_height: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct VideoChunkInfo {
    pub id: i64,
    pub file_path: String,
    pub device_name: String,
    pub recording_type: Option<String>,
    pub task_id: Option<String>,
    pub chunk_index: Option<i64>,
    pub created_at: DateTime<Utc>,
}
