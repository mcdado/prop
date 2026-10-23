use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_CONFIG: AtomicU64 = AtomicU64::new(0);

struct TestConfig {
    directory: PathBuf,
    path: PathBuf,
}

impl TestConfig {
    fn new(contents: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "prop-cli-{}-{timestamp}-{}",
            std::process::id(),
            NEXT_CONFIG.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, contents).unwrap();
        Self { directory, path }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_prop"))
            .arg("--config")
            .arg(&self.path)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for TestConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn rejects_zero_concurrency_and_timeout_during_validation() {
    for setting in ["concurrency", "timeout_ms"] {
        let config = TestConfig::new(&format!("[settings]\n{setting} = 0\n"));
        let output = config.run(&["config", "--validate"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("settings.{setting}")));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("config_valid=true"));
    }
}

#[test]
fn rejects_a_missing_explicit_configuration_file() {
    let config = TestConfig::new("");
    fs::remove_file(&config.path).unwrap();
    let output = config.run(&["config", "--validate"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read config file"));
    assert!(output.stdout.is_empty());
}

#[test]
fn validates_an_existing_configuration_file() {
    let config = TestConfig::new("[settings]\nconcurrency = 1\ntimeout_ms = 10\n");
    let output = config.run(&["config", "--validate"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("config_valid=true"));
}

#[test]
fn rejects_a_filter_that_matches_only_disabled_resolvers() {
    let config = TestConfig::new(
        "[[overrides]]\nid = 'google-primary'\nenabled = false\ntags = ['disabled-test']\n",
    );
    let output = config.run(&["check", "example.com", "--tag", "disabled-test", "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no resolvers matched"));
    assert!(output.stdout.is_empty());
}

#[test]
fn rejects_a_check_when_all_resolvers_are_disabled() {
    let config = TestConfig::new("");
    let output = config.run(&["servers", "--json"]);
    assert!(output.status.success());
    let servers: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!servers.is_empty());
    let overrides = servers
        .iter()
        .map(|server| {
            format!(
                "[[overrides]]\nid = {:?}\nenabled = false\n",
                server["id"].as_str().unwrap()
            )
        })
        .collect::<String>();
    fs::write(&config.path, overrides).unwrap();
    let output = config.run(&["check", "example.com", "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no resolvers matched"));
    assert!(output.stdout.is_empty());
}

#[test]
fn generates_shell_completions_without_loading_invalid_configuration() {
    let config = TestConfig::new("this is not valid TOML [");
    for (shell, registration) in [
        ("bash", "complete -F _prop"),
        ("zsh", "#compdef prop"),
        ("fish", "complete -c prop"),
        (
            "powershell",
            "Register-ArgumentCompleter -Native -CommandName 'prop'",
        ),
    ] {
        let output = config.run(&["completions", shell]);
        assert!(output.status.success(), "{shell}: {output:?}");
        assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(
            script.contains(registration),
            "{shell}: missing registration"
        );
        for command in ["check", "trace-parent", "servers", "config", "completions"] {
            assert!(script.contains(command), "{shell}: missing {command}");
        }
        for option in [
            "record-type",
            "region",
            "provider",
            "tag",
            "json",
            "config",
            "timeout-ms",
        ] {
            assert!(script.contains(option), "{shell}: missing {option}");
        }
        // The standard PowerShell generator completes names, not argument values.
        if shell != "powershell" {
            for value in ["aaaa", "cname", "mx", "ns", "txt"] {
                assert!(
                    script.contains(value),
                    "{shell}: missing record type {value}"
                );
            }
        }
    }
}

#[test]
fn generates_completions_without_an_existing_configuration_file() {
    let config = TestConfig::new("");
    fs::remove_file(&config.path).unwrap();
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let output = config.run(&["completions", shell]);
        assert!(output.status.success(), "{shell}: {output:?}");
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn requires_a_supported_shell_for_completions() {
    let config = TestConfig::new("");
    for args in [
        vec!["completions"],
        vec!["completions", "unknown"],
        vec!["completions", "elvish"],
    ] {
        let output = config.run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("<SHELL>"));
    }
}

#[test]
fn rejects_invalid_command_line_timeouts() {
    let config = TestConfig::new("");
    for command in ["check", "trace-parent"] {
        for value in ["0", "-1", "invalid", "18446744073709551616"] {
            let option = format!("--timeout-ms={value}");
            let output = config.run(&[command, "example.com", &option]);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{command} {option}: {output:?}"
            );
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("--timeout-ms"));
        }
    }
}
