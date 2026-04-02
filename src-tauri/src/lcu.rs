use anyhow::{anyhow, Result};
use base64::Engine as _;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct LcuCredentials {
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GamePhase {
    None,
    Lobby,
    Matchmaking,
    ReadyCheck,
    ChampSelect,
    GameStart,
    InProgress,
    WaitingForStats,
    EndOfGame,
    PreEndOfGame,
    Reconnect,
    Unknown(String),
}

impl From<&str> for GamePhase {
    fn from(s: &str) -> Self {
        match s {
            "None" => GamePhase::None,
            "Lobby" => GamePhase::Lobby,
            "Matchmaking" => GamePhase::Matchmaking,
            "ReadyCheck" => GamePhase::ReadyCheck,
            "ChampSelect" => GamePhase::ChampSelect,
            "GameStart" => GamePhase::GameStart,
            "InProgress" => GamePhase::InProgress,
            "WaitingForStats" => GamePhase::WaitingForStats,
            "EndOfGame" => GamePhase::EndOfGame,
            "PreEndOfGame" => GamePhase::PreEndOfGame,
            "Reconnect" => GamePhase::Reconnect,
            other => GamePhase::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    pub champion: String,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub result: String, // "Win" | "Loss" | "Remake"
    pub duration_sec: i32,
    pub game_id: String,
}

#[derive(Debug, Clone)]
struct CurrentSummonerIdentity {
    puuid: String,
    summoner_id: i64,
    aliases: HashSet<String>,
}

/// Read the League Client (LCU) lockfile to discover port and auth token.
/// The LCU lockfile lives in the League install directory, NOT the Riot Client config dir.
/// Format: LeagueClient:{pid}:{port}:{password}:{protocol}
pub fn read_lockfile() -> Result<LcuCredentials> {
    for path in league_lockfile_candidates() {
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let trimmed = contents.trim();
        // Must start with "LeagueClient" — ignore Riot Client lockfiles
        if !trimmed.starts_with("LeagueClient") {
            continue;
        }
        let parts: Vec<&str> = trimmed.splitn(5, ':').collect();
        if parts.len() < 5 {
            continue;
        }
        let port: u16 = parts[2]
            .parse()
            .map_err(|e| anyhow!("Bad port in lockfile: {e}"))?;
        let password = parts[3].to_string();
        return Ok(LcuCredentials { port, password });
    }
    Err(anyhow!("League client is not running (lockfile not found)"))
}

/// Return candidate paths to check for the League Client lockfile, in priority order.
fn league_lockfile_candidates() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();

    // 1. Registry: HKLM\SOFTWARE\WOW6432Node\Riot Games\League of Legends → Location
    if let Some(p) = league_install_from_registry() {
        paths.push(std::path::PathBuf::from(&p).join("lockfile"));
    }

    // 2. Common default install locations
    for root in &[
        r"C:\Riot Games",
        r"D:\Riot Games",
        r"C:\Program Files\Riot Games",
    ] {
        paths.push(
            std::path::PathBuf::from(root)
                .join("League of Legends")
                .join("lockfile"),
        );
    }

    paths
}

fn league_install_from_registry() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key_path in &[
        r"SOFTWARE\WOW6432Node\Riot Games\League of Legends",
        r"SOFTWARE\Riot Games\League of Legends",
    ] {
        if let Ok(key) = hklm.open_subkey(key_path) {
            if let Ok(loc) = key.get_value::<String, _>("Location") {
                return Some(loc);
            }
        }
    }
    None
}

/// Build a reqwest client that skips TLS verification (LCU uses self-signed cert).
pub fn build_lcu_client() -> Result<Client> {
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(10))
        .build()?;
    Ok(client)
}

/// Poll for LCU to become available, retrying until lockfile exists.
pub async fn wait_for_lcu() -> LcuCredentials {
    loop {
        match read_lockfile() {
            Ok(creds) => {
                info!("LCU found on port {}", creds.port);
                return creds;
            }
            Err(_) => {
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Poll the LCU REST API every 3 seconds for gameflow phase changes.
/// More reliable than WebSocket — no SSL handshake issues, simpler format.
pub async fn watch_game_phase(tx: mpsc::Sender<GamePhase>) {
    loop {
        // Wait until the LCU is running
        let creds = wait_for_lcu().await;

        let client = match build_lcu_client() {
            Ok(c) => c,
            Err(e) => {
                warn!("LCU client build failed: {e}");
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let url = format!(
            "https://127.0.0.1:{}/lol-gameflow/v1/gameflow-phase",
            creds.port
        );
        let auth =
            base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", creds.password));

        info!("LCU polling started on port {}", creds.port);
        let mut last_phase = GamePhase::None;

        loop {
            // Stop polling if LCU closes (lockfile disappears)
            if read_lockfile().is_err() {
                info!("LCU lockfile gone, waiting for restart");
                break;
            }

            match client
                .get(&url)
                .header("Authorization", format!("Basic {}", auth))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(text) = resp.text().await {
                        // LCU returns a JSON-encoded string: "\"InProgress\""
                        let phase_str = text.trim().trim_matches('"');
                        let phase = GamePhase::from(phase_str);
                        if phase != last_phase {
                            info!("Phase: {:?} → {:?}", last_phase, phase);
                            last_phase = phase.clone();
                            if tx.send(phase).await.is_err() {
                                return; // receiver dropped, app is shutting down
                            }
                        }
                    }
                }
                Ok(resp) => {
                    warn!("LCU phase poll HTTP {}", resp.status());
                }
                Err(e) => {
                    warn!("LCU phase poll error: {e}");
                    sleep(Duration::from_secs(5)).await;
                    break; // re-discover LCU
                }
            }

            sleep(Duration::from_secs(3)).await;
        }
    }
}

/// Fetch match summary from LCU after game ends.
/// Returns data for the most recent completed game.
pub async fn fetch_match_summary(
    client: &Client,
    creds: &LcuCredentials,
    summoner_name: &str,
) -> Result<MatchSummary> {
    let current_summoner = fetch_current_summoner_identity(client, creds, summoner_name).await?;
    let url = format!(
        "https://127.0.0.1:{}/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=0&endIndex=5",
        creds.port
    );
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", creds.password));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Basic {}", auth))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("LCU match history returned {}", resp.status()));
    }

    let json: Value = resp.json().await?;
    let games = json["games"]["games"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for game in games {
        if let Some(summary) = extract_match_summary(&game, &current_summoner) {
            return Ok(summary);
        }
    }

    Err(anyhow!("Could not resolve local player in recent match history payload"))
}

fn extract_match_summary(game: &Value, current_summoner: &CurrentSummonerIdentity) -> Option<MatchSummary> {
    let game_id = game["gameId"].as_i64()?.to_string();
    let duration_sec = game["gameDuration"].as_i64().unwrap_or(0) as i32;
    let participants = game["participantIdentities"].as_array()?;
    let our_id = participants.iter().find_map(|participant| {
        let player = &participant["player"];
        if participant_matches_current_summoner(player, current_summoner) {
            participant["participantId"].as_i64()
        } else {
            None
        }
    })?;

    let all_participants = game["participants"].as_array()?;
    let our_participant = all_participants
        .iter()
        .find(|participant| participant["participantId"].as_i64() == Some(our_id))?;

    let stats = &our_participant["stats"];
    let kills = stats["kills"].as_i64().unwrap_or(0) as i32;
    let deaths = stats["deaths"].as_i64().unwrap_or(0) as i32;
    let assists = stats["assists"].as_i64().unwrap_or(0) as i32;

    let win = stats["win"]
        .as_bool()
        .or_else(|| team_win_for_participant(game, our_participant))
        .unwrap_or(false);
    let result = if win { "Win" } else { "Loss" }.to_string();

    let champion_id = our_participant["championId"].as_i64().unwrap_or(0);
    let champion = champion_name_from_id(champion_id);

    Some(MatchSummary {
        champion,
        kills,
        deaths,
        assists,
        result,
        duration_sec,
        game_id,
    })
}

fn participant_matches_current_summoner(
    player: &Value,
    current_summoner: &CurrentSummonerIdentity,
) -> bool {
    if player["puuid"]
        .as_str()
        .is_some_and(|puuid| puuid.eq_ignore_ascii_case(&current_summoner.puuid))
    {
        return true;
    }

    if player["summonerId"].as_i64() == Some(current_summoner.summoner_id) {
        return true;
    }

    participant_aliases(player)
        .into_iter()
        .any(|alias| current_summoner.aliases.contains(&alias))
}

fn team_win_for_participant(game: &Value, participant: &Value) -> Option<bool> {
    let team_id = participant["teamId"].as_i64()?;
    game["teams"].as_array()?.iter().find_map(|team| {
        if team["teamId"].as_i64() != Some(team_id) {
            return None;
        }

        team["win"].as_str().map(|status| status.eq_ignore_ascii_case("win"))
    })
}

async fn fetch_current_summoner_identity(
    client: &Client,
    creds: &LcuCredentials,
    summoner_name: &str,
) -> Result<CurrentSummonerIdentity> {
    let url = format!(
        "https://127.0.0.1:{}/lol-summoner/v1/current-summoner",
        creds.port
    );
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", creds.password));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Basic {}", auth))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "LCU current-summoner returned {}",
            resp.status()
        ));
    }

    let json: Value = resp.json().await?;
    let mut aliases = HashSet::new();

    for value in [
        Some(summoner_name),
        json["displayName"].as_str(),
        json["gameName"].as_str(),
        json["internalName"].as_str(),
        json["name"].as_str(),
    ] {
        if let Some(value) = value {
            add_aliases(&mut aliases, value);
        }
    }

    if let (Some(game_name), Some(tag_line)) = (json["gameName"].as_str(), json["tagLine"].as_str()) {
        add_aliases(&mut aliases, &format!("{game_name}#{tag_line}"));
    }

    Ok(CurrentSummonerIdentity {
        puuid: json["puuid"].as_str().unwrap_or_default().to_string(),
        summoner_id: json["summonerId"].as_i64().unwrap_or_default(),
        aliases,
    })
}

fn participant_aliases(player: &Value) -> HashSet<String> {
    let mut aliases = HashSet::new();

    for key in ["summonerName", "gameName", "displayName"] {
        if let Some(value) = player[key].as_str() {
            add_aliases(&mut aliases, value);
        }
    }

    if let (Some(game_name), Some(tag_line)) = (player["gameName"].as_str(), player["tagLine"].as_str()) {
        add_aliases(&mut aliases, &format!("{game_name}#{tag_line}"));
    }

    aliases
}

fn add_aliases(target: &mut HashSet<String>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }

    target.insert(normalize_player_name(trimmed));
    if let Some((game_name, _)) = trimmed.split_once('#') {
        target.insert(normalize_player_name(game_name));
    }
}

fn normalize_player_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Fetch summoner name from LCU.
pub async fn fetch_summoner_name(client: &Client, creds: &LcuCredentials) -> Result<String> {
    let url = format!(
        "https://127.0.0.1:{}/lol-summoner/v1/current-summoner",
        creds.port
    );
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", creds.password));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Basic {}", auth))
        .send()
        .await?;

    let json: Value = resp.json().await?;
    let name = json["displayName"]
        .as_str()
        .or_else(|| json["gameName"].as_str())
        .unwrap_or("Unknown")
        .to_string();

    Ok(name)
}

fn champion_name_from_id(id: i64) -> String {
    match id {
        1 => "Annie",
        2 => "Olaf",
        3 => "Galio",
        4 => "Twisted Fate",
        5 => "Xin Zhao",
        6 => "Urgot",
        7 => "LeBlanc",
        8 => "Vladimir",
        9 => "Fiddlesticks",
        10 => "Kayle",
        11 => "Master Yi",
        12 => "Alistar",
        13 => "Ryze",
        14 => "Sion",
        15 => "Sivir",
        16 => "Soraka",
        17 => "Teemo",
        18 => "Tristana",
        19 => "Warwick",
        20 => "Nunu & Willump",
        21 => "Miss Fortune",
        22 => "Ashe",
        23 => "Tryndamere",
        24 => "Jax",
        25 => "Morgana",
        26 => "Zilean",
        27 => "Singed",
        28 => "Evelynn",
        29 => "Twitch",
        30 => "Karthus",
        31 => "Cho'Gath",
        32 => "Amumu",
        33 => "Rammus",
        34 => "Anivia",
        35 => "Shaco",
        36 => "Dr. Mundo",
        37 => "Sona",
        38 => "Kassadin",
        39 => "Irelia",
        40 => "Janna",
        41 => "Gangplank",
        42 => "Corki",
        43 => "Karma",
        44 => "Taric",
        45 => "Veigar",
        48 => "Trundle",
        50 => "Swain",
        51 => "Caitlyn",
        53 => "Blitzcrank",
        54 => "Malphite",
        55 => "Katarina",
        56 => "Nocturne",
        57 => "Maokai",
        58 => "Renekton",
        59 => "Jarvan IV",
        60 => "Elise",
        61 => "Orianna",
        62 => "Wukong",
        63 => "Brand",
        64 => "Lee Sin",
        67 => "Vayne",
        68 => "Rumble",
        69 => "Cassiopeia",
        72 => "Skarner",
        74 => "Heimerdinger",
        75 => "Nasus",
        76 => "Nidalee",
        77 => "Udyr",
        78 => "Poppy",
        79 => "Gragas",
        80 => "Pantheon",
        81 => "Ezreal",
        82 => "Mordekaiser",
        83 => "Yorick",
        84 => "Akali",
        85 => "Kennen",
        86 => "Garen",
        89 => "Leona",
        90 => "Malzahar",
        91 => "Talon",
        92 => "Riven",
        96 => "Kog'Maw",
        98 => "Shen",
        99 => "Lux",
        101 => "Xerath",
        102 => "Shyvana",
        103 => "Ahri",
        104 => "Graves",
        105 => "Fizz",
        106 => "Volibear",
        107 => "Rengar",
        110 => "Varus",
        111 => "Nautilus",
        112 => "Viktor",
        113 => "Sejuani",
        114 => "Fiora",
        115 => "Ziggs",
        117 => "Lulu",
        119 => "Draven",
        120 => "Hecarim",
        121 => "Kha'Zix",
        122 => "Darius",
        126 => "Jayce",
        127 => "Lissandra",
        131 => "Diana",
        133 => "Quinn",
        134 => "Syndra",
        136 => "Aurelion Sol",
        141 => "Kayn",
        142 => "Zoe",
        143 => "Zyra",
        145 => "Kai'Sa",
        147 => "Seraphine",
        150 => "Gnar",
        154 => "Zac",
        157 => "Yasuo",
        161 => "Vel'Koz",
        163 => "Taliyah",
        164 => "Camille",
        166 => "Akshan",
        200 => "Bel'Veth",
        201 => "Braum",
        202 => "Jhin",
        203 => "Kindred",
        221 => "Zeri",
        222 => "Jinx",
        223 => "Tahm Kench",
        233 => "Briar",
        234 => "Viego",
        235 => "Senna",
        236 => "Lucian",
        238 => "Zed",
        240 => "Kled",
        245 => "Ekko",
        246 => "Qiyana",
        254 => "Vi",
        266 => "Aatrox",
        267 => "Nami",
        268 => "Azir",
        350 => "Yuumi",
        360 => "Samira",
        412 => "Thresh",
        420 => "Illaoi",
        421 => "Rek'Sai",
        427 => "Ivern",
        429 => "Kalista",
        432 => "Bard",
        497 => "Rakan",
        498 => "Xayah",
        516 => "Ornn",
        517 => "Sylas",
        518 => "Neeko",
        523 => "Aphelios",
        526 => "Rell",
        555 => "Pyke",
        711 => "Vex",
        777 => "Yone",
        799 => "Ambessa",
        800 => "Mel",
        804 => "Yunara",
        875 => "Sett",
        876 => "Lillia",
        887 => "Gwen",
        888 => "Renata Glasc",
        893 => "Aurora",
        895 => "Nilah",
        897 => "K'Sante",
        901 => "Smolder",
        902 => "Milio",
        904 => "Zaahen",
        910 => "Hwei",
        950 => "Naafiri",
        _ => "Unknown",
    }
    .to_string()
}

/// Fetch the Active Game summoner name for the local player.
pub async fn fetch_active_game_summoner(client: &Client) -> Result<String> {
    let url = "https://127.0.0.1:2999/liveclientdata/activeplayername";
    let resp = client.get(url).send().await?;
    let name: String = resp.json().await?;
    Ok(name)
}
