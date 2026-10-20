use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use mcp_host::error::HostError;

async fn answered(error: HostError) -> (StatusCode, String, serde_json::Value) {
  let response = error.into_response();
  let status = response.status();
  let kind = response.headers()[CONTENT_TYPE].to_str().unwrap().to_owned();
  let body = response.into_body().collect().await.unwrap().to_bytes();

  (status, kind, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn an_unknown_token_is_refused_as_unauthorised_in_the_common_error_shape() {
  let (status, kind, body) = answered(HostError::UnknownToken).await;

  assert_eq!(status, StatusCode::UNAUTHORIZED);
  assert_eq!(kind, "application/json");
  assert_eq!(body.as_object().unwrap().len(), 2, "{body}");
  assert_eq!(body["code"], "unauthorized");
  assert_eq!(body["error"], "the token names no session");
}

#[tokio::test]
async fn each_case_answers_the_common_code_of_its_status() {
  let cases = [
    (HostError::NoToken, StatusCode::UNAUTHORIZED, "unauthorized"),
    (HostError::IssuerOnly, StatusCode::FORBIDDEN, "forbidden"),
    (HostError::SessionOnly, StatusCode::FORBIDDEN, "forbidden"),
    (HostError::GrantRefused("no grant".into()), StatusCode::FORBIDDEN, "forbidden"),
    (HostError::UnknownNamespace("garden".into()), StatusCode::NOT_FOUND, "not-found"),
    (HostError::UnknownSession, StatusCode::NOT_FOUND, "not-found"),
    (HostError::UnknownRoute("/x".into()), StatusCode::NOT_FOUND, "no-route"),
    (HostError::MethodNotAllowed, StatusCode::NOT_FOUND, "no-route"),
    (HostError::MalformedRecord("no holder".into()), StatusCode::BAD_REQUEST, "bad-request"),
    (
      HostError::ArgumentsMisfit("x".into()),
      StatusCode::UNPROCESSABLE_ENTITY,
      "arguments-misfit",
    ),
    (HostError::ToolFailed("x".into()), StatusCode::BAD_GATEWAY, "tool-failed"),
    (HostError::Unwritable("x".into()), StatusCode::INTERNAL_SERVER_ERROR, "failed"),
  ];

  for (error, status, code) in cases {
    let (answered_status, _, body) = answered(error).await;

    assert_eq!((answered_status, body["code"].as_str().unwrap()), (status, code));
  }
}
