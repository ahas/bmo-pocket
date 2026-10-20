//! The plugin's tools: the four fixed ones `bmo_pocket_client::harness` declares, each a call of its route, and the
//! hosted tools of the session's namespaces, each under the name `<namespace>_<name>`.

use std::collections::BTreeMap;

use bmo_pocket_client::harness::{NamespacesInput, ToolInput, ToolsInput, harness_tools};
use bmo_pocket_client::{Failure, NameQuery, Pocket, Tool, ToolCallRequest};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A failed call as the tool error's text: the host's status, code and sentence, or why nothing answered.
pub fn refusal(failure: &Failure) -> Value {
  match failure {
    Failure::Refused { status, error } => {
      json!({ "status": status.as_u16(), "code": error.code, "error": error.error })
    }
    other => json!({ "error": other.to_string() }),
  }
}

/// The arguments read as the tool's input, or why not as the host would say it.
fn input<T: DeserializeOwned>(arguments: Value) -> Result<T, Value> {
  serde_json::from_value(arguments)
    .map_err(|why| json!({ "code": "bad-request", "error": why.to_string() }))
}

fn answered<T: serde::Serialize>(answer: Result<T, Failure>) -> Result<Value, Value> {
  answer
    .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
    .map_err(|failure| refusal(&failure))
}

/// Whether a name is one of the four fixed tools'.
pub fn fixed(name: &str) -> bool {
  harness_tools().iter().any(|tool| tool.name == name)
}

/// One fixed tool called with its arguments: the route's answer as JSON, or the refusal as the tool error's text.
pub async fn call_fixed(client: &Pocket, name: &str, arguments: Value) -> Result<Value, Value> {
  match name {
    "pocket_namespaces" => {
      let asked: NamespacesInput = input(arguments)?;

      answered(
        client
          .namespaces(&NameQuery {
            after: asked.after,
            limit: asked.limit,
          })
          .await,
      )
    }
    "pocket_tools" => {
      let asked: ToolsInput = input(arguments)?;
      let page = NameQuery {
        after: asked.after,
        limit: asked.limit,
      };

      answered(client.tools(&asked.namespace, &page).await)
    }
    "pocket_tool" => {
      let asked: ToolInput = input(arguments)?;

      answered(client.tool(&asked.namespace, &asked.tool_name).await)
    }
    "pocket_call" => {
      let asked: ToolCallRequest = input(arguments)?;

      answered(client.call(&asked).await)
    }
    other => Err(json!({ "code": "not-found", "error": format!("There is no tool {other}.") })),
  }
}

/// The hosted tools under the names a harness calls them by, `<namespace>_<name>`: a name one of the fixed tools
/// holds, or one two hosted tools would share, is given to none of them, and each such tool is reached through
/// `pocket_call` alone.
pub fn hosted_names(tools: &[Tool]) -> Vec<(String, &Tool)> {
  let mut by_name: BTreeMap<String, Vec<&Tool>> = BTreeMap::new();

  for tool in tools {
    by_name.entry(format!("{}_{}", tool.namespace, tool.name)).or_default().push(tool);
  }

  by_name
    .into_iter()
    .filter(|(name, sharing)| sharing.len() == 1 && !fixed(name))
    .map(|(name, mut sharing)| (name, sharing.remove(0)))
    .collect()
}

/// Every tool of every namespace the session sees, read page by page.
pub async fn every_hosted(client: &Pocket) -> Result<Vec<Tool>, Failure> {
  let mut namespaces = Vec::new();
  let mut after = None;

  loop {
    let page = client.namespaces(&NameQuery { after, limit: None }).await?;

    after = page.items.last().map(|namespace| namespace.name.clone());
    namespaces.extend(page.items.into_iter().map(|namespace| namespace.name));

    if !page.more || after.is_none() {
      break;
    }
  }

  let mut tools = Vec::new();

  for namespace in namespaces {
    let mut after = None;

    loop {
      let page = client.tools(&namespace, &NameQuery { after, limit: None }).await?;

      after = page.items.last().map(|tool| tool.name.clone());
      tools.extend(page.items);

      if !page.more || after.is_none() {
        break;
      }
    }
  }

  Ok(tools)
}
