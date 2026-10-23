//! The control socket served in this process, each route asked through the Rust client as a caller asks it.

use std::sync::Arc;

use bmo_pocket_client::transport::request;
use bmo_pocket_client::{
  ErrorCode, Failure, Granted, NameQuery, Pocket, SessionRequest, TimeQuery, ToolCallRequest,
};
use http::{Method, StatusCode};
use mcp_host::schedule::Schedules;
use mcp_host::server::{page_by_name, page_by_time};
use serde_json::json;

use crate::fixture::{Fixture, ISSUER};

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
  assert_eq!(health.version, env!("CARGO_PKG_VERSION"));

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

  for rule in ["`list`", "`describe`", "`call`", "`act`", "never call a tool by a name"] {
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
