//! External servers (docs/mcp-host.md, 「External servers」): an MCP server a registration names (docs/api.md, 「The
//! registration of an external server」), connected once per registration and kept for every record's calls, its
//! tools listed and read as entries of the catalogue, and each call of one forwarded to it as a `tools/call`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use bmo_pocket_client::{ReachedBy, Server, Transport};
use bmo_tool::manifest::{
  Entry, HintsPart, Manifest, Mode, PROTOCOL, ToolPart, entry_name, inside, manual, namespace_name,
};
use http::{HeaderName, HeaderValue};
use rmcp::model::{CallToolRequestParams, Tool as Listed};
use rmcp::service::{NotificationContext, Peer, RunningService, ServiceError};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::running::{KEPT, ended_with_host};

/// The registration's file name, in the file a person writes and in its placed version.
pub const REGISTRATION: &str = "server.toml";

/// How long a registration with no listing waits before it is tried again, where nothing else moves first.
pub const RETRY: Duration = Duration::from_secs(30);

/// A server the host starts and speaks to on its standard input and output.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioPart {
  /// The command as words, the first a path or a name found on the path.
  pub command: Vec<String>,
  /// Laid over the host's path, home and runtime directory.
  #[serde(default)]
  pub env: BTreeMap<String, String>,
}

/// A streamable HTTP server the host connects to.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpPart {
  pub url: String,
  /// Sent with every request.
  #[serde(default)]
  pub headers: BTreeMap<String, String>,
  /// The file whose text, trimmed, is sent as the bearer token, read afresh at each connection.
  #[serde(default)]
  pub token_file: Option<PathBuf>,
}

/// What a registration says of one of its server's tools, by the server's name for it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedPart {
  /// A Markdown file, by its path relative to the registration's folder and inside it.
  #[serde(default)]
  pub manual: Option<String>,
}

// Where a registration leaves out which of `call` and `act` reaches a tool its annotations do not mark.
fn acts() -> ReachedBy {
  ReachedBy::Act
}

/// One external server's registration, read and held to its rules, its manuals read beside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
  pub name: String,
  pub namespace: String,
  pub capabilities: Vec<String>,
  #[serde(default = "acts")]
  pub reached_by: ReachedBy,
  /// Read and given no effect: connecting, listing and each call wait for the server's answer.
  #[serde(default)]
  pub timeout: Option<u64>,
  #[serde(default)]
  pub stdio: Option<StdioPart>,
  #[serde(default)]
  pub http: Option<HttpPart>,
  #[serde(default)]
  pub tools: BTreeMap<String, ListedPart>,
  /// Each manual's text, by the server's name of its tool.
  #[serde(skip)]
  pub manuals: BTreeMap<String, String>,
}

/// A registration's name: lower-case letters, digits, `-` and `_`.
pub fn registration_name(name: &str) -> bool {
  !name.is_empty()
    && name
      .chars()
      .all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '-' || one == '_')
}

impl Registration {
  /// The registration standing as `server.toml` in `dir`, its manuals read from beside it; or why it does not
  /// read.
  pub fn read(dir: &Path) -> Result<Self, String> {
    Self::read_file(&dir.join(REGISTRATION))
  }

  /// The registration in a file of any name, its manuals read from the file's folder; or why it does not read.
  pub fn read_file(file: &Path) -> Result<Self, String> {
    let text = std::fs::read_to_string(file).map_err(|why| format!("{}: {why}", file.display()))?;
    let mut registration = Self::parse(&text)?;
    let dir = file.parent().unwrap_or(Path::new("."));

    for (tool, part) in &registration.tools {
      if let Some(file) = &part.manual {
        let text = manual(dir, file).map_err(|why| format!("{tool}: {why}"))?;

        registration.manuals.insert(tool.clone(), text);
      }
    }

    Ok(registration)
  }

  /// A registration's text held to its rules, its manuals not yet read.
  pub fn parse(text: &str) -> Result<Self, String> {
    let registration: Self =
      toml::from_str(text).map_err(|why| format!("{REGISTRATION}: {}", why.message()))?;

    registration.faults()?;
    Ok(registration)
  }

  fn faults(&self) -> Result<(), String> {
    if !registration_name(&self.name) {
      return Err(format!("`{}` is no registration's name", self.name));
    }

    if !namespace_name(&self.namespace) || self.namespace == "toolbook" {
      return Err(format!("`{}` is no namespace's name", self.namespace));
    }

    if self.capabilities.is_empty() {
      return Err("it asks for no capability, so every session would be shown its tools".to_owned());
    }

    if self.capabilities.iter().any(|one| one.trim().is_empty()) {
      return Err("a capability with no name".to_owned());
    }

    match (&self.stdio, &self.http) {
      (None, None) => return Err("it names neither a [stdio] nor an [http] server".to_owned()),
      (Some(_), Some(_)) => return Err("it names both a [stdio] and an [http] server".to_owned()),
      (Some(stdio), None) if stdio.command.first().is_none_or(|word| word.trim().is_empty()) => {
        return Err("its command names nothing to run".to_owned());
      }
      (None, Some(http))
        if !url::Url::parse(&http.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) =>
      {
        return Err(format!("`{}` is no http or https address", http.url));
      }
      _ => {}
    }

    if let Some((tool, file)) = self
      .tools
      .iter()
      .find_map(|(tool, part)| Some((tool, part.manual.as_deref().filter(|file| !inside(file))?)))
    {
      return Err(format!("{tool}: its manual `{file}` does not stand inside the registration's folder"));
    }

    Ok(())
  }

  /// The files of its folder it names beside itself: each manual, by its path in the folder.
  pub fn named_files(&self) -> Vec<String> {
    let mut files: Vec<String> = self.tools.values().filter_map(|part| part.manual.clone()).collect();

    files.sort();
    files.dedup();
    files
  }

  /// Where its server is reached, for a reader: the command's words, or the address.
  pub fn address(&self) -> String {
    match (&self.stdio, &self.http) {
      (Some(stdio), _) => format!("stdio `{}`", stdio.command.join(" ")),
      (None, Some(http)) => http.url.clone(),
      (None, None) => "nowhere".to_owned(),
    }
  }
}

/// A listed tool's name made into an entry's alphabet: an upper-case letter lower-cased, with `_` before it where
/// it follows a lower-case letter or a digit, and every other character outside the alphabet made `_`.
pub fn entry_name_of(listed: &str) -> String {
  let mut name = String::new();
  let mut in_word = false;

  for one in listed.chars() {
    if one.is_ascii_uppercase() {
      if in_word {
        name.push('_');
      }

      name.push(one.to_ascii_lowercase());
      in_word = false;
    } else if one.is_ascii_lowercase() || one.is_ascii_digit() {
      name.push(one);
      in_word = true;
    } else {
      name.push('_');
      in_word = false;
    }
  }

  name
}

/// The tools a server listed read as a manifest's entries under its registration (docs/api.md, 「How a listed tool
/// reads as an entry」), the server's own name of each by its entry's name, and why each tool left out was left
/// out.
pub fn entries(
  registration: &Registration,
  listed: &[Listed],
) -> (Manifest, BTreeMap<String, String>, Vec<String>) {
  let mut names = BTreeMap::new();
  let mut read = Vec::new();
  let mut faults = Vec::new();

  for tool in listed {
    let own = tool.name.to_string();
    let name = entry_name_of(&own);

    if !entry_name(&name) {
      faults.push(format!("its tool `{own}` has no name an entry can take"));
      continue;
    }

    if let Some(first) = names.get(&name) {
      faults.push(format!("its tool `{own}` takes the name {name}, which its tool `{first}` holds"));
      continue;
    }

    let input = Value::Object((*tool.input_schema).clone());
    let output = tool.output_schema.as_ref().map(|schema| Value::Object((**schema).clone()));

    if let Some(why) = std::iter::once(&input)
      .chain(&output)
      .find_map(|schema| jsonschema::validator_for(schema).err())
    {
      faults.push(format!("its tool `{own}`: its schema is not a JSON Schema: {why}"));
      continue;
    }

    let notes = tool.annotations.as_ref();
    let title = tool.title.clone().or_else(|| notes.and_then(|notes| notes.title.clone()));
    let description = tool
      .description
      .as_ref()
      .map(|description| description.to_string())
      .filter(|description| !description.trim().is_empty())
      .or_else(|| title.clone())
      .unwrap_or_else(|| own.clone());
    let read_only = notes
      .and_then(|notes| notes.read_only_hint)
      .unwrap_or(registration.reached_by == ReachedBy::Call);
    let hints = HintsPart {
      read_only,
      destructive: notes.and_then(|notes| notes.destructive_hint).unwrap_or(!read_only),
      idempotent: notes.and_then(|notes| notes.idempotent_hint),
      open_world: notes.and_then(|notes| notes.open_world_hint).unwrap_or(true),
    };

    read.push(Entry {
      namespace: registration.namespace.clone(),
      name: name.clone(),
      title,
      description,
      manual: registration.tools.get(&own).and_then(|part| part.manual.clone()),
      input: String::new(),
      output: None,
      hints,
      capabilities: registration.capabilities.clone(),
      grant: None,
      timeout: None,
      unbounded: false,
      ends_at_stop: false,
      system: false,
      bundle: BTreeMap::new(),
      listing: None,
      facts: None,
      claims: None,
      schedule: Vec::new(),
      input_schema: input,
      output_schema: output,
      manual_text: registration.manuals.get(&own).cloned(),
    });
    names.insert(name, own);
  }

  let manifest = Manifest {
    tool: ToolPart {
      name: registration.name.clone(),
      protocol: PROTOCOL,
      language: None,
      build: None,
      check: None,
      run: Vec::new(),
      mode: Mode::Server,
      timeout: None,
      shared: Vec::new(),
      after_deploy: None,
    },
    entries: read,
    resources: None,
    card: None,
    helper: None,
    app: None,
    permissions: Default::default(),
  };

  (manifest, names, faults)
}

// What the server tells the host while it is connected: that its tool list changed, which has it listed again.
struct Watch {
  link: Weak<Link>,
}

impl ClientHandler for Watch {
  async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
    if let Some(link) = self.link.upgrade() {
      link.stale.store(true, Ordering::SeqCst);
      link.poke();
    }
  }
}

// One connection to a registration's server, ended at once where a call fails on it, or once nothing holds it.
struct Connection {
  peer: Peer<RoleClient>,
  running: Mutex<Option<RunningService<RoleClient, Watch>>>,
}

impl Connection {
  async fn end(&self) {
    let running = self.running.lock().unwrap_or_else(PoisonError::into_inner).take();

    if let Some(running) = running {
      let _ = running.cancel().await;
    }
  }
}

/// One registration as the host holds it: its placed version, its last listing, and its connection, made when it
/// is first needed and kept for every record's calls.
pub struct Link {
  pub registration: Registration,
  pub dir: PathBuf,
  listing: Mutex<Option<Arc<Vec<Listed>>>>,
  /// Why its last connection or listing failed, where it did.
  why: Mutex<Option<String>>,
  /// Whether the server said its tool list changed since it was last listed.
  stale: AtomicBool,
  /// Whether a listing of it is under way, so a reading that comes while its server has not yet answered does not
  /// queue another behind it.
  listing_now: AtomicBool,
  connection: tokio::sync::Mutex<Option<Arc<Connection>>>,
  poke: Option<UnboundedSender<()>>,
}

impl std::fmt::Debug for Link {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("Link")
      .field("name", &self.registration.name)
      .field("dir", &self.dir)
      .finish_non_exhaustive()
  }
}

impl Link {
  /// The tools its server last listed; none before it first answered.
  pub fn listing(&self) -> Option<Arc<Vec<Listed>>> {
    self.listing.lock().unwrap_or_else(PoisonError::into_inner).clone()
  }

  /// Why its last connection or listing failed, where it did.
  pub fn why(&self) -> Option<String> {
    self.why.lock().unwrap_or_else(PoisonError::into_inner).clone()
  }

  /// The server as `GET /servers` answers it: where it is reached, never its headers, token file or environment,
  /// and how far its last listing went.
  pub fn server(&self) -> Server {
    let registration = &self.registration;
    let (transport, address) = match (&registration.stdio, &registration.http) {
      (Some(stdio), _) => (Transport::Stdio, stdio.command.join(" ")),
      (None, Some(http)) => (Transport::Http, http.url.clone()),
      (None, None) => (Transport::Stdio, String::new()),
    };

    Server {
      name: registration.name.clone(),
      namespace: registration.namespace.clone(),
      transport,
      address,
      capabilities: registration.capabilities.clone(),
      reached_by: registration.reached_by,
      listed: self.listing().map(|listed| u32::try_from(listed.len()).unwrap_or(u32::MAX)),
      error: self.why(),
    }
  }

  // Whether it is to be listed: never listed yet, or said to have changed.
  fn wanting(&self) -> bool {
    self.stale.load(Ordering::SeqCst) || self.listing().is_none()
  }

  // The catalogue asked to be read again.
  fn poke(&self) {
    if let Some(poke) = &self.poke {
      let _ = poke.send(());
    }
  }

  fn failed(&self, why: &str) {
    eprintln!("mcp-host: the external server {}: {why}", self.registration.name);
    *self.why.lock().unwrap_or_else(PoisonError::into_inner) = Some(why.to_owned());
  }

  // A listing kept; whether it differs from the last.
  fn keep(&self, listed: Vec<Listed>) -> bool {
    let mut listing = self.listing.lock().unwrap_or_else(PoisonError::into_inner);
    let moved = listing.as_ref().is_none_or(|was| {
      serde_json::to_value(was.as_slice()).ok() != serde_json::to_value(listed.as_slice()).ok()
    });

    *listing = Some(Arc::new(listed));
    *self.why.lock().unwrap_or_else(PoisonError::into_inner) = None;
    moved
  }

  // The stdio command: started in the placed version, in a clean environment of the host's path, home and runtime
  // directory and the registration's own variables, in its own process group, ended with the host.
  fn command(&self, stdio: &StdioPart) -> Command {
    let mut command = Command::new(&stdio.command[0]);

    command.args(&stdio.command[1..]).current_dir(&self.dir).env_clear();

    for kept in KEPT {
      if let Some(value) = std::env::var_os(kept) {
        command.env(kept, value);
      }
    }

    command.envs(&stdio.env);
    ended_with_host(&mut command);
    command
  }

  // The streamable HTTP client's configuration: the address, the headers, and the token file's text as the bearer
  // token, read now.
  fn http_config(&self, http: &HttpPart) -> Result<StreamableHttpClientTransportConfig, String> {
    let mut headers = HashMap::new();

    for (name, value) in &http.headers {
      let name = HeaderName::try_from(name.as_str()).map_err(|why| format!("the header {name}: {why}"))?;
      let value =
        HeaderValue::try_from(value.as_str()).map_err(|why| format!("the header {name}: {why}"))?;

      headers.insert(name, value);
    }

    let mut config =
      StreamableHttpClientTransportConfig::with_uri(http.url.as_str()).custom_headers(headers);

    if let Some(file) = &http.token_file {
      let token = std::fs::read_to_string(file)
        .map_err(|why| format!("its token file {} could not be read: {why}", file.display()))?;

      config = config.auth_header(token.trim());
    }

    Ok(config)
  }

  // A new connection to its server, its tools listed on it.
  async fn connect(self: &Arc<Self>) -> Result<(Arc<Connection>, Vec<Listed>), String> {
    let watch = Watch {
      link: Arc::downgrade(self),
    };
    let made = async {
      let running = match (&self.registration.stdio, &self.registration.http) {
        (Some(stdio), _) => {
          let (process, _) = TokioChildProcess::builder(self.command(stdio))
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|why| format!("`{}` could not be started: {why}", stdio.command[0]))?;

          watch.serve(process).await.map_err(|why| format!("it did not start: {why}"))?
        }
        (None, Some(http)) => {
          let transport = StreamableHttpClientTransport::from_config(self.http_config(http)?);

          watch
            .serve(transport)
            .await
            .map_err(|why| format!("it could not be connected: {why}"))?
        }
        (None, None) => return Err("it names no server".to_owned()),
      };
      let listed = match running.peer().list_all_tools().await {
        Ok(listed) => listed,
        Err(why) => {
          let _ = running.cancel().await;

          return Err(format!("its tools could not be listed: {why}"));
        }
      };

      Ok((running, listed))
    };
    let (running, listed) = made.await?;

    Ok((
      Arc::new(Connection {
        peer: running.peer().clone(),
        running: Mutex::new(Some(running)),
      }),
      listed,
    ))
  }

  // The connection standing, or one made now where none stands or the server went away from the one that did;
  // and, where it was made now, whether its listing moved.
  async fn connection(self: &Arc<Self>) -> Result<(Arc<Connection>, Option<bool>), String> {
    let mut held = self.connection.lock().await;

    if let Some(standing) = held.as_ref() {
      if !standing.peer.is_transport_closed() {
        return Ok((standing.clone(), None));
      }

      if let Some(gone) = held.take() {
        gone.end().await;
      }
    }

    let (made, listed) = self.connect().await?;
    let moved = self.keep(listed);

    *held = Some(made.clone());
    Ok((made, Some(moved)))
  }

  // The connection ended where it is still the one standing.
  async fn disconnect(&self, connection: &Arc<Connection>) {
    let mut held = self.connection.lock().await;

    if held.as_ref().is_some_and(|standing| Arc::ptr_eq(standing, connection)) {
      *held = None;
    }

    drop(held);
    connection.end().await;
  }

  /// Its server listed where it is wanting — never listed, or said to have changed — connecting first where no
  /// connection stands; whether the listing moved. A failure is kept and said in the log.
  pub async fn list(self: &Arc<Self>) -> bool {
    if !self.wanting() || self.listing_now.swap(true, Ordering::SeqCst) {
      return false;
    }

    self.stale.store(false, Ordering::SeqCst);

    let listed = async {
      match self.connection().await? {
        (_, Some(moved)) => Ok(moved),
        (connection, None) => match connection.peer.list_all_tools().await {
          Ok(listed) => Ok(self.keep(listed)),
          Err(why) => {
            self.disconnect(&connection).await;
            Err(format!("its tools could not be listed: {why}"))
          }
        },
      }
    };

    let moved = match listed.await {
      Ok(moved) => moved,
      Err(why) => {
        self.failed(&why);
        false
      }
    };

    self.listing_now.store(false, Ordering::SeqCst);
    moved
  }

  /// Its connection ended where one stands, as the host stops.
  pub async fn end(&self) {
    let held = self.connection.lock().await.take();

    if let Some(connection) = held {
      connection.end().await;
    }
  }

  /// One `tools/call` of the server's tool of this name: its result as the server answered it. A
  /// call failing on the connection ends it, and the next call connects again; an error the server answers in
  /// the protocol fails the call alone.
  pub async fn call(self: &Arc<Self>, name: &str, arguments: Value) -> Result<Value, String> {
    let (connection, moved) = match self.connection().await {
      Ok(made) => made,
      Err(why) => {
        self.failed(&why);
        return Err(format!("its server could not be reached: {why}"));
      }
    };

    if moved == Some(true) {
      self.poke();
    }

    let mut asked = CallToolRequestParams::new(name.to_owned());

    if let Value::Object(arguments) = arguments {
      asked = asked.with_arguments(arguments);
    }

    let why = match connection.peer.call_tool(asked).await {
      Ok(result) => return serde_json::to_value(result).map_err(|why| why.to_string()),
      Err(ServiceError::McpError(refused)) => {
        return Err(format!("its server refused the call: {}", refused.message));
      }
      Err(why) => format!("its server failed the call: {why}"),
    };

    self.disconnect(&connection).await;
    Err(why)
  }
}

/// A placed tool's link to its external server: the registration's link, and the server's own name of each entry
/// by the entry's name.
#[derive(Clone, Debug)]
pub struct External {
  pub link: Arc<Link>,
  pub names: BTreeMap<String, String>,
}

impl External {
  /// One call of an entry, by its full name, forwarded to the server under the server's own name of it.
  pub async fn call(&self, entry: &str, arguments: Value) -> Result<Value, String> {
    let name = entry.rsplit('/').next().unwrap_or(entry);
    let Some(own) = self.names.get(name) else {
      return Err(format!("its server lists no tool for {entry}"));
    };

    self.link.call(own, arguments).await
  }
}

/// Every registration's link the host holds, by its placed version's folder, and the channel that has the
/// catalogue read again.
#[derive(Default)]
pub struct Externals {
  links: Mutex<BTreeMap<PathBuf, Arc<Link>>>,
  poke: OnceLock<UnboundedSender<()>>,
}

impl Externals {
  /// The catalogue read again whenever a link's listing moves outside a reading: a call connecting anew, or a
  /// server saying its list changed.
  pub fn poke_with(&self, poke: UnboundedSender<()>) {
    let _ = self.poke.set(poke);
  }

  /// The link of the registration placed in `dir`: the one held, or a new one with no connection and no listing.
  pub fn link(&self, dir: &Path, registration: Registration) -> Arc<Link> {
    let mut links = self.links.lock().unwrap_or_else(PoisonError::into_inner);

    links
      .entry(dir.to_owned())
      .or_insert_with(|| {
        Arc::new(Link {
          registration,
          dir: dir.to_owned(),
          listing: Mutex::new(None),
          why: Mutex::new(None),
          stale: AtomicBool::new(false),
          listing_now: AtomicBool::new(false),
          connection: tokio::sync::Mutex::new(None),
          poke: self.poke.get().cloned(),
        })
      })
      .clone()
  }

  /// Every link but those of these placed versions let go: its connection ends once no call holds it.
  pub fn keep_only(&self, dirs: &BTreeSet<PathBuf>) {
    self
      .links
      .lock()
      .unwrap_or_else(PoisonError::into_inner)
      .retain(|dir, _| dirs.contains(dir));
  }

  /// Every link held: one for each registration the last reading of the catalogue read.
  pub fn every(&self) -> Vec<Arc<Link>> {
    self.links.lock().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()
  }

  /// Whether a link is to be listed.
  pub fn wanting(&self) -> bool {
    self
      .links
      .lock()
      .unwrap_or_else(PoisonError::into_inner)
      .values()
      .any(|link| link.wanting())
  }

  /// Every link wanting a listing listed, side by side; whether any listing moved.
  pub async fn list(&self) -> bool {
    let wanting: Vec<Arc<Link>> = self
      .links
      .lock()
      .unwrap_or_else(PoisonError::into_inner)
      .values()
      .filter(|link| link.wanting())
      .cloned()
      .collect();
    let listing: Vec<_> =
      wanting.into_iter().map(|link| tokio::spawn(async move { link.list().await })).collect();
    let mut moved = false;

    for one in listing {
      moved |= one.await.unwrap_or(false);
    }

    moved
  }
}
