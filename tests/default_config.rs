use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const VALID_CONFIG: &str = r#"
[blocks.core.env]
EDITOR = "nvim"
"#;

fn sandbox_home(root: &Path) -> PathBuf {
    root.join("home")
}

struct XdgTestEnv {
    root: PathBuf,
    config_home: PathBuf,
    config_dirs: PathBuf,
}

impl XdgTestEnv {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("conch-{name}-{}-{nonce}", std::process::id()));
        let config_home = root.join("config-home");
        let config_dirs = root.join("config-dirs");

        fs::create_dir_all(sandbox_home(&root)).unwrap();
        fs::create_dir_all(&config_home).unwrap();
        fs::create_dir_all(&config_dirs).unwrap();

        Self {
            root,
            config_home,
            config_dirs,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_conch"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", sandbox_home(&self.root))
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_CONFIG_DIRS", &self.config_dirs)
            .output()
            .expect("failed to run conch binary")
    }
}

impl Drop for XdgTestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn init_uses_flat_xdg_config_home_before_nested_config_path() {
    let env = XdgTestEnv::new("flat-home-precedence");
    write_file(&env.config_home.join("conch.toml"), VALID_CONFIG);
    write_file(
        &env.config_home.join("conch/config.toml"),
        "this is not valid toml",
    );

    let output = env.run(&["init", "fish"]);

    assert!(output.status.success(), "init failed: {:?}", output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("# block: core"));
}

#[test]
fn check_prefers_xdg_config_home_over_xdg_config_dirs() {
    let env = XdgTestEnv::new("home-before-dirs");
    write_file(&env.config_home.join("conch/config.toml"), VALID_CONFIG);
    write_file(
        &env.config_dirs.join("conch.toml"),
        "this is not valid toml",
    );

    let output = env.run(&["check"]);

    assert!(output.status.success(), "check failed: {:?}", output);
}

#[test]
fn missing_default_config_reports_xdg_search_paths() {
    let env = XdgTestEnv::new("missing-default-config");

    let output = env.run(&["check"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("default config file not found"));
    assert!(
        stderr.contains(&env.config_home.join("conch.toml").display().to_string()),
        "stderr did not mention config-home flat path: {stderr:?}"
    );
    assert!(
        stderr.contains(
            &env.config_home
                .join("conch/config.toml")
                .display()
                .to_string()
        ),
        "stderr did not mention config-home nested path: {stderr:?}"
    );
    assert!(
        stderr.contains(&env.config_dirs.join("conch.toml").display().to_string()),
        "stderr did not mention config-dirs flat path: {stderr:?}"
    );
}
