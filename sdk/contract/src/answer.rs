//! What a tool answers with: MCP content, in the shape the protocol writes it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One piece of an answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
  Text {
    text: String,
  },
  Image {
    /// The picture, base64 encoded.
    data: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
  },
}

/// A tool's answer. A refusal — a desk that is not open, a node that is not there — is an answer
/// marked as an error, which the model reads as it reads any other. A tool that declares an output schema
/// answers the object beside its text, which is the same object written out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
  pub content: Vec<Content>,
  #[serde(rename = "isError", default)]
  pub is_error: bool,
  #[serde(rename = "structuredContent", default, skip_serializing_if = "Option::is_none")]
  pub structured: Option<serde_json::Value>,
}

impl Answer {
  pub fn text(text: impl Into<String>) -> Self {
    Self {
      content: vec![Content::Text { text: text.into() }],
      is_error: false,
      structured: None,
    }
  }

  pub fn refusal(text: impl Into<String>) -> Self {
    Self {
      content: vec![Content::Text { text: text.into() }],
      is_error: true,
      structured: None,
    }
  }

  /// An object as a tool with an output schema answers it: the object, and the same object as its text.
  pub fn structured(object: serde_json::Value) -> Self {
    Self {
      content: vec![Content::Text {
        text: object.to_string(),
      }],
      is_error: false,
      structured: Some(object),
    }
  }

  /// A tool that could not read what it answers out of: the reason, marked as the verbs mark it.
  pub fn failure(reason: impl std::fmt::Display) -> Self {
    Self::refusal(format!("ERROR: {reason}"))
  }

  /// Lines, under the ceiling every verb answers under, with what was cut said in the answer itself:
  /// a tool result is one text, and a caller told nothing would read a short answer as the whole.
  pub fn lines(lines: &[String], how: &str) -> Self {
    let capped =
      crate::ceiling::cap_items(lines, |kept| kept.join("\n"), crate::ceiling::Said::new("lines", how));
    let mut said: Vec<&str> = capped.kept.iter().map(String::as_str).collect();

    if let Some(note) = &capped.note {
      said.push(note);
    }

    Self::text(said.join("\n"))
  }

  /// The text of every text piece, joined, as a reader of text alone would read the answer.
  pub fn plain(&self) -> String {
    self
      .content
      .iter()
      .filter_map(|piece| match piece {
        Content::Text { text } => Some(text.as_str()),
        Content::Image { .. } => None,
      })
      .collect::<Vec<_>>()
      .join("\n")
  }
}

/// A call the tool never ran: what it was handed does not fit its input. The host answers it as the
/// protocol's invalid-arguments error rather than as the tool's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
  InvalidArguments(String),
}

impl fmt::Display for CallError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::InvalidArguments(why) => write!(f, "invalid arguments: {why}"),
    }
  }
}

impl std::error::Error for CallError {}
