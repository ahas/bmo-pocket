use bmo_tool::{MAX_ANSWER_CHARS, MAX_VALUE_CHARS, Said, cap_items, cap_value, count_text};
use serde::Deserialize;

// The cases the ceiling is held to.
const CASES: &str = include_str!("../fixtures/answer-cap-cases.json");

#[derive(Deserialize)]
#[serde(untagged)]
enum Piece {
  Text(String),
  Repeated { text: String, times: usize },
}

fn joined(pieces: &[Piece]) -> String {
  pieces
    .iter()
    .map(|piece| match piece {
      Piece::Text(text) => text.clone(),
      Piece::Repeated { text, times } => text.repeat(*times),
    })
    .collect()
}

#[derive(Deserialize)]
struct ItemCase {
  name: String,
  items: Vec<Piece>,
  copies: Option<usize>,
  limit: Option<usize>,
  render: String,
  unit: String,
  how: String,
  kept: usize,
  left: usize,
  note: Option<String>,
}

#[derive(Deserialize)]
struct ValueCase {
  name: String,
  text: Vec<Piece>,
  limit: Option<usize>,
  expect: Vec<Piece>,
}

#[derive(Deserialize)]
struct Cases {
  items: Vec<ItemCase>,
  values: Vec<ValueCase>,
}

fn render(kind: &str) -> impl Fn(&[String]) -> String {
  let json = kind == "json";

  move |kept| if json { serde_json::to_string_pretty(kept).unwrap() } else { kept.join("\n") }
}

#[test]
fn a_list_is_cut_to_whole_items_under_the_ceiling() {
  let cases: Cases = serde_json::from_str(CASES).unwrap();

  for case in &cases.items {
    let pieces: Vec<String> = case.items.iter().map(|piece| joined(std::slice::from_ref(piece))).collect();
    let items: Vec<String> =
      pieces.iter().cloned().cycle().take(pieces.len() * case.copies.unwrap_or(1)).collect();
    let said = Said {
      unit: &case.unit,
      how: &case.how,
      limit: case.limit.unwrap_or(MAX_ANSWER_CHARS),
    };
    let capped = cap_items(&items, render(&case.render), said);

    assert_eq!(
      (capped.kept.len(), capped.left, capped.note.clone()),
      (case.kept, case.left, case.note.clone()),
      "{}",
      case.name
    );
  }
}

#[test]
fn a_value_is_cut_where_it_stands_under_the_ceiling() {
  let cases: Cases = serde_json::from_str(CASES).unwrap();

  for case in &cases.values {
    assert_eq!(
      cap_value(&joined(&case.text), case.limit.unwrap_or(MAX_VALUE_CHARS)),
      joined(&case.expect),
      "{}",
      case.name
    );
  }
}

#[test]
fn counts_carry_the_thousands_mark() {
  assert_eq!(count_text(0), "0");
  assert_eq!(count_text(999), "999");
  assert_eq!(count_text(1_000), "1,000");
  assert_eq!(count_text(30_000), "30,000");
  assert_eq!(count_text(1_234_567), "1,234,567");
}
