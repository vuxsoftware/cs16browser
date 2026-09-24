use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// Server identity (IP + port).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Endpoint {
    pub ip: Ipv4Addr,
    pub port: u16,
}

impl Endpoint {
    pub fn new(ip: Ipv4Addr, port: u16) -> Self {
        Self { ip, port }
    }

    /// Parse "1.2.3.4:27015" format.
    pub fn parse(s: &str) -> Option<Self> {
        let (ip, port) = s.split_once(':')?;
        let ip: Ipv4Addr = ip.parse().ok()?;
        let port: u16 = port.parse().ok()?;
        Some(Self { ip, port })
    }

    pub fn socket_addr(&self) -> std::net::SocketAddr {
        std::net::SocketAddr::from((self.ip, self.port))
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

/// Possible gamedirs to filter on master servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Game {
    CS16,
    CZ,
    CZRetail,
    CStrikeBeta,
    HalfLife,
    TFC,
    DOD,
    DMC,
    Gearbox,
    Ricochet,
}

impl Game {
    /// Steam AppID used in master server queries.
    pub fn app_id(self) -> u32 {
        match self {
            Game::HalfLife => 70,
            Game::CS16 => 10,
            Game::CZ => 80,
            Game::CZRetail => 100,
            Game::CStrikeBeta => 365,
            Game::TFC => 20,
            Game::DOD => 30,
            Game::DMC => 40,
            Game::Gearbox => 50,
            Game::Ricochet => 60,
        }
    }

    /// The `gamedir` token sent in the master filter.
    pub fn gamedir(self) -> &'static str {
        match self {
            Game::HalfLife => "valve",
            Game::CS16 => "cstrike",
            Game::CZ => "czero",
            Game::CZRetail => "czeror",
            Game::CStrikeBeta => "cstrike_beta",
            Game::TFC => "tfc",
            Game::DOD => "dod",
            Game::DMC => "dmc",
            Game::Gearbox => "gearbox",
            Game::Ricochet => "ricochet",
        }
    }

    /// Human-readable label.
    /// The stock game description a server of this game reports, used until
    /// the server itself answers with its own.
    pub fn description(self) -> &'static str {
        match self {
            Game::HalfLife => "Half-Life",
            Game::CS16 | Game::CStrikeBeta => "Counter-Strike",
            Game::CZ | Game::CZRetail => "Condition Zero",
            Game::TFC => "Team Fortress",
            Game::DOD => "Day of Defeat",
            Game::DMC => "Deathmatch Classic",
            Game::Gearbox => "Opposing Force",
            Game::Ricochet => "Ricochet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Game::HalfLife => "Half-Life",
            Game::CS16 => "Counter-Strike 1.6",
            Game::CZ => "Condition Zero",
            Game::CZRetail => "CZ Retail",
            Game::CStrikeBeta => "CS 1.6 Beta",
            Game::TFC => "TFC",
            Game::DOD => "Day of Defeat",
            Game::DMC => "Deathmatch Classic",
            Game::Gearbox => "Opposing Force",
            Game::Ricochet => "Ricochet",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "cstrike" | "cs" | "cs16" | "counter-strike" => Game::CS16,
            "czero" | "cz" => Game::CZ,
            "czeror" => Game::CZRetail,
            "cstrike_beta" => Game::CStrikeBeta,
            "valve" | "hl" | "halflife" => Game::HalfLife,
            "tfc" => Game::TFC,
            "dod" => Game::DOD,
            "dmc" => Game::DMC,
            "gearbox" | "opposing_force" | "of" => Game::Gearbox,
            "ricochet" => Game::Ricochet,
            _ => Game::CS16,
        }
    }
}

/// Server type as reported by A2S_INFO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerType {
    Dedicated,
    Listen,
    SourceTV,
}

impl ServerType {
    pub fn from_byte(b: u8) -> Self {
        match b {
            b'd' => ServerType::Dedicated,
            b'l' => ServerType::Listen,
            b'p' => ServerType::SourceTV,
            _ => ServerType::Dedicated,
        }
    }
}

/// Operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    Windows,
    Linux,
    Mac,
}

impl Os {
    pub fn from_byte(b: u8) -> Self {
        match b {
            b'w' => Os::Windows,
            b'l' | b'L' => Os::Linux,
            b'm' | b'o' => Os::Mac,
            _ => Os::Windows,
        }
    }
}

/// VAC (anti-cheat) status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vac {
    Unsecured,
    Secured,
}

/// Country code from GeoIP.
pub type CountryCode = String;

/// A player currently connected to a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub index: u8,
    pub name: String,
    pub score: i32,
    pub duration_seconds: f32,
}

/// Server info returned by A2S_INFO + A2S_PLAYER + A2S_RULES.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub endpoint: Endpoint,
    pub protocol: u8,
    pub hostname: String,
    pub map: String,
    pub gamedir: String,
    pub game: Game,
    pub app_id: u16,
    /// The server's own game description (A2S `game` field): what the in-game
    /// browser's Game column shows — "Counter-Strike" by default, but a mod
    /// may set "CSDM", "Zombie Plague" and so on. Steam's listing does not
    /// carry it, so a listed-only row uses [`Game::description`].
    pub game_desc: String,
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    pub server_type: ServerType,
    pub os: Os,
    pub password: bool,
    pub vac: Vac,
    pub version: String,
    pub ping_ms: Option<u32>,
    pub country: Option<CountryCode>,
    pub city: Option<String>,
    pub players_list: Vec<Player>,
    /// The bot plugin the server's rules reveal (`"YaPB 4.4.957"`). Such a
    /// plugin can report its bots as humans, so `bots` alone is not trusted.
    /// `None` when the rules were not asked for or name no plugin.
    #[serde(default)]
    pub bot_plugin: Option<String>,
    pub ping_history: Vec<u32>,
    pub response_time_ms: u32,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}
