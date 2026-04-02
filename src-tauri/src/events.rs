use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEvent {
    pub event_type: String, // Kill, Death, Dragon, Baron, Herald, Turret, Inhibitor, Assist
    pub timestamp_sec: f64,
    pub is_local_player: bool,
    pub raw_data: String,
}

#[derive(Default)]
struct LocalPlayerAliases {
    names: HashSet<String>,
}

impl LocalPlayerAliases {
    fn from_seed(seed: &str) -> Self {
        let mut aliases = Self::default();
        aliases.add(seed);
        aliases
    }

    fn add(&mut self, value: &str) {
        for alias in expand_name_aliases(value) {
            self.names.insert(alias);
        }
    }

    fn contains(&self, value: &str) -> bool {
        expand_name_aliases(value)
            .into_iter()
            .any(|alias| self.names.contains(&alias))
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn len(&self) -> usize {
        self.names.len()
    }
}

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const ACTIVE_PLAYER_ENDPOINT: &str = "https://127.0.0.1:2999/liveclientdata/activeplayer";
const ACTIVE_PLAYER_NAME_ENDPOINT: &str = "https://127.0.0.1:2999/liveclientdata/activeplayername";
const PLAYER_LIST_ENDPOINT: &str = "https://127.0.0.1:2999/liveclientdata/playerlist";
const EVENT_DATA_ENDPOINT: &str = "https://127.0.0.1:2999/liveclientdata/eventdata";
const ALL_GAME_DATA_ENDPOINT: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";

/// Poll the Live Game API for events during an active game.
/// Stops when `stop_flag` is set to true.
pub async fn poll_events(
    summoner_name: String,
    stop_flag: Arc<Mutex<bool>>,
    recording_started_at: Instant,
) -> Vec<GameEvent> {
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let mut local_aliases = LocalPlayerAliases::from_seed(&summoner_name);
    let mut collected: Vec<GameEvent> = Vec::new();
    let mut seen_event_ids: HashSet<u64> = HashSet::new();
    let mut timeline_offset_sec = 0.0;
    let mut logged_timeline_offset_sec: Option<i64> = None;

    loop {
        if let Err(e) = refresh_local_player_aliases(&client, &mut local_aliases).await {
            warn!("Could not refresh local player aliases: {e}");
        }

        if let Ok(game_time_sec) = fetch_current_game_time(&client).await {
            let candidate_offset_sec =
                (recording_started_at.elapsed().as_secs_f64() - game_time_sec).max(0.0);
            timeline_offset_sec = candidate_offset_sec;

            let rounded = candidate_offset_sec.round() as i64;
            if logged_timeline_offset_sec != Some(rounded) {
                debug!(
                    "Timeline offset: recording leads game clock by {:.1}s",
                    candidate_offset_sec
                );
                logged_timeline_offset_sec = Some(rounded);
            }
        }

        if *stop_flag.lock().await {
            let _ = fetch_events_once(
                &client,
                &local_aliases,
                &mut seen_event_ids,
                timeline_offset_sec,
            )
                .await
                .map(|new_events| {
                    if !new_events.is_empty() {
                        info!("Collected {} final game events", new_events.len());
                        collected.extend(new_events);
                    }
                });
            break;
        }

        match fetch_events_once(
            &client,
            &local_aliases,
            &mut seen_event_ids,
            timeline_offset_sec,
        )
        .await
        {
            Ok(new_events) => {
                if !new_events.is_empty() {
                    info!("Collected {} new game events", new_events.len());
                    collected.extend(new_events);
                }
            }
            Err(e) => {
                debug!("Event poll error (expected during game start/end): {e}");
            }
        }

        sleep(POLL_INTERVAL).await;
    }

    info!("Event polling stopped. Total events: {}", collected.len());
    collected
}

async fn refresh_local_player_aliases(
    client: &Client,
    aliases: &mut LocalPlayerAliases,
) -> Result<()> {
    let before = aliases.len();

    if let Ok(value) = fetch_json(client, ACTIVE_PLAYER_ENDPOINT).await {
        add_alias_value(aliases, &value, "summonerName");
        add_alias_value(aliases, &value, "riotId");
        add_alias_value(aliases, &value, "riotIdGameName");
        add_alias_value(aliases, &value, "gameName");
        add_alias_value(aliases, &value, "displayName");
    }

    if let Ok(value) = fetch_text_json(client, ACTIVE_PLAYER_NAME_ENDPOINT).await {
        if let Some(name) = value.as_str() {
            aliases.add(name);
        }
    }

    if let Ok(value) = fetch_json(client, PLAYER_LIST_ENDPOINT).await {
        if let Some(players) = value.as_array() {
            for player in players {
                let mut matched_existing = aliases.is_empty();

                for field in [
                    "summonerName",
                    "riotId",
                    "riotIdGameName",
                    "gameName",
                    "displayName",
                    "championName",
                ] {
                    if let Some(name) = player.get(field).and_then(Value::as_str) {
                        if aliases.contains(name) {
                            matched_existing = true;
                        }
                    }
                }

                if matched_existing {
                    add_alias_value(aliases, player, "summonerName");
                    add_alias_value(aliases, player, "riotId");
                    add_alias_value(aliases, player, "riotIdGameName");
                    add_alias_value(aliases, player, "gameName");
                    add_alias_value(aliases, player, "displayName");
                    add_alias_value(aliases, player, "championName");
                }
            }
        }
    }

    if aliases.len() > before {
        info!("Resolved {} local player aliases", aliases.len());
    }

    Ok(())
}

async fn fetch_events_once(
    client: &Client,
    local_aliases: &LocalPlayerAliases,
    seen_ids: &mut HashSet<u64>,
    timeline_offset_sec: f64,
) -> Result<Vec<GameEvent>> {
    #[derive(Deserialize)]
    struct EventDataResponse {
        #[serde(rename = "Events")]
        events: Vec<Value>,
    }

    let body = fetch_text(client, EVENT_DATA_ENDPOINT).await?;
    let data: EventDataResponse = serde_json::from_str(&body).with_context(|| {
        format!(
            "error decoding eventdata response body: {}",
            body_preview(&body)
        )
    })?;

    let mut new_events = Vec::new();

    for event in &data.events {
        let event_id = event["EventID"].as_u64().unwrap_or(0);
        if seen_ids.contains(&event_id) {
            continue;
        }

        let event_name = event["EventName"].as_str().unwrap_or("");
        let timestamp = event["EventTime"].as_f64().unwrap_or(0.0);

        if let Some(parsed) = parse_event(
            event_name,
            timestamp + timeline_offset_sec,
            local_aliases,
            event,
        ) {
            seen_ids.insert(event_id);
            new_events.push(parsed);
        } else if should_mark_seen_without_match(event_name) {
            seen_ids.insert(event_id);
        }
    }

    Ok(new_events)
}

fn should_mark_seen_without_match(event_name: &str) -> bool {
    !matches!(event_name, "ChampionKill")
}

fn parse_event(
    event_name: &str,
    timestamp: f64,
    local_aliases: &LocalPlayerAliases,
    raw: &Value,
) -> Option<GameEvent> {
    let raw_str = raw.to_string();

    match event_name {
        "ChampionKill" => {
            let killer = raw["KillerName"].as_str().unwrap_or("");
            let victim = raw["VictimName"].as_str().unwrap_or("");
            let assisters = raw["Assisters"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();

            let local_is_killer = local_aliases.contains(killer);
            let local_is_victim = local_aliases.contains(victim);
            let local_assisted = assisters.iter().any(|a| local_aliases.contains(a));

            if local_is_killer {
                Some(GameEvent {
                    event_type: "Kill".to_string(),
                    timestamp_sec: timestamp,
                    is_local_player: true,
                    raw_data: raw_str,
                })
            } else if local_is_victim {
                Some(GameEvent {
                    event_type: "Death".to_string(),
                    timestamp_sec: timestamp,
                    is_local_player: true,
                    raw_data: raw_str,
                })
            } else if local_assisted {
                Some(GameEvent {
                    event_type: "Assist".to_string(),
                    timestamp_sec: timestamp,
                    is_local_player: true,
                    raw_data: raw_str,
                })
            } else {
                None
            }
        }
        "DragonKill" => Some(GameEvent {
            event_type: "Dragon".to_string(),
            timestamp_sec: timestamp,
            is_local_player: false,
            raw_data: raw_str,
        }),
        "BaronKill" => Some(GameEvent {
            event_type: "Baron".to_string(),
            timestamp_sec: timestamp,
            is_local_player: false,
            raw_data: raw_str,
        }),
        "HeraldKill" => Some(GameEvent {
            event_type: "Herald".to_string(),
            timestamp_sec: timestamp,
            is_local_player: false,
            raw_data: raw_str,
        }),
        "TurretKilled" => Some(GameEvent {
            event_type: "Turret".to_string(),
            timestamp_sec: timestamp,
            is_local_player: false,
            raw_data: raw_str,
        }),
        "InhibKilled" | "InhibitorKilled" => Some(GameEvent {
            event_type: "Inhibitor".to_string(),
            timestamp_sec: timestamp,
            is_local_player: false,
            raw_data: raw_str,
        }),
        _ => None,
    }
}

async fn fetch_json(client: &Client, url: &str) -> Result<Value> {
    let body = fetch_text(client, url).await?;
    serde_json::from_str(&body).with_context(|| {
        format!(
            "error decoding JSON response from {url}: {}",
            body_preview(&body)
        )
    })
}

async fn fetch_current_game_time(client: &Client) -> Result<f64> {
    let body = fetch_text(client, ALL_GAME_DATA_ENDPOINT).await?;
    let json: Value = serde_json::from_str(&body).with_context(|| {
        format!(
            "error decoding allgamedata response body: {}",
            body_preview(&body)
        )
    })?;

    json["gameData"]["gameTime"]
        .as_f64()
        .ok_or_else(|| anyhow!("allgamedata missing gameData.gameTime"))
}

async fn fetch_text_json(client: &Client, url: &str) -> Result<Value> {
    let body = fetch_text(client, url).await?;
    serde_json::from_str(&body).with_context(|| {
        format!(
            "error decoding JSON string response from {url}: {}",
            body_preview(&body)
        )
    })
}

async fn fetch_text(client: &Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "HTTP {} from {}: {}",
            status,
            url,
            body_preview(&body)
        ));
    }

    Ok(body)
}

fn add_alias_value(aliases: &mut LocalPlayerAliases, value: &Value, field: &str) {
    if let Some(text) = value.get(field).and_then(Value::as_str) {
        aliases.add(text);
    }
}

fn expand_name_aliases(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut aliases = HashSet::new();
    let normalized = normalize_name(trimmed);
    aliases.insert(normalized);

    if let Some((game_name, _tag)) = trimmed.split_once('#') {
        aliases.insert(normalize_name(game_name));
    }

    aliases.into_iter().collect()
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn body_preview(body: &str) -> String {
    let single_line = body.replace('\n', " ").replace('\r', " ");
    let preview: String = single_line.chars().take(180).collect();
    if single_line.len() > preview.len() {
        format!("{preview}...")
    } else {
        preview
    }
}
