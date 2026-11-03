//! The `tool` nouns `bmo-pocket` declares (docs/api.md, 「The CLI」): the host's catalogue listed, one entry
//! described, one entry called, and the site tools the web namespace's entries add, each on the control socket's
//! routes with the issuer token.

use std::collections::BTreeSet;
use std::path::Path;

use bmo_pocket_client::{
  Failure, Granted, NameQuery, Pocket, SessionRequest, Tool, ToolCallRequest, ToolResult,
};
use clap::Subcommand;
use http::StatusCode;
use serde_json::{Map, Value};

/// Every noun this command declares, as `bmo-pocket nouns` prints them: `tool` itself, which answers its usage, and
/// its four nouns, which the longest match reaches first.
pub const NOUNS: [&str; 5] = ["tool", "tool list", "tool describe", "tool call", "tool sites"];

/// The most members one page asks for; the pages go on until none stands past the last.
const PAGE: u32 = 1000;

/// The `tool` nouns as they are asked.
#[derive(Subcommand, Debug)]
pub enum Noun {
  /// Every entry the host serves, one a line, narrowed by the words given.
  List {
    /// Words the entry's full name or description must hold.
    words: Vec<String>,
  },
  /// One entry by its full name, namespace/name.
  Describe {
    /// The entry's full name, namespace/name.
    #[arg(value_parser = full_name)]
    tool: FullName,
  },
  /// One entry called by its full name, its arguments one JSON object.
  Call {
    /// The entry's full name, namespace/name.
    #[arg(value_parser = full_name)]
    tool: FullName,
    /// The entry's arguments, one JSON object.
    #[arg(default_value = "{}", value_parser = object)]
    arguments: Map<String, Value>,
  },
  /// The site tools the web namespace's entries add.
  Sites {
    /// Each site tool's description beside its other words.
    #[arg(long)]
    detail: bool,
  },
}

/// A tool's full name, split into the namespace and the name its manifest declares.
#[derive(Clone, Debug)]
pub struct FullName {
  namespace: String,
  name: String,
}

impl FullName {
  fn full(&self) -> String {
    format!("{}/{}", self.namespace, self.name)
  }
}

/// What a noun printed, and whether it ends as failed: a tool's own failed result is an answer, not a refusal.
#[derive(Debug, Default)]
pub struct Answered {
  /// What the noun writes on standard output, a line each.
  pub lines: Vec<String>,
  /// What it says on standard error without changing its answer.
  pub notes: Vec<String>,
  /// Whether it ends 1.
  pub failed: bool,
}

impl Answered {
  fn ok(lines: Vec<String>) -> Self {
    Self {
      lines,
      ..Self::default()
    }
  }

  fn failed(lines: Vec<String>) -> Self {
    Self {
      lines,
      failed: true,
      ..Self::default()
    }
  }
}

/// Runs one `tool` noun against the host the data root's issuer token opens.
pub async fn run(
  noun: Noun,
  data_root: &Path,
  issuer_token_file: Option<&Path>,
) -> Result<Answered, String> {
  let issuer = issuer_token(data_root, issuer_token_file)?;
  let client = Pocket::standing(issuer);

  match noun {
    Noun::List { words } => list(&client, &words).await,
    Noun::Describe { tool } => describe(&client, &tool).await,
    Noun::Call { tool, arguments } => call(&client, &tool, &arguments).await,
    Noun::Sites { detail } => sites(&client, detail).await,
  }
}

/// Whether a stop of the running host would wait on anything: true where it would not, with the line that says
/// so. A host that cannot be read holds nothing it could be told of, so it is answered true, saying why.
pub async fn quiet(data_root: &Path, issuer_token_file: Option<&Path>) -> (bool, String) {
  let read = match issuer_token(data_root, issuer_token_file) {
    Ok(issuer) => Pocket::standing(issuer).health().await.map_err(|why| why.to_string()),
    Err(why) => Err(why),
  };

  match read.map(|health| health.calls_in_hand) {
    Ok(0) => (true, "no tool call in hand".to_owned()),
    Ok(1) => (false, "1 tool call in hand".to_owned()),
    Ok(calls) => (false, format!("{calls} tool calls in hand")),
    Err(why) => (true, format!("the tool host could not be read, so no call is waited on: {why}")),
  }
}

// The issuer token the host keeps, read where it stands.
fn issuer_token(data_root: &Path, named: Option<&Path>) -> Result<String, String> {
  let file = named.map_or_else(|| data_root.join("tool-host/issuer-token"), Path::to_path_buf);
  let token = std::fs::read_to_string(&file).map_err(|why| {
    format!(
      "the issuer token {} could not be read: {why}; the host may not have started",
      file.display()
    )
  })?;
  let token = token.trim();

  if token.is_empty() {
    return Err(format!("the issuer token {} is empty", file.display()));
  }

  Ok(token.to_owned())
}

// One tool as its full name names it, or the host's refusal.
async fn entry(client: &Pocket, target: &FullName) -> Result<Tool, String> {
  client.tool(&target.namespace, &target.name).await.map_err(|failure| match failure {
    Failure::Refused { status, .. } if status == StatusCode::NOT_FOUND => {
      format!("the tool host serves no {}", target.full())
    }
    other => other.to_string(),
  })
}

// Every namespace the token sees, in order of name, every page read.
async fn namespaces(client: &Pocket) -> Result<Vec<String>, String> {
  let mut found = Vec::new();
  let mut after = None;

  loop {
    let page = client
      .namespaces(&NameQuery {
        after,
        limit: Some(PAGE),
      })
      .await
      .map_err(|failure| failure.to_string())?;
    let Some(last) = page.items.last() else {
      break;
    };

    after = Some(last.name.clone());
    found.extend(page.items.into_iter().map(|namespace| namespace.name));

    if !page.more {
      break;
    }
  }

  Ok(found)
}

// Every entry of one namespace the token sees, in order of name, every page read.
async fn tools(client: &Pocket, namespace: &str) -> Result<Vec<Tool>, String> {
  let mut found = Vec::new();
  let mut after = None;

  loop {
    let page = client
      .tools(
        namespace,
        &NameQuery {
          after,
          limit: Some(PAGE),
        },
      )
      .await
      .map_err(|failure| failure.to_string())?;
    let Some(last) = page.items.last() else {
      break;
    };

    after = Some(last.name.clone());
    found.extend(page.items);

    if !page.more {
      break;
    }
  }

  Ok(found)
}

async fn list(client: &Pocket, words: &[String]) -> Result<Answered, String> {
  let wanted: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
  let mut lines = Vec::new();

  for namespace in namespaces(client).await? {
    for tool in tools(client, &namespace).await? {
      let name = format!("{}/{}", tool.namespace, tool.name);
      let said = format!("{name} {}", tool.description).to_lowercase();

      if wanted.iter().all(|word| said.contains(word)) {
        lines.push(line(&tool));
      }
    }
  }

  Ok(Answered::ok(lines))
}

// A description's first sentence.
fn first_sentence(text: &str) -> &str {
  let text = text.trim();

  text.find(". ").map_or(text, |end| &text[..=end])
}

// One entry on its line: its full name, marked where it is for the system alone, and its first sentence.
fn line(tool: &Tool) -> String {
  let name = format!("{}/{}", tool.namespace, tool.name);

  if tool.system {
    format!("{name} · system: {}", first_sentence(&tool.description))
  } else {
    format!("{name}: {}", first_sentence(&tool.description))
  }
}

async fn describe(client: &Pocket, target: &FullName) -> Result<Answered, String> {
  let tool = entry(client, target).await?;
  let mut lines = vec![format!("{}/{}: {}", tool.namespace, tool.name, tool.description)];

  if let Some(title) = &tool.title {
    lines.push(format!("Title: {title}"));
  }

  let capabilities = serde_json::to_string(&tool.capabilities)
    .map_err(|why| format!("the capabilities do not write: {why}"))?;

  lines.push(format!("Capabilities: {capabilities}"));
  lines.push(format!("Grant: {}", tool.grant.as_deref().unwrap_or("none")));

  if tool.system {
    lines.push("System: for the system alone".to_owned());
  }

  if let Some(listing) = &tool.listing {
    lines.push(format!("Listing: {}", listing.line));
  }

  Ok(Answered::ok(lines))
}

async fn call(
  client: &Pocket,
  target: &FullName,
  arguments: &Map<String, Value>,
) -> Result<Answered, String> {
  let tool = entry(client, target).await?;
  // A session of its own: the entry's namespace, the capabilities it asks for and the grant it declares, and the
  // entry shown by its full name. It stands until the command takes it back after the answer, so no span of it
  // ends the call under it.
  let asked = SessionRequest {
    holder_name: "shell".to_owned(),
    namespaces: BTreeSet::from([tool.namespace.clone()]),
    capabilities: tool.capabilities.iter().cloned().collect(),
    grants: tool
      .grant
      .iter()
      .map(|resource| Granted {
        resource: resource.clone(),
      })
      .collect(),
    shown: BTreeSet::from([target.full()]),
    ttl_ms: None,
    ..SessionRequest::default()
  };
  let session = client
    .issue(&asked)
    .await
    .map_err(|why| format!("a session for the call was not issued: {why}"))?;
  let called = ToolCallRequest {
    namespace: target.namespace.clone(),
    name: target.name.clone(),
    input: arguments.clone(),
  };
  let answered = Pocket::at(client.socket(), session.token.clone()).call(&called).await;
  let taken_back = client.revoke(&session.token).await;
  let mut notes = Vec::new();

  if let Err(why) = taken_back {
    notes.push(format!("the session for the call was not taken back: {why}"));
  }

  let answered = answered.map_err(|why| why.to_string())?;
  let mut answer = if answered.result.is_error == Some(true) {
    Answered::failed(content(&answered.result))
  } else {
    Answered::ok(content(&answered.result))
  };

  answer.notes = notes;

  Ok(answer)
}

// Each content piece as its text, or as JSON where it carries none.
fn content(result: &ToolResult) -> Vec<String> {
  result
    .content
    .iter()
    .map(|piece| {
      piece
        .get("text")
        .and_then(Value::as_str)
        .map_or_else(|| piece.to_string(), str::to_owned)
    })
    .collect()
}

async fn sites(client: &Pocket, detail: bool) -> Result<Answered, String> {
  let namespace = client.namespace("web").await.map_err(|failure| match failure {
    Failure::Refused { status, .. } if status == StatusCode::NOT_FOUND => {
      "the tool host keeps no site catalogue".to_owned()
    }
    other => other.to_string(),
  })?;
  let Some(sites) = namespace.facts.get("sites").and_then(Value::as_array) else {
    return Err("the tool host keeps no site catalogue".to_owned());
  };

  Ok(Answered::ok(site_lines(sites, detail)))
}

// One site tool's words of a namespace's facts.
fn text<'a>(tool: &'a Value, field: &str) -> &'a str {
  tool.get(field).and_then(Value::as_str).unwrap_or_default()
}

fn site_lines(sites: &[Value], detail: bool) -> Vec<String> {
  let mut lines = vec![format!("{} site tools", sites.len())];

  for tool in sites {
    let arguments: Vec<&str> = tool
      .get("arguments")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(Value::as_str)
      .collect();

    lines.push(String::new());
    lines.push(format!("{}: {}", text(tool, "name"), text(tool, "what")));
    lines.push(format!("  site: {}", text(tool, "site")));
    lines.push(format!("  since: {}", text(tool, "since")));
    lines.push(format!(
      "  arguments: {}",
      if arguments.is_empty() { "none".to_owned() } else { arguments.join(", ") }
    ));

    if let Some(answers) = tool.get("answers").and_then(Value::as_str) {
      lines.push(format!("  answers: {answers}"));
    }

    if detail {
      lines.push(format!("  description: {}", text(tool, "description")));
    }
  }

  lines
}

// The words of a full name, refused where they are not one.
fn full_name(text: &str) -> Result<FullName, String> {
  let (namespace, name) =
    text.split_once('/').ok_or_else(|| "a tool is named namespace/name".to_owned())?;

  if namespace.is_empty() || name.is_empty() || name.contains('/') {
    return Err("a tool is named namespace/name".to_owned());
  }

  Ok(FullName {
    namespace: namespace.to_owned(),
    name: name.to_owned(),
  })
}

// The arguments of a call, refused where they are not one JSON object.
fn object(text: &str) -> Result<Map<String, Value>, String> {
  match serde_json::from_str::<Value>(text) {
    Ok(Value::Object(arguments)) => Ok(arguments),
    Ok(_) => Err("the arguments are one JSON object".to_owned()),
    Err(why) => Err(format!("the arguments do not read as JSON: {why}")),
  }
}
