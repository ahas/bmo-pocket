//! The control socket served in this process, each route asked through the Rust client as a caller asks it.

use std::sync::Arc;

use bmo_pocket_client::transport::{request, request_with};
use bmo_pocket_client::{
  Booked, ErrorCode, EventKind, EventQuery, Failure, Granted, NameQuery, Pocket, ReachedBy, SessionQuery,
  SessionRequest, TimeQuery, ToolCallRequest, ToolChange,
};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::schedule::Schedules;
use mcp_host::server::{page_by_name, page_by_time};
use serde_json::{Value, json};
use tokio::net::{TcpStream, UnixStream};

use crate::units::fixture::{Fixture, ISSUER};

#[test]
fn a_page_by_name_answers_the_names_after_its_cursor_and_says_whether_more_stand() {
  let names = || vec!["c", "a", "d", "b"];
  let page = |after, limit| page_by_name(names(), |one| (*one).to_owned(), after, limit).unwrap();

  assert_eq!(page(None, None), (vec!["a", "b", "c", "d"], false));
  assert_eq!(page(None, Some(2)), (vec!["a", "b"], true));
  assert_eq!(page(Some("b"), Some(2)), (vec!["c", "d"], false));
  assert_eq!(page(Some("d"), None), (Vec::<&str>::new(), false));
  assert!(page_by_name(names(), |one| (*one).to_owned(), None, Some(0)).is_err());
}

#[test]
fn a_page_by_time_never_ends_inside_a_run_of_equal_times() {
  let due = || vec![(10, "a"), (20, "b"), (20, "c"), (20, "d"), (30, "e")];
  let page = |after, limit| page_by_time(due(), |one| one.0, after, limit).unwrap();
  let names = |(page, more): (Vec<(i64, &'static str)>, bool)| {
    (page.into_iter().map(|(_, name)| name).collect::<Vec<_>>(), more)
  };

  assert_eq!(names(page(None, Some(1))), (vec!["a"], true));
  assert_eq!(names(page(None, Some(2))), (vec!["a", "b", "c", "d"], true));
  assert_eq!(names(page(Some(10), Some(1))), (vec!["b", "c", "d"], true));
  assert_eq!(names(page(Some(20), Some(5))), (vec!["e"], false));
  assert_eq!(names(page(None, None)), (vec!["a", "b", "c", "d", "e"], false));
  assert!(page_by_time(due(), |one| one.0, None, Some(0)).is_err());
}

// The code a refused call was answered with.
fn code<T: std::fmt::Debug>(answer: Result<T, Failure>) -> (StatusCode, ErrorCode) {
  match answer {
    Err(Failure::Refused { status, error }) => (status, error.code),
    other => panic!("not refused: {other:?}"),
  }
}

fn asked(capabilities: &[&str], grants: &[&str]) -> SessionRequest {
  SessionRequest {
    holder_name: "tester".to_owned(),
    namespaces: ["notes".to_owned()].into(),
    capabilities: capabilities.iter().map(|one| (*one).to_owned()).collect(),
    grants: grants
      .iter()
      .map(|one| Granted {
        resource: (*one).to_owned(),
      })
      .collect(),
    ttl_ms: Some(60_000),
    ..SessionRequest::default()
  }
}

#[tokio::test]
async fn the_issuer_token_alone_issues_and_revokes_and_every_route_takes_a_token() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let health = issuer.health().await.unwrap();

  assert!(health.ready && health.reason.is_none());
  assert_eq!(health.version, bmo_pocket_plugin::VERSION);

  let bare = request(&socket, Method::GET, "/health", None, None).await.unwrap();

  assert_eq!(bare.status, StatusCode::UNAUTHORIZED);
  assert_eq!(bare.json().unwrap()["code"], "unauthorized");
  assert_eq!(
    code(Pocket::at(&socket, "nobody's").health().await),
    (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
  );

  let issued = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();

  assert_eq!(issued.holder_name, "tester");
  assert_eq!(issued.expires_at, Some(issued.created_at + 60_000));

  let session = Pocket::at(&socket, issued.token.clone());

  assert!(session.health().await.unwrap().ready);
  assert_eq!(
    code(session.issue(&asked(&["notes.write"], &["notes.keep"])).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );
  assert_eq!(code(session.revoke(&issued.token).await), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));

  let malformed = request(&socket, Method::POST, "/tool-sessions", Some(ISSUER), Some(&json!({ "x": 1 })))
    .await
    .unwrap();

  assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
  assert_eq!(malformed.json().unwrap()["code"], "bad-request");

  issuer.revoke(&issued.token).await.unwrap();

  assert_eq!(code(issuer.revoke(&issued.token).await), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
  assert_eq!(code(session.health().await), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized));

  let elsewhere = request(&socket, Method::GET, "/sessions", Some(ISSUER), None).await.unwrap();

  assert_eq!(elsewhere.status, StatusCode::NOT_FOUND);
  assert_eq!(elsewhere.json().unwrap()["code"], "no-route");
}

#[tokio::test]
async fn a_session_sees_its_namespaces_and_the_tools_it_holds_and_the_issuer_sees_every_one() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let reader = Pocket::at(&socket, issuer.issue(&asked(&["notes.read"], &[])).await.unwrap().token);
  let all = NameQuery::default();

  let seen = reader.namespaces(&all).await.unwrap();

  assert_eq!((seen.items.len(), seen.more), (1, false));
  assert_eq!((seen.items[0].name.as_str(), seen.items[0].tool_count), ("notes", 1));
  assert_eq!(issuer.namespaces(&all).await.unwrap().items[0].tool_count, 3);

  let notes = issuer.namespace("notes").await.unwrap();

  assert_eq!(notes.facts["digest"].as_str().unwrap().len(), 64);
  assert_eq!(notes.facts["icons"], json!([{ "name": "read_note", "icon": "i-lucide-notebook" }]));
  assert_ne!(reader.namespace("notes").await.unwrap().facts["digest"], notes.facts["digest"]);
  assert_eq!(code(issuer.namespace("garden").await), (StatusCode::NOT_FOUND, ErrorCode::NotFound));

  let first = issuer
    .tools(
      "notes",
      &NameQuery {
        after: None,
        limit: Some(2),
      },
    )
    .await
    .unwrap();
  let names: Vec<&str> = first.items.iter().map(|tool| tool.name.as_str()).collect();

  assert_eq!((names, first.more), (vec!["broken", "read_note"], true));

  let rest = issuer
    .tools(
      "notes",
      &NameQuery {
        after: Some("read_note".to_owned()),
        limit: Some(2),
      },
    )
    .await
    .unwrap();

  assert_eq!(rest.items.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(), ["write_note"]);
  assert!(!rest.more);
  assert_eq!(rest.items[0].grant.as_deref(), Some("notes.keep"));

  let read = reader.tool("notes", "read_note").await.unwrap();

  assert_eq!(read.icon.as_deref(), Some("i-lucide-notebook"));
  assert_eq!(read.listing.as_ref().unwrap().line, "read_note: says a note back");
  assert!(read.hints.read_only && !read.system);
  assert_eq!(read.input["required"], json!(["text"]));
  assert!(issuer.tool("notes", "broken").await.unwrap().system);
  assert_eq!(code(reader.tool("notes", "broken").await), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
  assert_eq!(
    code(reader.tool("notes", "write_note").await),
    (StatusCode::NOT_FOUND, ErrorCode::NotFound)
  );

  let refused = reader
    .tools(
      "notes",
      &NameQuery {
        after: None,
        limit: Some(0),
      },
    )
    .await;

  assert_eq!(code(refused), (StatusCode::BAD_REQUEST, ErrorCode::BadRequest));
}

#[tokio::test]
async fn a_call_runs_under_the_tokens_record_and_each_refusal_answers_its_status() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let ungranted = Pocket::at(&socket, issuer.issue(&asked(&["notes.write"], &[])).await.unwrap().token);
  let granted =
    Pocket::at(&socket, issuer.issue(&asked(&["notes.write"], &["notes.keep"])).await.unwrap().token);
  let call = |name: &str, text: serde_json::Value| ToolCallRequest {
    namespace: "notes".to_owned(),
    name: name.to_owned(),
    input: json!({ "text": text }).as_object().unwrap().clone(),
  };

  assert_eq!(
    code(ungranted.call(&call("write_note", json!("a"))).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );

  let made = granted.call(&call("write_note", json!("a"))).await.unwrap();

  assert_eq!((made.namespace.as_str(), made.name.as_str()), ("notes", "write_note"));
  assert_eq!(made.result.content[0]["text"], "noted:a");
  assert_eq!(fixture.handed()[0]["grants"], json!([{ "resource": "notes.keep" }]));

  let failed = granted.call(&call("write_note", json!("fail"))).await.unwrap();

  assert_eq!(failed.result.is_error, Some(true));
  assert_eq!(
    code(granted.call(&call("write_note", json!(3))).await),
    (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::ArgumentsMisfit)
  );
  assert_eq!(
    code(granted.call(&call("read_note", json!("x"))).await),
    (StatusCode::NOT_FOUND, ErrorCode::NotFound)
  );
  // An entry for the system alone of the record's namespaces is reached here, and this one breaks.
  assert_eq!(
    code(granted.call(&call("broken", json!("x"))).await),
    (StatusCode::BAD_GATEWAY, ErrorCode::ToolFailed)
  );
  assert_eq!(
    code(issuer.call(&call("write_note", json!("a"))).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );

  let claimed = granted.claim("https://example.org/a").await.unwrap();

  assert!(!claimed.granted && claimed.tool_name.is_none());
  assert_eq!(claimed.reason.as_deref(), Some("no tool the session holds claims it"));
  assert_eq!(
    code(issuer.claim("https://example.org/a").await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );
}

#[tokio::test]
async fn the_schedules_are_paged_by_when_each_is_due_and_seen_where_their_entry_is() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let reader = Pocket::at(&socket, issuer.issue(&asked(&["notes.read"], &[])).await.unwrap().token);
  let writer = Pocket::at(&socket, issuer.issue(&asked(&["notes.write"], &[])).await.unwrap().token);
  let every = issuer.schedules(&TimeQuery::default()).await.unwrap();

  assert_eq!(every.items.len(), 1);
  assert!(!every.more);

  let one = &every.items[0];

  assert_eq!(
    (one.namespace.as_str(), one.tool_name.as_str(), one.cron.as_str()),
    ("notes", "read_note", "0 7 * * *")
  );
  assert_eq!(one.arguments, *json!({ "text": "morning" }).as_object().unwrap());
  assert!(one.last_at.is_none() && !one.running);
  assert_eq!(reader.schedules(&TimeQuery::default()).await.unwrap().items, every.items);
  assert!(writer.schedules(&TimeQuery::default()).await.unwrap().items.is_empty());

  let past = issuer
    .schedules(&TimeQuery {
      after: Some(one.next_at),
      limit: None,
    })
    .await
    .unwrap();

  assert!(past.items.is_empty());
  assert_eq!(
    code(
      issuer
        .schedules(&TimeQuery {
          after: None,
          limit: Some(0)
        })
        .await
    ),
    (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
  );
}

// The discipline (docs/toolbook.md, 「The rules a prompt carries」): the prompt paragraph, as text, to either token
// and to no one without one.
#[tokio::test]
async fn the_discipline_answers_the_prompt_paragraph_as_text_to_either_token() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let issued = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();
  let session = Pocket::at(&socket, issued.token.clone());
  let said = issuer.discipline().await.unwrap();

  assert_eq!(said, mcp_host::server::DISCIPLINE);
  assert_eq!(session.discipline().await.unwrap(), said);
  assert!(said.starts_with("# Finding and using tools\n"), "{said}");

  for rule in [
    "`bmo use`",
    "`bmo use --tools`",
    "--describe",
    "--async",
    "--memory",
    "never call a tool by a name",
  ] {
    assert!(said.contains(rule), "{rule}\n{said}");
  }

  let bare = request(&socket, Method::GET, "/discipline", Some(ISSUER), None).await.unwrap();

  assert_eq!(bare.status, StatusCode::OK);
  assert!(bare.json().is_none());
  assert_eq!(
    code(Pocket::at(&socket, "nobody's").discipline().await),
    (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
  );
}

// The tool sessions standing (docs/api.md, 「A standing session」): to the issuer alone, in order of when each was
// issued, never a token or a variable's value; and the external servers, to the issuer alone.
#[tokio::test]
async fn the_issuer_alone_reads_the_sessions_standing_and_the_external_servers() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let mut first = asked(&["notes.read"], &["notes.keep"]);

  first.variables = [("PLOT".to_owned(), "a secret".to_owned())].into();
  first.labels = json!({ "desk": "one" }).as_object().unwrap().clone();

  let one = issuer.issue(&first).await.unwrap();

  tokio::time::sleep(std::time::Duration::from_millis(5)).await;

  let two = issuer.issue(&asked(&["notes.write"], &[])).await.unwrap();
  let gone = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();

  issuer.revoke(&gone.token).await.unwrap();

  let every = issuer.tool_sessions(&SessionQuery::default()).await.unwrap();
  let made: Vec<u64> = every.items.iter().map(|session| session.created_at).collect();

  assert_eq!((made, every.more), (vec![one.created_at, two.created_at], false));
  assert_eq!(every.items[0].variables, ["PLOT"]);
  assert_eq!(every.items[0].labels["desk"], "one");
  assert_eq!(every.items[0].grants[0].resource, "notes.keep");
  assert_eq!(every.items[0].expires_at, one.expires_at);

  let said = request(&socket, Method::GET, "/tool-sessions", Some(ISSUER), None).await.unwrap();
  let text = String::from_utf8_lossy(&said.body);

  assert!(
    !text.contains(&one.token) && !text.contains("a secret") && !text.contains("\"token\""),
    "{text}"
  );

  let first_page = issuer
    .tool_sessions(&SessionQuery {
      limit: Some(1),
      ..SessionQuery::default()
    })
    .await
    .unwrap();

  assert_eq!((first_page.items.len(), first_page.more), (1, true));

  let rest = issuer
    .tool_sessions(&SessionQuery {
      after: Some(i64::try_from(one.created_at).unwrap()),
      ..SessionQuery::default()
    })
    .await
    .unwrap();

  assert_eq!(
    rest.items.iter().map(|session| session.created_at).collect::<Vec<_>>(),
    [two.created_at]
  );

  let session = Pocket::at(&socket, one.token.clone());

  assert_eq!(
    code(session.tool_sessions(&SessionQuery::default()).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );
  assert_eq!(
    code(session.servers(&NameQuery::default()).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );

  let servers = issuer.servers(&NameQuery::default()).await.unwrap();

  assert!(servers.items.is_empty() && !servers.more);
  assert_eq!(
    code(
      issuer
        .servers(&NameQuery {
          after: None,
          limit: Some(0)
        })
        .await
    ),
    (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
  );
}

// A tool as an event carries it, of one read-only entry of `notes`.
fn change(tool: &str) -> ToolChange {
  ToolChange {
    tool: tool.to_owned(),
    namespaces: vec!["notes".to_owned()],
    entries: vec![Booked {
      tool: format!("notes/{tool}_entry"),
      reached_by: ReachedBy::Call,
      summary: "Reads.".to_owned(),
    }],
    replaced: false,
  }
}

// The event stream opened on the control socket with these headers: its status, its type, and its body as it comes.
async fn subscribe(
  socket: &std::path::Path,
  path: &str,
  headers: &[(&str, &str)],
) -> (StatusCode, String, hyper::body::Incoming) {
  let stream = UnixStream::connect(socket).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let mut asked = Request::get(path).header("host", "localhost");

  for (name, value) in headers {
    asked = asked.header(*name, *value);
  }

  let response = link.send_request(asked.body(Empty::<Bytes>::new()).unwrap()).await.unwrap();
  let kind = response
    .headers()
    .get("content-type")
    .map(|value| value.to_str().unwrap().to_owned())
    .unwrap_or_default();

  (response.status(), kind, response.into_body())
}

// The stream's text read until it holds `until`, within a few seconds.
async fn read_until(body: &mut hyper::body::Incoming, until: &str) -> String {
  let mut said = String::new();

  tokio::time::timeout(std::time::Duration::from_secs(5), async {
    while !said.contains(until) {
      let frame = body.frame().await.expect("the stream ended").unwrap();

      if let Some(data) = frame.data_ref() {
        said.push_str(&String::from_utf8_lossy(data));
      }
    }
  })
  .await
  .unwrap_or_else(|_| panic!("the stream never said {until}: {said}"));

  said
}

#[tokio::test]
async fn the_event_stream_tells_each_event_as_it_is_published_and_resumes_after_a_cursor() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let events = &fixture.host.placed.events;
  let bearer = format!("Bearer {ISSUER}");
  let (status, kind, mut body) = subscribe(&socket, "/events", &[("authorization", &bearer)]).await;

  assert_eq!((status, kind.as_str()), (StatusCode::OK, "text/event-stream"));

  let first = events.publish(EventKind::ToolPlaced, change("almanac"));
  let said = read_until(&mut body, "\n\n").await;
  let (head, data) = said.trim_end().split_once("\ndata: ").unwrap();

  assert_eq!(head, format!("id: {}\nevent: tool.placed", first.id));
  assert_eq!(serde_json::from_str::<Value>(data).unwrap(), serde_json::to_value(&first).unwrap());
  // Numbered from above the moment the run began, which was before the event was read.
  assert!(first.id / 1000 <= i64::try_from(first.at).unwrap());

  let second = events.publish(EventKind::ToolRemoved, change("almanac"));

  assert!(
    read_until(&mut body, "event: tool.removed")
      .await
      .contains(&format!("id: {}", second.id))
  );

  // A reader resuming after the first is told the second first; one narrowed to placements is told neither.
  let resumed = first.id.to_string();
  let (_, _, mut again) =
    subscribe(&socket, "/events", &[("authorization", &bearer), ("last-event-id", &resumed)]).await;

  assert!(
    read_until(&mut again, "\n\n")
      .await
      .starts_with(&format!("id: {}\nevent: tool.removed", second.id))
  );

  let issuer = Pocket::at(&socket, ISSUER);
  let page = issuer.events(&EventQuery::default()).await.unwrap();

  assert_eq!((page.items, page.more), (vec![first.clone(), second.clone()], false));

  let narrowed = issuer
    .events(&EventQuery {
      after: None,
      kinds: Some("tool.placed".to_owned()),
      limit: Some(1),
    })
    .await
    .unwrap();

  assert_eq!((narrowed.items, narrowed.more), (vec![first.clone()], false));

  let after = issuer
    .events(&EventQuery {
      after: Some(first.id),
      ..EventQuery::default()
    })
    .await
    .unwrap();

  assert_eq!(after.items, [second]);
}

#[tokio::test]
async fn the_event_stream_refuses_a_reader_with_no_token_a_sessions_token_and_a_cursor_gone() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let bare = request(&socket, Method::GET, "/events", None, None).await.unwrap();

  assert_eq!(bare.status, StatusCode::UNAUTHORIZED);
  assert_eq!(bare.json().unwrap()["code"], "unauthorized");

  let (status, _, _) = subscribe(&socket, "/events", &[("authorization", "Bearer no-such-token")]).await;

  assert_eq!(status, StatusCode::UNAUTHORIZED);

  let session = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();

  assert_eq!(
    code(Pocket::at(&socket, session.token.clone()).events(&EventQuery::default()).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );

  // A cursor of an earlier run, below the numbers this run began at, is gone, on the stream and on a page alike.
  let stale = request_with(&socket, Method::GET, "/events?after=7", Some(ISSUER), None, &[])
    .await
    .unwrap();

  assert_eq!(stale.status, StatusCode::GONE);
  assert_eq!(stale.json().unwrap()["code"], "gone");
  assert_eq!(
    code(
      issuer
        .events(&EventQuery {
          after: Some(7),
          ..EventQuery::default()
        })
        .await
    ),
    (StatusCode::GONE, ErrorCode::Gone)
  );
  assert_eq!(
    code(
      issuer
        .events(&EventQuery {
          limit: Some(0),
          ..EventQuery::default()
        })
        .await
    ),
    (StatusCode::BAD_REQUEST, ErrorCode::BadRequest)
  );
}

// One JSON-RPC request on the tools face, answered by its status.
async fn rpc(port: u16, token: &str, namespace: &str, method: &str, params: Value) -> StatusCode {
  let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
  let asked = Request::post(format!("/mcp/{namespace}"))
    .header("host", "127.0.0.1")
    .header("content-type", "application/json")
    .header("accept", "application/json, text/event-stream")
    .header("mcp-protocol-version", "2025-06-18")
    .header("authorization", format!("Bearer {token}"))
    .body(Full::new(Bytes::from(body.to_string())))
    .unwrap();
  let response = link.send_request(asked).await.unwrap();
  let status = response.status();

  response.into_body().collect().await.unwrap();
  status
}

#[tokio::test]
async fn a_session_says_which_servers_of_the_tools_face_its_clients_reached() {
  let fixture = Fixture::new();
  let (socket, port) = fixture.serve_with_tools();
  let issuer = Pocket::at(&socket, ISSUER);
  let one = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();

  tokio::time::sleep(std::time::Duration::from_millis(5)).await;

  let other = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();

  assert!(issuer.tool_session(&one.token).await.unwrap().connected.is_empty());

  let opening = json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "probe", "version": "1" } });

  assert_eq!(rpc(port, &one.token, "notes", "initialize", opening.clone()).await, StatusCode::OK);
  assert_eq!(rpc(port, &one.token, "toolbook", "tools/list", json!({})).await, StatusCode::OK);
  // A namespace the record does not name is refused before it is reached.
  assert_eq!(
    rpc(port, &one.token, "pages", "initialize", opening.clone()).await,
    StatusCode::NOT_FOUND
  );

  let read = issuer.tool_session(&one.token).await.unwrap();
  let reached: Vec<&str> = read.connected.iter().map(|server| server.namespace.as_str()).collect();

  assert_eq!(reached, ["notes", "toolbook"]);
  assert!(read.connected.iter().all(|server| server.at >= one.created_at));

  // The first answer's moment is kept.
  let at = read.connected[0].at;

  tokio::time::sleep(std::time::Duration::from_millis(5)).await;
  rpc(port, &one.token, "notes", "tools/list", json!({})).await;
  assert_eq!(issuer.tool_session(&one.token).await.unwrap().connected[0].at, at);

  let every = issuer.tool_sessions(&SessionQuery::default()).await.unwrap();
  let of = |created_at: u64| {
    every
      .items
      .iter()
      .find(|session| session.created_at == created_at)
      .unwrap()
      .connected
      .len()
  };

  assert_eq!((of(one.created_at), of(other.created_at)), (2, 0));
  assert_eq!(
    code(Pocket::at(&socket, one.token.clone()).tool_session(&one.token).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden)
  );

  issuer.revoke(&one.token).await.unwrap();
  assert_eq!(
    code(issuer.tool_session(&one.token).await),
    (StatusCode::NOT_FOUND, ErrorCode::NotFound)
  );
}

// A session whose token is lost (docs/api.md, 「A standing session」): its id, the SHA-256 of its token, reads on
// the page, a holder's sessions alone are read by the holder's name, and the id revokes it as the token would.
#[tokio::test]
async fn a_session_is_read_by_its_holder_and_revoked_by_its_id() {
  use sha2::{Digest, Sha256};

  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issuer = Pocket::at(&socket, ISSUER);
  let mut theirs = asked(&["notes.read"], &[]);

  theirs.holder_name = "a-prober".to_owned();

  let lost = issuer.issue(&theirs).await.unwrap();
  let kept = issuer.issue(&asked(&["notes.read"], &[])).await.unwrap();
  let id: String = Sha256::digest(lost.token.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
  let probing = issuer
    .tool_sessions(&SessionQuery {
      holder: Some("a-prober".to_owned()),
      ..SessionQuery::default()
    })
    .await
    .unwrap();

  assert_eq!(probing.items.iter().map(|one| one.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
  assert_eq!(probing.items[0].created_at, lost.created_at);

  assert_eq!(
    code(Pocket::at(&socket, &kept.token).revoke_record(&id).await),
    (StatusCode::FORBIDDEN, ErrorCode::Forbidden),
    "a session's own token revokes nothing"
  );

  issuer.revoke_record(&id).await.unwrap();

  assert_eq!(
    code(Pocket::at(&socket, &lost.token).discipline().await),
    (StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
    "the revoked session's token is refused"
  );
  assert_eq!(code(issuer.revoke_record(&id).await), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
  assert_eq!(
    code(issuer.revoke_record("not-an-id").await),
    (StatusCode::NOT_FOUND, ErrorCode::NotFound)
  );
  assert!(issuer.tool_session(&kept.token).await.is_ok(), "another session stands");
}

#[tokio::test]
async fn the_kept_calls_are_the_issuers_to_read_and_no_sessions() {
  let fixture = Fixture::new();
  let socket = fixture.serve(Arc::new(Schedules::default()));
  let issued = Pocket::at(&socket, ISSUER).issue(&asked(&["notes.read"], &[])).await.unwrap();

  fixture.kept_call("kept", ("bmo/wait", "agent-a", 1), json!({ "note": "n" }), None);

  let read = request(&socket, Method::GET, "/uses", Some(ISSUER), None).await.unwrap();

  assert_eq!(read.status, StatusCode::OK);
  assert_eq!(read.json().unwrap()["items"][0]["id"], "kept");

  let refused = request(&socket, Method::GET, "/uses", Some(&issued.token), None).await.unwrap();

  assert_eq!(refused.status, StatusCode::FORBIDDEN);
}
