//! An app's two settings declared on the account service's delegation chain as its tool is put, or from its unit's
//! declaration file where the app is a screen alone (docs/api.md, 「An app on the delegation chain」): whether the
//! app is offered, which a link below only narrows, and whether it is on, which the nearest link decides. The host
//! reads neither; a session's issuer resolves them.

use std::path::Path;

use bmo_tool::manifest::AppPart;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Request, Uri};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpStream;

/// The unit every app's settings are declared by, so a later put of the same app declares them again.
pub const DECLARED_BY: &str = "pocket";

/// The variables naming the account service's address and its token, and the env file under the data root they
/// stand in where the environment names neither.
const ADDRESS: &str = "WEB_ADDRESS";
const TOKEN: &str = "WEB_TOKEN";
const TOKEN_FILE: &str = "WEB_TOKEN_FILE";
const ENV_FILE: &str = "env/web.env";

/// The account service the delegation chain is kept by: its base address and its bearer token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
  pub address: String,
  pub token: String,
}

/// The kind key of an app's setting of this name.
pub fn kind_key(app: &str, setting: &str) -> String {
  format!("app.{app}.{setting}")
}

/// The two kinds an app declares, each by its key: offered, a flag that narrows, at the platform and the units;
/// and on, a flag the nearest link decides, at the platform, the units and the person, written by the person and
/// their heads.
pub fn kinds(app: &AppPart) -> Vec<(String, Value)> {
  let said = format!("{}: {}", app.name.trim(), app.description.trim());
  let offered = kind_key(&app.key, "offered");
  let on = kind_key(&app.key, "on");

  vec![
    (
      offered.clone(),
      json!({
        "key": offered,
        "declaredBy": DECLARED_BY,
        "description": format!("Whether the app is offered at and below this link. {said}"),
        "shape": "flag",
        "choices": [],
        "mode": "narrowing",
        "default": app.offered,
        "rules": true,
        "levels": ["platform", "unit"],
        "unitPermission": "manage",
        "personWriter": "heads",
      }),
    ),
    (
      on.clone(),
      json!({
        "key": on,
        "declaredBy": DECLARED_BY,
        "description": format!("Whether the app is on where it is offered. {said}"),
        "shape": "flag",
        "choices": [],
        "mode": "nearest",
        "default": app.on,
        "rules": true,
        "levels": ["platform", "unit", "person"],
        "unitPermission": "manage",
        "personWriter": "both",
      }),
    ),
  ]
}

impl Chain {
  /// The account service the environment names, else the one the data root's env file names; none where neither
  /// names an address.
  pub fn named(data_root: &Path) -> Option<Self> {
    let file = env_file(&data_root.join(ENV_FILE));
    let named = |name: &str| {
      std::env::var(name)
        .ok()
        .or_else(|| file.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone()))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    };
    let address = named(ADDRESS)?;
    let token = named(TOKEN)
      .or_else(|| {
        named(TOKEN_FILE)
          .and_then(|path| std::fs::read_to_string(path).ok())
          .map(|token| token.trim().to_owned())
      })
      .unwrap_or_default();

    Some(Self {
      address: address.trim_end_matches('/').to_owned(),
      token,
    })
  }

  /// Both settings of the app declared, a line said for each; or why the account service did not take one.
  pub fn declare(&self, app: &AppPart) -> Result<Vec<String>, String> {
    let chain = self.clone();
    let kinds = kinds(app);

    // A thread of its own, so a put asked from inside a runtime blocks no runtime's thread.
    std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|why| why.to_string())?;

      runtime.block_on(async {
        let mut said = Vec::new();

        for (key, kind) in kinds {
          chain.put_kind(&key, &kind).await?;
          said.push(format!("declared the setting {key} on the delegation chain"));
        }

        Ok(said)
      })
    })
    .join()
    .map_err(|_| "the declaration's thread failed".to_owned())?
  }

  // One kind put on the account service, answered 200.
  async fn put_kind(&self, key: &str, kind: &Value) -> Result<(), String> {
    let address = &self.address;
    let base: Uri = address
      .parse()
      .map_err(|why| format!("the account service address `{address}` does not read: {why}"))?;
    let host = base
      .host()
      .ok_or_else(|| format!("the account service address `{address}` names no host"))?
      .to_owned();
    let port = base.port_u16().unwrap_or(80);
    let path = format!("{}/chain/kinds/{key}", base.path().trim_end_matches('/'));
    let stream = TcpStream::connect((host.as_str(), port))
      .await
      .map_err(|why| format!("the account service at {address} is out of reach: {why}"))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
      .await
      .map_err(|why| format!("the account service at {address} could not be spoken to: {why}"))?;

    tokio::spawn(connection);

    let request = Request::builder()
      .method("PUT")
      .uri(path)
      .header("host", format!("{host}:{port}"))
      .header("authorization", format!("Bearer {}", self.token))
      .header("content-type", "application/json")
      .body(Full::new(Bytes::from(kind.to_string())))
      .map_err(|why| format!("the declaration of {key} could not be built: {why}"))?;
    let answer = sender
      .send_request(request)
      .await
      .map_err(|why| format!("the account service at {address} is out of reach: {why}"))?;
    let status = answer.status();
    let body = answer
      .into_body()
      .collect()
      .await
      .map_err(|why| format!("the account service at {address} answered unreadably: {why}"))?
      .to_bytes();

    if status.is_success() {
      return Ok(());
    }

    let said: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    Err(format!(
      "the account service did not take the setting {key}: {status} {}: {}",
      said["code"].as_str().unwrap_or("refused"),
      said["error"].as_str().unwrap_or_default()
    ))
  }
}

// The `KEY=VALUE` lines of the deployment's env file, each value verbatim after its line's first `=`, as the data
// root's secret files are written: a dotenv reader would reinterpret a quote, a `#` or a `$` in a token.
fn env_file(path: &Path) -> Vec<(String, String)> {
  std::fs::read_to_string(path)
    .unwrap_or_default()
    .lines()
    .filter_map(|line| line.split_once('='))
    .map(|(key, value)| (key.trim().to_owned(), value.to_owned()))
    .collect()
}
