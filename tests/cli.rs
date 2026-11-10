use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("OPENPROMPT_TEST_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "openprompt-cli-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openprompt"));
        command
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env("NO_COLOR", "1");
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn help_and_version() {
    let sandbox = Sandbox::new();
    for args in [["--help"], ["--version"]] {
        let result = sandbox.run(&args);
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("openprompt"));
    }
}

#[test]
fn render_handles_unicode_spaces_negative_status_and_no_color() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "render",
        "--path",
        "demo's path/проект",
        "--status",
        "-9",
        "--duration",
        "2500",
        "--width",
        "120",
        "--no-color",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prompt = String::from_utf8(output.stdout).unwrap();
    assert!(prompt.contains("проект"));
    assert!(prompt.contains("exit:-9"));
    assert!(prompt.contains("2.5s"));
    assert!(!prompt.contains('\x1b'));
    assert!(prompt.ends_with("> "));
    assert!(!prompt.ends_with('\n'));
}

#[test]
fn configuration_creation_is_explicit_and_never_overwrites() {
    let sandbox = Sandbox::new();
    assert!(sandbox.run(&["render", "--path", "."]).status.success());
    let path = sandbox.0.join("openprompt/config.toml");
    assert!(!path.exists());
    assert!(sandbox.run(&["config", "init"]).status.success());
    let initial = fs::read(&path).unwrap();
    assert!(!sandbox.run(&["config", "init"]).status.success());
    assert_eq!(fs::read(&path).unwrap(), initial);
    assert!(sandbox.run(&["config", "check"]).status.success());
    fs::write(&path, "theme = 'broken'").unwrap();
    assert!(!sandbox.run(&["config", "check"]).status.success());
    let output = sandbox.run(&["render"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn explicit_missing_configuration_is_an_error() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["--config", "missing.toml", "render"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Cannot read"));
}

#[test]
fn all_theme_previews_and_init_work_without_config() {
    let sandbox = Sandbox::new();
    for theme in ["minimal", "classic", "blocks"] {
        let result = sandbox.run(&["render", "--path", "demo", "--theme", theme]);
        assert!(result.status.success());
        let prompt = String::from_utf8(result.stdout).unwrap();
        assert_eq!(
            prompt.lines().count(),
            if theme == "minimal" { 1 } else { 2 }
        );
    }
    let result = sandbox.run(&["init", "powershell"]);
    assert!(result.status.success());
    assert!(result.stdout.is_ascii());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("function global:prompt"));
    assert!(text.contains("FromBase64String"));
    assert!(!text.contains("__OPENPROMPT_EXECUTABLE__"));
    assert!(!sandbox.0.join("openprompt").exists());
}

#[test]
fn explicit_config_and_segment_toggles_apply() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("custom.toml"), "theme = 'minimal'\n[directory]\nenabled = false\n[git]\nenabled = false\n[duration]\nenabled = false\n[status]\nenabled = false\n").unwrap();
    let result = sandbox.run(&[
        "render",
        "--config",
        "custom.toml",
        "--status",
        "12",
        "--duration",
        "5000",
    ]);
    assert!(result.status.success());
    assert_eq!(result.stdout, b"> ");
}
