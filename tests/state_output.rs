use super::*;

// This first slice intentionally specifies no version, IDs, or timestamps.
const EVENTS_FILE: &str = "events.jsonl";

fn read_events(path: &Path) -> Vec<Value> {
  let bytes = fs::read(path).expect("failed to read events file");
  assert!(!bytes.is_empty(), "events file is empty");
  assert_eq!(bytes.last(), Some(&b'\n'), "missing final newline");

  str::from_utf8(&bytes)
    .expect("events are not UTF-8")
    .lines()
    .map(|line| {
      let event: Value = serde_json::from_str(line).expect("invalid JSON event");
      assert!(event.is_object(), "event is not a JSON object: {event}");
      event
    })
    .collect()
}

#[test]
fn events_file_contains_jsonl() {
  let output = Test::new()
    .args(["--events-file", EVENTS_FILE])
    .justfile("build:")
    .success();

  read_events(&output.tempdir.path().join(EVENTS_FILE));
}

#[test]
fn successful_recipe_reports_successful_outcomes() {
  let output = Test::new()
    .args(["--events-file", EVENTS_FILE])
    .justfile("build:\n  @echo built")
    .stdout("built\n")
    .success();

  let events = read_events(&output.tempdir.path().join(EVENTS_FILE));
  assert!(
    events.iter().any(|event| event["type"] == "recipe_finished"
      && event["recipe"] == "build"
      && event["outcome"] == "success"),
    "missing successful recipe outcome: {events:?}"
  );
  assert!(
    events
      .iter()
      .any(|event| event["type"] == "run_finished" && event["outcome"] == "success"),
    "missing successful run outcome: {events:?}"
  );
}

#[test]
fn events_do_not_mix_with_terminal_output() {
  let output = Test::new()
    .args(["--events-file", EVENTS_FILE])
    .justfile(
      "
        build:
          printf '%s\\n' '{\"type\":\"recipe_stdout\"}'
          @printf '%s\\n' '{\"type\":\"recipe_stderr\"}' >&2
      ",
    )
    .stdout("{\"type\":\"recipe_stdout\"}\n")
    .stderr(
      r#"
        printf '%s\n' '{"type":"recipe_stdout"}'
        {"type":"recipe_stderr"}
      "#,
    )
    .success();

  for event in read_events(&output.tempdir.path().join(EVENTS_FILE)) {
    assert_ne!(event["type"], "recipe_stdout");
    assert_ne!(event["type"], "recipe_stderr");
  }
}

#[test]
fn events_output_preserves_recipe_stdin() {
  let output = Test::new()
    .args(["--events-file", EVENTS_FILE])
    .justfile("build:\n  @read -r message; printf '%s\\n' \"$message\"")
    .stdin("hello\n")
    .stdout("hello\n")
    .success();

  read_events(&output.tempdir.path().join(EVENTS_FILE));
}
