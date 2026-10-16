use {super::*, std::io::Read, std::process::Child};

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
fn successful_recipe_has_complete_lifecycle() {
  let output = Test::new()
    .args(["--events-file", EVENTS_FILE])
    .justfile("build:\n  @echo built")
    .stdout("built\n")
    .success();

  let events = read_events(&output.tempdir.path().join(EVENTS_FILE));
  let types: Vec<_> = events.iter().map(|event| event["type"].as_str()).collect();

  assert_eq!(
    types,
    [
      Some("run_started"),
      Some("recipe_started"),
      Some("recipe_finished"),
      Some("run_finished"),
    ]
  );
  assert_eq!(events[1]["recipe"], "build");
  assert_eq!(events[2]["recipe"], "build");
  assert_eq!(events[2]["outcome"], "success");
  assert_eq!(events[3]["outcome"], "success");
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

// Always reap the test process, including when an assertion or deadline fails.
struct RunningJust(Child);

impl Drop for RunningJust {
  fn drop(&mut self) {
    self.0.stdin.take();
    self.0.kill().ok();
    self.0.wait().ok();
  }
}

fn assert_running(child: &mut Child) {
  if let Some(status) = child.try_wait().unwrap() {
    let mut stderr = String::new();
    child
      .stderr
      .as_mut()
      .unwrap()
      .read_to_string(&mut stderr)
      .unwrap();
    panic!("just exited before emitting a live recipe-start event: {status}\n{stderr}");
  }
}

#[test]
fn events_are_visible_before_run_finishes() {
  let tmp = tempdir();
  fs::write(
    tmp.path().join("justfile"),
    "build:\n  @read -r release\n  @echo released\n",
  )
  .unwrap();

  let mut child = RunningJust(
    Command::new(JUST)
      .args(["--shell", "bash", "--events-file", EVENTS_FILE])
      .current_dir(tmp.path())
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .unwrap(),
  );

  let deadline = Instant::now() + Duration::from_secs(10);
  loop {
    assert_running(&mut child.0);

    // Only parse complete records: a concurrent write may end mid-record.
    let bytes = match fs::read(tmp.path().join(EVENTS_FILE)) {
      Ok(bytes) => bytes,
      Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
      Err(error) => panic!("failed to read live events: {error}"),
    };
    let recipe_started = bytes
      .split_inclusive(|byte| *byte == b'\n')
      .filter(|line| line.last() == Some(&b'\n'))
      .map(|line| serde_json::from_slice::<Value>(line).expect("invalid live JSON event"))
      .any(|event| event["type"] == "recipe_started" && event["recipe"] == "build");

    if recipe_started {
      break;
    }

    assert!(
      Instant::now() < deadline,
      "recipe-start event was not flushed while running"
    );
    thread::sleep(Duration::from_millis(10));
  }

  assert_running(&mut child.0);
  child
    .0
    .stdin
    .take()
    .unwrap()
    .write_all(b"release\n")
    .unwrap();

  let deadline = Instant::now() + Duration::from_secs(10);
  let status = loop {
    if let Some(status) = child.0.try_wait().unwrap() {
      break status;
    }
    assert!(
      Instant::now() < deadline,
      "just did not finish after releasing the recipe"
    );
    thread::sleep(Duration::from_millis(10));
  };

  assert!(
    status.success(),
    "just failed after releasing the recipe: {status}"
  );
  let mut stdout = String::new();
  child
    .0
    .stdout
    .as_mut()
    .unwrap()
    .read_to_string(&mut stdout)
    .unwrap();
  assert_eq!(stdout, "released\n");
}
