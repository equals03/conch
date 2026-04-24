use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_conch"))
        .args(args)
        .output()
        .expect("failed to run conch binary")
}

fn stdout(output: std::process::Output) -> String {
    assert!(output.status.success(), "command did not succeed");
    String::from_utf8(output.stdout).unwrap()
}

fn stderr(output: std::process::Output) -> String {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn check_succeeds_on_simple_single_app_fixture() {
    let output = run(&[
        "check",
        "--config",
        "tests/fixtures/configs/simple-single.toml",
    ]);
    assert!(output.status.success());
}

#[test]
fn check_succeeds_on_simple_single_yaml_fixture() {
    let output = run(&[
        "check",
        "--config",
        "tests/fixtures/configs/simple-single.yaml",
    ]);
    assert!(output.status.success());
}

#[test]
fn check_succeeds_on_simple_single_json_fixture() {
    let output = run(&[
        "check",
        "--config",
        "tests/fixtures/configs/simple-single.json",
    ]);
    assert!(output.status.success());
}

#[test]
fn explain_shows_deterministic_order_for_ordered_multi_fixture() {
    let output = run(&[
        "explain",
        "fish",
        "--color",
        "never",
        "--config",
        "tests/fixtures/configs/ordered-multi.toml",
    ]);
    let text = stdout(output);

    let base = text.find("1. base").unwrap();
    let editor = text.find("2. editor").unwrap();
    let nvim = text.find("3. nvim").unwrap();
    assert!(base < editor && editor < nvim);
}

#[test]
fn check_reports_unordered_conflict_fixture() {
    let output = run(&[
        "check",
        "fish",
        "--config",
        "tests/fixtures/configs/unordered-conflict.toml",
    ]);
    let text = stderr(output);
    assert!(text.contains("merge conflict"));
    assert!(text.contains("vim"));
}

#[test]
fn check_reports_cycle_fixture() {
    let output = run(&["check", "--config", "tests/fixtures/configs/cycle.toml"]);
    let text = stderr(output);
    assert!(text.contains("cycle detected"));
}

#[test]
fn init_renders_shell_override_and_target_shell_predicates() {
    let fish = stdout(run(&[
        "init",
        "fish",
        "--config",
        "tests/fixtures/configs/shell-override.toml",
    ]));
    assert!(fish.contains("alias v=\"nvim\";"));
    assert!(!fish.contains("alias vi=\"nvim\";"));
    assert!(fish.contains("if begin; begin; set -q EDITOR; and test -n \"$EDITOR\"; end; and begin; set -q VISUAL; and test \"$VISUAL\" = \"nvim\"; end; end"));

    let bash = stdout(run(&[
        "init",
        "bash",
        "--config",
        "tests/fixtures/configs/shell-override.toml",
    ]));
    assert!(bash.contains("alias vi='nvim'"));
    assert!(!bash.contains("alias v='nvim'"));
    assert!(bash.contains("if false; then"));
}

#[test]
fn init_renders_shell_specific_source_guards() {
    let fish = stdout(run(&[
        "init",
        "fish",
        "--config",
        "tests/fixtures/golden/guarded.toml",
    ]));
    assert!(fish.contains("if not set -q __CONCH_FISH_SOURCED"));
    assert!(fish.contains("set -g __CONCH_SOURCED 1"));
    assert!(fish.contains("set -g __CONCH_FISH_SOURCED 1"));
    assert!(!fish.contains("if not set -q __CONCH_SOURCED"));

    let bash = stdout(run(&[
        "init",
        "bash",
        "--config",
        "tests/fixtures/golden/guarded.toml",
    ]));
    assert!(bash.contains("if [[ -z \"${__CONCH_BASH_SOURCED:-}\" ]]; then"));
    assert!(bash.contains("__CONCH_SOURCED=1"));
    assert!(bash.contains("__CONCH_BASH_SOURCED=1"));
    assert!(!bash.contains("[[ -z \"${__CONCH_SOURCED:-}\" ]]"));
}

#[test]
fn init_renders_structured_source_entries() {
    let fish = stdout(run(&[
        "init",
        "fish",
        "--config",
        "tests/fixtures/configs/structured-source.toml",
    ]));
    assert!(fish.contains("source \"$HOME/.shared-rc\""));
    assert!(fish.contains("\"starship\" \"init\" \"fish\" | source"));

    let bash = stdout(run(&[
        "init",
        "bash",
        "--config",
        "tests/fixtures/configs/structured-source.toml",
    ]));
    assert!(bash.contains("source \"$HOME/.shared-rc\""));
    assert!(bash.contains("eval \"$('starship' 'init' 'bash')\""));
}

#[test]
fn explain_preserves_deterministic_order_when_no_edges_exist() {
    let output = run(&[
        "explain",
        "fish",
        "--color",
        "never",
        "--config",
        "tests/fixtures/configs/no-edges-order.toml",
    ]);
    let text = stdout(output);

    let alpha = text.find("1. alpha").unwrap();
    let beta = text.find("2. beta").unwrap();
    let gamma = text.find("3. gamma").unwrap();
    assert!(alpha < beta && beta < gamma);
}

#[test]
fn explain_shows_path_operation_order_across_apps() {
    let output = run(&[
        "explain",
        "fish",
        "--color",
        "never",
        "--config",
        "tests/fixtures/configs/path-order.toml",
    ]);
    let text = stdout(output);

    let prepend = text.find("  1. base  prepend \"~/base/bin\"").unwrap();
    let move_front = text.find("  2. base  move_front \"~/shared/bin\"").unwrap();
    let append = text.find("  3. lang  append \"~/lang/bin\"").unwrap();
    let move_back = text.find("  4. lang  move_back \"~/shared/bin\"").unwrap();
    assert!(prepend < move_front && move_front < append && append < move_back);
}

#[test]
fn check_reports_parse_errors_for_malformed_predicates() {
    let output = run(&[
        "check",
        "--config",
        "tests/fixtures/configs/malformed-predicate.toml",
    ]);
    let text = stderr(output);
    assert!(text.contains("invalid predicate"));
    assert!(text.contains("block `broken` has invalid `when` predicate"));
}

#[test]
fn build_folds_host_predicates_but_keeps_runtime_guards() {
    let fish = stdout(run(&[
        "build",
        "fish",
        "--config",
        "tests/fixtures/configs/build-fold.toml",
    ]));
    assert!(fish.contains("if not set -q __CONCH_FISH_SOURCED"));
    assert!(fish.contains("if status is-interactive"));
    assert!(fish.contains("set -q HOME"));
    assert!(!fish.contains("test -e \"Cargo.toml\""));
    assert!(!fish.contains("test -d \"src\""));
    assert!(fish.contains("\"starship\" \"init\" \"fish\" | source"));

    let bash = stdout(run(&[
        "build",
        "bash",
        "--config",
        "tests/fixtures/configs/shell-override.toml",
    ]));
    assert!(!bash.contains("# block: core"));
    assert_eq!(bash.trim(), "# Generated by conch for bash.");
}

#[test]
fn explain_build_shows_folded_predicates() {
    let text = stdout(run(&[
        "explain",
        "fish",
        "build",
        "--color",
        "never",
        "--config",
        "tests/fixtures/configs/build-fold.toml",
    ]));
    assert!(text.contains("conch explain fish build"));
    assert!(text.contains("when        interactive"));
    assert!(text.contains("requires    env:HOME"));
    assert!(!text.contains("shell:fish"));
    assert!(!text.contains("file:Cargo.toml"));
    assert!(!text.contains("dir:src"));
}

#[test]
fn check_explain_without_shell_reports_both_targets() {
    let text = stdout(run(&[
        "check",
        "--explain",
        "--color",
        "never",
        "--config",
        "tests/fixtures/configs/simple-single.toml",
    ]));
    assert!(text.contains("conch explain fish check"));
    assert!(text.contains("conch explain bash check"));
}
