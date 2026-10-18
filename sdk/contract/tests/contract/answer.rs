use bmo_tool::{Answer, Content};
use serde_json::json;

#[test]
fn an_answer_is_written_as_the_protocol_writes_one() {
  assert_eq!(
    serde_json::to_value(Answer::refusal("No desk is open.")).unwrap(),
    json!({ "content": [{ "type": "text", "text": "No desk is open." }], "isError": true }),
  );
  assert_eq!(
    serde_json::to_value(Content::Image {
      data: "AAA=".into(),
      mime_type: "image/jpeg".into()
    })
    .unwrap(),
    json!({ "type": "image", "data": "AAA=", "mimeType": "image/jpeg" }),
  );
}
