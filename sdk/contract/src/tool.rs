//! What a tool declares about itself, and the one shape the host serves every tool through.

use std::future::Future;
use std::pin::Pin;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::answer::{Answer, CallError};
use crate::session::Session;

/// The protocol's hints about a tool. A read-only tool is one the CLIs send side by side with others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hints {
  pub read_only: bool,
  pub destructive: bool,
  /// Whether calling it twice with the same arguments does what calling it once does; unsaid where
  /// the tool says nothing.
  pub idempotent: Option<bool>,
  pub open_world: bool,
}

impl Hints {
  /// A tool that reads what this machine holds and changes nothing.
  pub const READ_ONLY: Self = Self {
    read_only: true,
    destructive: false,
    idempotent: None,
    open_world: false,
  };
  /// A tool that writes what it is handed, the same way however often it is handed it.
  pub const WRITES: Self = Self {
    read_only: false,
    destructive: false,
    idempotent: Some(true),
    open_world: false,
  };
  /// A tool that takes something away.
  pub const DELETES: Self = Self {
    read_only: false,
    destructive: true,
    idempotent: None,
    open_world: false,
  };
}

/// The room relation a requester must hold for a tool that acts to run: what the tool does, as the account
/// service's rules name it. The host checks it against the
/// sender of the message the call names, in the running turn's room (docs/mcp-host.md, 「The grant
/// lock」).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
  Search,
  Notes,
  Memories,
  Reminders,
  FileTodo,
  Approve,
  Launch,
  Watch,
  RoomRules,
  Deploy,
  WorkerTalk,
}

impl Grant {
  /// The relation the account service's rules name it by.
  pub fn relation(self) -> &'static str {
    match self {
      Grant::Search => "search",
      Grant::Notes => "notes",
      Grant::Memories => "memories",
      Grant::Reminders => "reminders",
      Grant::FileTodo => "file_todo",
      Grant::Approve => "approve",
      Grant::Launch => "launch",
      Grant::Watch => "watch",
      Grant::RoomRules => "room_rules",
      Grant::Deploy => "deploy",
      Grant::WorkerTalk => "worker_talk",
    }
  }
}

/// A smaller form of a tool shown where a session holds a role's bundle and not the tool's whole
/// capabilities: the bundle's capability, the arguments of the tool's own schema it shows, and those it
/// must give where they differ from the schema's own (docs/mcp-host.md, 「Namespaces and capabilities」).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bundle {
  pub capability: &'static str,
  pub shown: &'static [&'static str],
  pub required: Option<&'static [&'static str]>,
}

/// The form of a tool a session holds: the whole of it, or its bundle's narrower form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
  Whole,
  Bundle,
}

/// A schema narrowed to a bundle's form: the arguments it shows alone, requiring those it names, or else
/// those of the whole schema it still shows.
pub fn narrowed<S: AsRef<str>>(mut schema: Value, shown: &[S], required: Option<&[S]>) -> Value {
  let shows = |name: &str| shown.iter().any(|one| one.as_ref() == name);

  if let Some(Value::Object(properties)) = schema.get_mut("properties") {
    properties.retain(|name, _| shows(name));
  }

  let required: Vec<Value> = match required {
    Some(named) => named.iter().map(|name| Value::from(name.as_ref())).collect(),
    None => schema["required"]
      .as_array()
      .into_iter()
      .flatten()
      .filter(|name| name.as_str().is_some_and(shows))
      .cloned()
      .collect(),
  };

  schema["required"] = Value::Array(required);
  schema
}

/// A tool, as its author writes it: everything it is, declared beside the one thing it does.
pub trait Tool: Send + Sync + 'static {
  /// The name the CLI calls it by within its namespace.
  const NAME: &'static str;
  /// The namespace it is shown under, as its manifest declares it.
  const NAMESPACE: &'static str;
  /// A short human title, where the tool has one.
  const TITLE: Option<&'static str> = None;
  /// What it answers, and nothing about when to reach for it: every description stands in the fixed
  /// prompt of every session shown it.
  const DESCRIPTION: &'static str;
  const HINTS: Hints;
  /// What a session must hold to be shown it, as its manifest declares it; at least one.
  const CAPABILITIES: &'static [&'static str];
  /// The grant a requester must hold for it to run, for a tool that acts.
  const GRANT: Option<Grant> = None;
  /// The narrower form a role's bundle holds it in, where one does; such a form acts under no grant.
  const BUNDLE: Option<Bundle> = None;
  /// Whether it is for the system alone: never shown to a session, and reached through the host's control
  /// socket or another tool's call-back socket.
  const SYSTEM: bool = false;

  /// Its arguments, whose JSON schema is the tool's input schema.
  type Input: DeserializeOwned + JsonSchema + Send;

  /// The schema of the object it answers with beside its text, where it answers one.
  fn output_schema() -> Option<Value> {
    None
  }

  /// Its arguments for a page address it reads better than a plain fetch does, where it claims the address.
  fn claim(_address: &str) -> Option<Value> {
    None
  }

  fn call(&self, session: &Session, input: Self::Input) -> impl Future<Output = Answer> + Send;
}

/// A future a served tool or resource answers with.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The future a served call is.
pub type Call<'a> = Pending<'a, Result<Answer, CallError>>;

/// A tool as the host serves it, whatever its input type: every [`Tool`] is one.
pub trait Served: Send + Sync {
  fn name(&self) -> &'static str;
  fn namespace(&self) -> &'static str;
  fn title(&self) -> Option<&'static str>;
  fn description(&self) -> &'static str;
  fn hints(&self) -> Hints;
  fn capabilities(&self) -> &'static [&'static str];
  fn grant(&self) -> Option<Grant>;
  fn bundle(&self) -> Option<Bundle>;
  fn system(&self) -> bool;
  /// Its arguments for a page address it claims, where it claims the address.
  fn claim(&self, address: &str) -> Option<Value>;
  /// The JSON schema of its arguments.
  fn input_schema(&self) -> Value;
  fn output_schema(&self) -> Option<Value>;

  /// The form a session holds it in: whole where the session holds its every capability, its bundle's where
  /// it holds that bundle alone, and none otherwise or for a tool for the system alone.
  fn held(&self, session: &Session) -> Option<Held> {
    if self.system() {
      return None;
    }

    if session.holds(self.capabilities()) {
      return Some(Held::Whole);
    }

    self.bundle().filter(|bundle| session.holds(&[bundle.capability])).map(|_| Held::Bundle)
  }

  /// The JSON schema of its arguments in the form held: a bundle's shows its own arguments alone, and
  /// requires those it names, or else those of the whole schema it still shows.
  fn schema_held(&self, held: Held) -> Value {
    match (held, self.bundle()) {
      (Held::Bundle, Some(bundle)) => narrowed(self.input_schema(), bundle.shown, bundle.required),
      _ => self.input_schema(),
    }
  }
  /// The call, with its arguments as the protocol handed them; absent arguments are an empty object.
  fn call<'a>(&'a self, session: &'a Session, arguments: Value) -> Call<'a>;
}

impl<T: Tool> Served for T {
  fn name(&self) -> &'static str {
    T::NAME
  }

  fn namespace(&self) -> &'static str {
    T::NAMESPACE
  }

  fn title(&self) -> Option<&'static str> {
    T::TITLE
  }

  fn description(&self) -> &'static str {
    T::DESCRIPTION
  }

  fn hints(&self) -> Hints {
    T::HINTS
  }

  fn capabilities(&self) -> &'static [&'static str] {
    T::CAPABILITIES
  }

  fn grant(&self) -> Option<Grant> {
    T::GRANT
  }

  fn bundle(&self) -> Option<Bundle> {
    T::BUNDLE
  }

  fn system(&self) -> bool {
    T::SYSTEM
  }

  fn claim(&self, address: &str) -> Option<Value> {
    T::claim(address)
  }

  fn output_schema(&self) -> Option<Value> {
    T::output_schema()
  }

  fn input_schema(&self) -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T::Input)).unwrap_or(Value::Null);

    // The protocol wants the object's schema alone; the draft it was written to and the Rust type's
    // own name say nothing a caller reads.
    if let Value::Object(fields) = &mut schema {
      fields.remove("$schema");
      fields.remove("title");
    }

    schema
  }

  fn call<'a>(&'a self, session: &'a Session, arguments: Value) -> Call<'a> {
    let arguments = if arguments.is_null() { Value::Object(Default::default()) } else { arguments };

    Box::pin(async move {
      let input = serde_json::from_value::<T::Input>(arguments)
        .map_err(|why| CallError::InvalidArguments(format!("{}: {why}", T::NAME)))?;

      Ok(Tool::call(self, session, input).await)
    })
  }
}
