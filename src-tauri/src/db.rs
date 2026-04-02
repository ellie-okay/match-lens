use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Match {
    pub id: i64,
    pub game_id: String,
    pub summoner_name: String,
    pub champion: String,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub result: String,
    pub duration_sec: i32,
    pub recorded_at: String,
    pub video_path: String,
    pub file_size_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub match_id: i64,
    pub event_type: String,
    pub timestamp_sec: f64,
    pub is_local_player: bool,
    pub raw_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSample {
    pub id: i64,
    pub match_id: i64,
    pub timestamp_sec: f64,
    pub target: String,
    pub rtt_ms: Option<f64>,
    pub timed_out: bool,
    pub error: Option<String>,
}

pub fn db_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
    base.join("lol-review").join("recordings.db")
}

pub fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;
    migrate_schema(&conn);
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS matches (
            id              INTEGER PRIMARY KEY,
            game_id         TEXT UNIQUE,
            summoner_name   TEXT NOT NULL DEFAULT '',
            champion        TEXT NOT NULL DEFAULT '',
            kills           INTEGER NOT NULL DEFAULT 0,
            deaths          INTEGER NOT NULL DEFAULT 0,
            assists         INTEGER NOT NULL DEFAULT 0,
            result          TEXT NOT NULL DEFAULT 'Unknown',
            duration_sec    INTEGER NOT NULL DEFAULT 0,
            recorded_at     TEXT NOT NULL,
            video_path      TEXT NOT NULL,
            file_size_bytes INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS events (
            id              INTEGER PRIMARY KEY,
            match_id        INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
            event_type      TEXT NOT NULL,
            timestamp_sec   REAL NOT NULL,
            is_local_player INTEGER NOT NULL DEFAULT 0,
            raw_data        TEXT NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS network_samples (
            id              INTEGER PRIMARY KEY,
            match_id        INTEGER NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
            timestamp_sec   REAL NOT NULL,
            target          TEXT NOT NULL,
            rtt_ms          REAL,
            timed_out       INTEGER NOT NULL DEFAULT 0,
            error           TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_events_match ON events(match_id);
        CREATE INDEX IF NOT EXISTS idx_network_samples_match ON network_samples(match_id);
        CREATE INDEX IF NOT EXISTS idx_matches_recorded ON matches(recorded_at DESC);
        ",
    )?;
    Ok(())
}

fn migrate_schema(conn: &Connection) {
    // Add summoner_name column to existing databases that predate this field.
    // SQLite doesn't support IF NOT EXISTS on ALTER TABLE, so we swallow the
    // "duplicate column name" error that fires when the column already exists.
    let _ = conn.execute_batch(
        "ALTER TABLE matches ADD COLUMN summoner_name TEXT NOT NULL DEFAULT '';",
    );
}

pub fn insert_match(conn: &Connection, m: &Match) -> Result<i64> {
    conn.execute(
        "INSERT OR REPLACE INTO matches
         (game_id, summoner_name, champion, kills, deaths, assists, result, duration_sec, recorded_at, video_path, file_size_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            m.game_id, m.summoner_name, m.champion, m.kills, m.deaths, m.assists,
            m.result, m.duration_sec, m.recorded_at, m.video_path, m.file_size_bytes
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_event(conn: &Connection, e: &Event) -> Result<()> {
    conn.execute(
        "INSERT INTO events (match_id, event_type, timestamp_sec, is_local_player, raw_data)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            e.match_id,
            e.event_type,
            e.timestamp_sec,
            e.is_local_player as i32,
            e.raw_data
        ],
    )?;
    Ok(())
}

pub fn insert_network_sample(conn: &Connection, sample: &NetworkSample) -> Result<()> {
    conn.execute(
        "INSERT INTO network_samples (match_id, timestamp_sec, target, rtt_ms, timed_out, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            sample.match_id,
            sample.timestamp_sec,
            sample.target,
            sample.rtt_ms,
            sample.timed_out as i32,
            sample.error
        ],
    )?;
    Ok(())
}

pub fn get_matches(conn: &Connection) -> Result<Vec<Match>> {
    let mut stmt = conn.prepare(
        "SELECT id, game_id, summoner_name, champion, kills, deaths, assists, result, duration_sec,
                recorded_at, video_path, file_size_bytes
         FROM matches ORDER BY recorded_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Match {
            id: row.get(0)?,
            game_id: row.get(1)?,
            summoner_name: row.get(2)?,
            champion: row.get(3)?,
            kills: row.get(4)?,
            deaths: row.get(5)?,
            assists: row.get(6)?,
            result: row.get(7)?,
            duration_sec: row.get(8)?,
            recorded_at: row.get(9)?,
            video_path: row.get(10)?,
            file_size_bytes: row.get(11)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_events(conn: &Connection, match_id: i64) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, match_id, event_type, timestamp_sec, is_local_player, raw_data
         FROM events WHERE match_id = ?1 ORDER BY timestamp_sec",
    )?;
    let rows = stmt.query_map([match_id], |row| {
        Ok(Event {
            id: row.get(0)?,
            match_id: row.get(1)?,
            event_type: row.get(2)?,
            timestamp_sec: row.get(3)?,
            is_local_player: row.get::<_, i32>(4)? != 0,
            raw_data: row.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_network_samples(conn: &Connection, match_id: i64) -> Result<Vec<NetworkSample>> {
    let mut stmt = conn.prepare(
        "SELECT id, match_id, timestamp_sec, target, rtt_ms, timed_out, error
         FROM network_samples WHERE match_id = ?1 ORDER BY timestamp_sec",
    )?;
    let rows = stmt.query_map([match_id], |row| {
        Ok(NetworkSample {
            id: row.get(0)?,
            match_id: row.get(1)?,
            timestamp_sec: row.get(2)?,
            target: row.get(3)?,
            rtt_ms: row.get(4)?,
            timed_out: row.get::<_, i32>(5)? != 0,
            error: row.get(6)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_oldest_match(conn: &Connection) -> Result<Option<Match>> {
    let mut stmt = conn.prepare(
        "SELECT id, game_id, summoner_name, champion, kills, deaths, assists, result, duration_sec,
                recorded_at, video_path, file_size_bytes
         FROM matches ORDER BY recorded_at ASC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], |row| {
        Ok(Match {
            id: row.get(0)?,
            game_id: row.get(1)?,
            summoner_name: row.get(2)?,
            champion: row.get(3)?,
            kills: row.get(4)?,
            deaths: row.get(5)?,
            assists: row.get(6)?,
            result: row.get(7)?,
            duration_sec: row.get(8)?,
            recorded_at: row.get(9)?,
            video_path: row.get(10)?,
            file_size_bytes: row.get(11)?,
        })
    })?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub fn delete_match(conn: &Connection, match_id: i64) -> Result<()> {
    conn.execute("DELETE FROM matches WHERE id = ?1", [match_id])?;
    Ok(())
}

#[allow(dead_code)]
pub fn update_file_size(conn: &Connection, match_id: i64, bytes: i64) -> Result<()> {
    conn.execute(
        "UPDATE matches SET file_size_bytes = ?1 WHERE id = ?2",
        params![bytes, match_id],
    )?;
    Ok(())
}
