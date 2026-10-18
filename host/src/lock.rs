//! The grant lock (docs/mcp-host.md, 「The grant lock」): beneath the agent's own judgement, a tool that acts
//! runs only where the sender of the message it names, in the running turn's room, holds the room relation the
//! tool declares, and a job run for another user uses a tool only where that user holds its capabilities. The
//! account service decides (docs/api.md, 「What the host calls」); the host asks it and refuses with the missing
//! grant named.

use bmo_tool::Session;
use reqwest::StatusCode;
use serde_json::{Value, json};

/// Where the account service stands: its base address and the bearer token it is asked with.
#[derive(Clone, Debug)]
pub struct Accounts {
  pub base: String,
  pub token: Option<String>,
}

/// The lock as the configuration names it.
#[derive(Clone, Debug, Default)]
pub struct Lock {
  /// The account service the lock asks; with none, nothing is checked.
  pub accounts: Option<Accounts>,
  /// Whether the switch turns the lock off.
  pub off: bool,
  pub client: reqwest::Client,
}

/// The room of the turn a call is made in.
pub fn turn_room(session: &Session) -> Option<i64> {
  session.turn_variable("BMO_TURN_CHAT").and_then(|chat| chat.trim().parse().ok())
}

/// Whether every new message of the turn is the owner's, or no person's message opened it: no member
/// is the one a call acts for.
fn turn_privileged(session: &Session) -> bool {
  session.turn_variable("BMO_TURN_PRIVILEGED").is_some_and(|value| value.trim() == "1")
}

/// The user a job's turn is run for, where another user filed its item.
fn turn_user(session: &Session) -> Option<i32> {
  session.turn_variable("BMO_TURN_USER").and_then(|user| user.trim().parse().ok())
}

impl Accounts {
  // One route of the account service asked, answered as its JSON body where it answers 200; anything else is
  // the refusal of the call that asked, naming what the service said.
  async fn asked(
    &self,
    client: &reqwest::Client,
    path: &str,
    body: Option<&Value>,
  ) -> Result<Value, String> {
    let address = format!("{}{path}", self.base.trim_end_matches('/'));
    let mut request = match body {
      Some(body) => client.post(&address).json(body),
      None => client.get(&address),
    };

    if let Some(token) = &self.token {
      request = request.bearer_auth(token);
    }

    let reply = request
      .send()
      .await
      .map_err(|why| format!("refused: the account service does not answer at {}: {why}", self.base))?;
    let status = reply.status();
    let said: Value = reply.json().await.unwrap_or(Value::Null);

    if status != StatusCode::OK {
      let error = said.get("error").and_then(Value::as_str).unwrap_or("no reason given");
      let code = said.get("code").and_then(Value::as_str).unwrap_or("no code");

      return Err(format!("refused: the account service answered {status}: {error} ({code})"));
    }

    Ok(said)
  }

  // One check, answered as whether it opens and the grant it asked for.
  async fn check(
    &self,
    client: &reqwest::Client,
    path: &str,
    asked: &Value,
  ) -> Result<(bool, Option<String>), String> {
    let said = self.asked(client, path, Some(asked)).await?;

    Ok((
      said.get("allowed").and_then(Value::as_bool) == Some(true),
      said.get("grant").and_then(Value::as_str).map(str::to_owned),
    ))
  }
}

impl Lock {
  /// Whether a call may run, or the sentence refusing it: a job run for another user's item uses a tool only
  /// where that user holds each of its capabilities (`bmo_tools:<capability>#use`), and a tool that acts runs
  /// only where the sender of the message it names holds its room relation. The lock stands only where an
  /// account service is named, the switch does not turn it off, and the service's account model is in force.
  pub async fn permits(
    &self,
    session: &Session,
    relation: Option<&str>,
    capabilities: &[&str],
    arguments: &Value,
  ) -> Result<(), String> {
    let Some(accounts) = self.accounts.as_ref().filter(|_| !self.off) else {
      return Ok(());
    };
    let model = accounts.asked(&self.client, "/accounts/model", None).await?;

    if model.get("in_force").and_then(Value::as_bool) != Some(true) {
      return Ok(());
    }

    if let Some(user) = turn_user(session) {
      for capability in capabilities {
        let asked = json!({ "kind": "bmo_tools", "object": capability, "relation": "use", "user": user });
        let (allowed, grant) = accounts.check(&self.client, "/checks/object", &asked).await?;

        if !allowed {
          let grant = grant.unwrap_or_else(|| format!("bmo_tools:{capability}#use"));

          return Err(format!("refused: this job is run for user {user}, who does not hold {grant}"));
        }
      }
    }

    match relation {
      Some(relation) => self.acts(accounts, session, relation, arguments).await,
      None => Ok(()),
    }
  }

  async fn acts(
    &self,
    accounts: &Accounts,
    session: &Session,
    relation: &str,
    arguments: &Value,
  ) -> Result<(), String> {
    let Some(message) = arguments.get("message").and_then(Value::as_i64) else {
      if turn_privileged(session) {
        return Ok(());
      }

      return Err(format!(
        "refused: this tool acts, so it names the message it acts on in `message`, and this call named none \
         (bmo_room#{relation})"
      ));
    };
    let Some(room) = turn_room(session) else {
      return Err(format!(
        "refused: no room's turn is going for this session, so message {message} cannot be checked \
         (bmo_room#{relation})"
      ));
    };
    let asked = json!({ "room": room, "message": message, "relation": relation });
    let (allowed, grant) = accounts.check(&self.client, "/checks/message", &asked).await?;

    if allowed {
      Ok(())
    } else {
      let grant = grant.unwrap_or_else(|| format!("bmo_room:{room}#{relation}"));

      Err(format!("refused: the sender of message {message} does not hold {grant}"))
    }
  }
}
