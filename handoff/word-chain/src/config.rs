//! The service's configuration (docs/api.md, 「Configuration」): one TOML file, `env/word-chain.toml` under the data
//! root once deployed, which the service, its command line and its tool all read.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Another unit the service calls: its base address and the bearer token it takes.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
  pub base: String,
  pub token: String,
}

impl Peer {
  /// The address of one of its routes.
  pub fn url(&self, path: &str) -> String {
    format!("{}{path}", self.base.trim_end_matches('/'))
  }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
  /// The address the control routes answer on.
  pub listen: String,
  /// The bearer token every control route takes.
  pub token: String,
  /// The data root the word book stands under.
  pub data_root: PathBuf,
  /// The language a room's lines are rendered in.
  pub language: String,
  /// The agent a game's lines are said as, by its registration name.
  pub agent: String,
  /// The name the robot plays under.
  pub robot_name: String,
  /// The receiver: the rooms' messages, and where a game's lines go out.
  pub receiver: Peer,
  /// The web unit: the language packs a game's lines are rendered from.
  pub web: Peer,
}

impl Config {
  /// The configuration at `path`, or why it does not read.
  pub fn read(path: &Path) -> Result<Self, String> {
    let text = std::fs::read_to_string(path)
      .map_err(|why| format!("{} could not be read: {why}", path.display()))?;

    toml::from_str(&text).map_err(|why| format!("{} does not read: {why}", path.display()))
  }

  /// Where the configuration stands under a data root once deployed.
  pub fn deployed(data_root: &Path) -> PathBuf {
    data_root.join("env").join("word-chain.toml")
  }

  /// The address the control routes are asked at.
  pub fn control(&self) -> Peer {
    let host = self.listen.replace("0.0.0.0", "127.0.0.1");

    Peer {
      base: format!("http://{host}"),
      token: self.token.clone(),
    }
  }
}
