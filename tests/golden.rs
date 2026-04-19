use std::fs;
use std::process::Command;

fn run_build(shell: &str, config: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_conch"))
        .args(["build", shell, "--config", config])
        .output()
        .expect("failed to run conch binary");

    assert!(output.status.success(), "build failed for shell {shell}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn fish_output_matches_golden() {
    let actual = run_build("fish", "tests/fixtures/golden/basic.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/basic.fish").unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn bash_output_matches_golden() {
    let actual = run_build("bash", "tests/fixtures/golden/basic.toml");
    let expected = fs::read_to_string("tests/fixtures/golden/basic.bash").unwrap();
    assert_eq!(actual, expected);
}
