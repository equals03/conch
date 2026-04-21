use std::fs;
use std::process::Command;

fn run_init(shell: &str, config: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_conch"))
        .args(["init", shell, "--config", config])
        .output()
        .expect("failed to run conch binary");

    assert!(output.status.success(), "init failed for shell {shell}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn fish_output_matches_golden() {
    let actual = run_init("fish", "tests/fixtures/golden/basic.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/basic.fish").unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn bash_output_matches_golden() {
    let actual = run_init("bash", "tests/fixtures/golden/basic.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/basic.bash").unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn guarded_fish_output_matches_golden() {
    let actual = run_init("fish", "tests/fixtures/golden/guarded.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/guarded.fish").unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn guarded_bash_output_matches_golden() {
    let actual = run_init("bash", "tests/fixtures/golden/guarded.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/guarded.bash").unwrap();
    assert_eq!(actual, expected);
}
