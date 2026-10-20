use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jev"))
        .args(args)
        .env("TYPESAFE_API_KEY", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn help_lists_three_primitives_and_streaming_flags() {
    let output = run(&["--help"], "");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for name in ["choice", "score", "noul"] {
        assert!(text.contains(name));
    }
    let output = run(&["noul", "--help"], "");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for flag in ["jsonl", "--concurrency", "--question-file"] {
        assert!(text.contains(flag));
    }
}

#[test]
fn detailed_evaluation_help_is_self_contained_but_short_help_stays_concise() {
    for command in ["choice", "score", "noul"] {
        let output = run(&[command, "--help"], "");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let long = String::from_utf8(output.stdout).unwrap();
        for section in [
            "EXAMPLES:",
            "QUESTION FILE (",
            "ANSWER EXAMPLE:",
            "INPUT:",
            "OUTPUT:",
            "QUESTION FILES:",
            "STREAMING AND FAILURES:",
            "AUTHENTICATION AND PRIVACY:",
            "TYPESAFE_API_KEY",
            "input preserves the original JSON value",
            "result is the API answer",
            "source data",
            "input_tokens",
            "partial",
            "Exit codes:",
            "Retry-After",
            "jev-latest",
            "mutually exclusive",
        ] {
            assert!(long.contains(section), "{command} help missing {section}");
        }
        let output = run(&[command, "-h"], "");
        assert!(output.status.success());
        let short = String::from_utf8(output.stdout).unwrap();
        assert!(!short.contains("EXAMPLES:"));
        assert!(short.len() < long.len() / 2);

        // The documented question file is accepted by the actual CLI. An empty
        // credential makes this stop before any network call or keychain lookup.
        let question = long
            .lines()
            .find(|line| line.trim_start().starts_with("{\"type\":"))
            .expect("question JSON example")
            .trim();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(question.as_bytes()).unwrap();
        let output = run(
            &[command, "--question-file", file.path().to_str().unwrap()],
            "state",
        );
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("API key is empty")
        );
    }
}

#[test]
fn root_and_auth_help_explain_setup_without_credentials() {
    for args in [
        vec!["--help"],
        vec!["auth", "--help"],
        vec!["auth", "set-key", "--help"],
        vec!["auth", "delete-key", "--help"],
    ] {
        let output = run(&args, "");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        for required in ["TYPESAFE_API_KEY", "stderr", "Exit codes:"] {
            assert!(text.contains(required), "{args:?} missing {required}");
        }
    }
    let output = run(&["-h"], "");
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("QUICK START:")
    );
}

#[test]
fn rejects_invalid_arguments_before_credentials_or_network() {
    for (args, expected) in [
        (vec!["choice", "Pick"], "choice requires"),
        (
            vec!["choice", "Pick", "--option", "a", "--option", "a"],
            "duplicate option",
        ),
        (vec!["score", "Rate", "--level", "low"], "score requires"),
        (vec!["noul", ""], "instructions must not be empty"),
        (vec!["noul", "Yes?", "--concurrency", "0"], "invalid value"),
        (vec!["noul", "Yes?", "--retries", "11"], "invalid value"),
    ] {
        let output = run(&args, "state");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn question_file_is_typed_and_exclusive_with_inline_flags() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "{{\"type\":\"choice\",\"instructions\":{{\"question\":\"Pick\"}},\"criteria\":{{\"a\":null}}}}").unwrap();
    let path = file.path().to_str().unwrap();
    for args in [
        vec!["noul", "--question-file", path],
        vec!["choice", "Pick", "--question-file", path],
        vec!["choice", "--question-file", path, "--option", "b"],
    ] {
        let output = run(&args, "state");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("API key")
        );
    }
    let output = run(&["choice", "--question-file", path], "state");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("API key is empty")
    );
}

#[test]
fn empty_environment_key_does_not_fall_back_to_keychain() {
    let output = run(&["noul", "Yes?"], "state");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("API key is empty")
    );
}
