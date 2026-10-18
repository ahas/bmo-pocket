use bmo_tool::{AtLeastOne, NonEmpty, Positive};

#[test]
fn a_positive_number_and_a_list_of_one_are_held_to_their_bounds() {
  assert!(serde_json::from_value::<Positive>(0.into()).is_err());
  assert!(serde_json::from_value::<Positive>(1.5.into()).is_err());
  assert_eq!(serde_json::from_value::<Positive>(758.into()).unwrap(), Positive(758));
  assert!(serde_json::from_value::<AtLeastOne<String>>(serde_json::json!([])).is_err());
  assert_eq!(
    serde_json::from_value::<AtLeastOne<String>>(serde_json::json!(["\u{e9}"])).unwrap().0,
    ["\u{e9}"]
  );
}

#[test]
fn an_empty_string_is_refused_and_any_other_is_taken_as_it_stands() {
  assert!(serde_json::from_value::<NonEmpty>("".into()).is_err());
  assert_eq!(&*serde_json::from_value::<NonEmpty>("\u{e9}".into()).unwrap(), "\u{e9}");
}
