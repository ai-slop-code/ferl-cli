//! Process-level tests of the local contract: parsing, consent, key sources, profile
//! storage and selection, private-file enforcement, and concurrent writers. Requests
//! are counted against a loopback mock so that "zero HTTP requests" is observable.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{Harness, KEY, MockServer, Reply, account_json};

fn quiet_server() -> MockServer {
    MockServer::start(|_| Reply::json(200, &account_json("ferl_cli_admin", "admin")))
}

fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn help_and_version_need_no_config_network_or_key() {
    let harness = Harness::new();
    let help = harness.run(&["--help"]);
    assert_eq!(help.status, 0);
    assert!(help.stdout.contains("Usage"));
    assert!(help.stdout.contains("create"));
    assert!(
        !help.stdout.contains("--scope"),
        "deferred options must not be exposed"
    );
    let version = harness.run(&["--version"]);
    assert_eq!(version.status, 0);
    assert_eq!(
        version.stdout,
        format!("ferl {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        !harness.config.exists(),
        "help must not create configuration"
    );
}

#[test]
fn no_arguments_prints_help_and_exits_zero() {
    let harness = Harness::new();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ferl"));
    command
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    let output = command.output().expect("run");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage"));
    assert!(!harness.config.exists());
}

#[test]
fn invalid_invocations_fail_locally_with_zero_requests() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let cases: &[&[&str]] = &[
        &["frobnicate"],
        &["example.com"],
        &["--profile", "work", "list", "--scope", "all"],
        &[
            "--profile",
            "work",
            "create",
            "https://a.example",
            "--custom-code",
            "x",
        ],
        &[
            "--profile",
            "work",
            "create",
            "https://a.example",
            "https://b.example",
        ],
        &["--profile", "work", "list", "--limit", "5", "--limit", "6"],
        &["--profile", "work", "list", "--limit", "201"],
        &["--profile", "work", "list", "--max-pages", "5"],
        &[
            "--profile",
            "work",
            "list",
            "--cursor",
            "a",
            "--cursor-file",
            "b",
        ],
        &["--profile", "work", "list", "--copy"],
        &["--profile", "work", "--title", "x", "list"],
        &[
            "--profile",
            "work",
            "create",
            "https://a.example",
            "--title",
            "  ",
        ],
        &["--profile", "work", "create", "https://user@a.example"],
        &[
            "--profile",
            "work",
            "create",
            "https://a.example",
            "--tag",
            "x",
            "--tag",
            "y",
            "--tag",
            "z",
            "--tag",
            "w",
            "--tag",
            "1",
            "--tag",
            "2",
            "--tag",
            "3",
            "--tag",
            "4",
            "--tag",
            "5",
            "--tag",
            "6",
            "--tag",
            "7",
            "--tag",
            "8",
            "--tag",
            "9",
            "--tag",
            "10",
            "--tag",
            "11",
            "--tag",
            "12",
            "--tag",
            "13",
            "--tag",
            "14",
            "--tag",
            "15",
            "--tag",
            "16",
            "--tag",
            "17",
            "--tag",
            "18",
            "--tag",
            "19",
            "--tag",
            "20",
            "--tag",
            "21",
        ],
        &["--profile", "work", "delete", "UPPER"],
        &[
            "--profile",
            "work",
            "delete",
            "https://s.example.com/a1b2c3d4/",
        ],
        &["--profile", "work", "--timeout", "0", "list"],
        &["--profile", "work", "profile", "list"],
        &["profile", "list", "--profile", "work"],
        &["profile", "list", "--timeout", "5"],
        &["--output", "xml", "list"],
        &[
            "--profile",
            "work",
            "--output",
            "json",
            "list",
            "--columns",
            "title",
        ],
        &["--key-env", "FERL_API_KEY", "list"],
    ];
    for case in cases {
        let run = harness.run(case);
        assert_eq!(
            run.status, 2,
            "{case:?} should be a usage error; stderr: {}",
            run.stderr
        );
        run.assert_no_secret();
    }
    assert_eq!(
        server.requests().len(),
        0,
        "usage failures must not reach the server"
    );
}

#[test]
fn delete_without_consent_is_refused_before_any_request() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let run = harness.run(&["--profile", "work", "delete", "a1b2c3d4"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("--yes"));
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn business_commands_without_any_profile_are_configuration_errors() {
    let harness = Harness::new();
    let run = harness.run(&["list"]);
    assert_eq!(run.status, 3);
    assert!(run.stdout.is_empty());
}

#[test]
fn json_usage_errors_are_single_json_lines_on_stderr() {
    let harness = Harness::new();
    let run = harness.run(&["--output", "json", "frobnicate"]);
    assert_eq!(run.status, 2);
    assert!(run.stdout.is_empty());
    let records = run.json_stderr_lines();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["type"], "error");
    assert_eq!(records[0]["exit_code"], 2);
}

#[test]
fn profile_listing_and_show_never_reveal_the_key_and_storage_is_private() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);

    let list = harness.run(&["--output", "json", "profile", "list"]);
    list.assert_no_secret();
    assert_eq!(list.status, 0);
    let value = list.json_stdout();
    assert_eq!(value["data"]["items"][0]["name"], "work");
    assert_eq!(value["data"]["items"][0]["credential_present"], true);
    assert_eq!(
        value["data"]["items"][0]["credential_storage"],
        "private_file"
    );
    assert_eq!(value["data"]["default_profile"], "work");

    let show = harness.run(&["profile", "show"]);
    show.assert_no_secret();
    assert_eq!(show.status, 0);
    assert!(show.stdout.contains("name: work"));

    let config_text = std::fs::read_to_string(&harness.config).expect("config");
    assert!(
        !config_text.contains(KEY),
        "config must not contain the key"
    );
    let credentials = harness.path("config.toml.credentials.toml");
    assert!(
        std::fs::read_to_string(&credentials)
            .expect("credentials")
            .contains(KEY)
    );
    assert_eq!(mode_of(&harness.config), 0o600);
    assert_eq!(mode_of(&credentials), 0o600);
    assert_eq!(mode_of(harness.dir.path()), 0o700);
}

#[test]
fn key_sources_are_exclusive_and_private() {
    let server = quiet_server();
    let harness = Harness::new();
    let key_file = harness.write_private("work.key", &format!("{KEY}\n"));
    let run = harness.run(&[
        "profile",
        "add",
        "work",
        "--server",
        &server.url(),
        "--allow-http-loopback",
        "--key-file",
        &key_file.to_string_lossy(),
        "--key-env",
        "FERL_API_KEY",
    ]);
    assert_eq!(run.status, 2, "two key sources are a usage error");

    let public_key = harness.path("public.key");
    std::fs::write(&public_key, format!("{KEY}\n")).expect("write");
    std::fs::set_permissions(&public_key, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let unsafe_run = harness.run(&[
        "profile",
        "add",
        "work",
        "--server",
        &server.url(),
        "--allow-http-loopback",
        "--key-file",
        &public_key.to_string_lossy(),
    ]);
    assert_eq!(
        unsafe_run.status, 3,
        "group/other-readable key files are refused"
    );
    unsafe_run.assert_no_secret();

    let missing_env = harness.run(&[
        "profile",
        "add",
        "work",
        "--server",
        &server.url(),
        "--allow-http-loopback",
        "--key-env",
        "FERL_TEST_DEFINITELY_UNSET_VARIABLE",
    ]);
    assert_eq!(missing_env.status, 3);

    let good = harness.run(&[
        "profile",
        "add",
        "work",
        "--server",
        &server.url(),
        "--allow-http-loopback",
        "--key-file",
        &key_file.to_string_lossy(),
        "--default",
    ]);
    assert_eq!(good.status, 0, "{}", good.stderr);
    good.assert_no_secret();
    assert_eq!(
        server.requests().len(),
        0,
        "profile add without --validate is local-only"
    );
}

#[test]
fn env_key_override_is_in_memory_and_never_persisted() {
    let server = MockServer::start(|_| Reply::json(200, r#"{"items":[],"next_cursor":null}"#));
    let harness = Harness::new();
    assert_eq!(
        harness
            .add_profile("work", &server.url(), "ferl_ak_v1_STORED_KEY_ABCDEFG")
            .status,
        0
    );
    let before = std::fs::read(harness.path("config.toml.credentials.toml")).expect("read");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ferl"));
    command
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("FERL_OVERRIDE_TEST_KEY", KEY)
        .arg("--config")
        .arg(&harness.config)
        .args([
            "--non-interactive",
            "--profile",
            "work",
            "--key-env",
            "FERL_OVERRIDE_TEST_KEY",
            "--output",
            "json",
            "list",
        ]);
    let output = command.output().expect("run");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server.requests()[0].header("Authorization"),
        Some(&*format!("Bearer {KEY}"))
    );
    let after = std::fs::read(harness.path("config.toml.credentials.toml")).expect("read");
    assert_eq!(before, after, "runtime overrides must not persist");
}

#[test]
fn unsafe_config_permissions_and_symlinks_fail_without_repair() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    std::fs::set_permissions(&harness.config, std::fs::Permissions::from_mode(0o640))
        .expect("chmod");
    let run = harness.run(&["--profile", "work", "list"]);
    assert_eq!(run.status, 3);
    assert_eq!(
        mode_of(&harness.config),
        0o640,
        "unsafe permissions must not be repaired"
    );
    run.assert_no_secret();

    std::fs::set_permissions(&harness.config, std::fs::Permissions::from_mode(0o600))
        .expect("chmod");
    let target = harness.path("elsewhere.toml");
    std::fs::rename(&harness.config, &target).expect("move");
    std::os::unix::fs::symlink(&target, &harness.config).expect("symlink");
    let linked = harness.run(&["--profile", "work", "list"]);
    assert_eq!(linked.status, 3, "symbolic links are refused");
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn unknown_schema_versions_are_rejected_not_migrated() {
    let harness = Harness::new();
    harness.write_private("config.toml", "schema_version = 2\n");
    let run = harness.run(&["profile", "list"]);
    assert_eq!(run.status, 3);
    assert_eq!(
        std::fs::read_to_string(&harness.config).expect("read"),
        "schema_version = 2\n"
    );
}

#[test]
fn credential_bound_to_another_origin_is_never_used() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let credentials = harness.path("config.toml.credentials.toml");
    let text = std::fs::read_to_string(&credentials).expect("read");
    let tampered = text.replace(&server.url(), "http://127.0.0.1:1");
    std::fs::write(&credentials, tampered).expect("write");
    let run = harness.run(&["--profile", "work", "list"]);
    assert_eq!(
        run.status, 3,
        "a mismatched credential binding must not be used"
    );
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn removing_the_default_clears_it_instead_of_selecting_another_profile() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("alpha", &server.url(), KEY).status, 0);
    let second = harness.run_with(
        &[
            "profile",
            "add",
            "beta",
            "--server",
            &server.url(),
            "--allow-http-loopback",
            "--key-stdin",
        ],
        Some(format!("{KEY}\n").as_bytes()),
    );
    assert_eq!(second.status, 0, "{}", second.stderr);
    let default_before = harness
        .run(&["--output", "json", "profile", "list"])
        .json_stdout();
    assert_eq!(
        default_before["data"]["default_profile"], "alpha",
        "first profile becomes default"
    );
    assert_eq!(harness.run(&["profile", "use", "beta"]).status, 0);
    let remove = harness.run(&["profile", "remove", "beta", "--yes"]);
    assert_eq!(remove.status, 0, "{}", remove.stderr);
    let after = harness
        .run(&["--output", "json", "profile", "list"])
        .json_stdout();
    assert_eq!(after["data"]["default_profile"], serde_json::Value::Null);
    let no_default = harness.run(&["--profile", "beta", "list"]);
    assert_eq!(no_default.status, 3, "a removed profile is not selectable");
    let implicit = harness.run(&["list"]);
    assert_eq!(implicit.status, 3, "no fallback to another profile");
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn duplicate_profile_names_are_rejected_without_overwriting() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let again = harness.add_profile("work", &server.url(), "ferl_ak_v1_OTHER_0000000000000000");
    assert_eq!(again.status, 2);
    let credentials =
        std::fs::read_to_string(harness.path("config.toml.credentials.toml")).expect("read");
    assert!(credentials.contains(KEY));
    assert!(!credentials.contains("OTHER_0000"));
}

#[test]
fn profile_set_server_change_requires_a_key_and_starts_a_new_identity() {
    let first = quiet_server();
    let second = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &first.url(), KEY).status, 0);
    let before = harness
        .run(&["--output", "json", "profile", "show", "work"])
        .json_stdout();
    let old_id = before["data"]["profile"]["id"]
        .as_str()
        .expect("id")
        .to_owned();

    let no_key = harness.run(&[
        "profile",
        "set",
        "work",
        "--server",
        &second.url(),
        "--allow-http-loopback",
    ]);
    assert_eq!(no_key.status, 2);

    let run = harness.run_with(
        &[
            "profile",
            "set",
            "work",
            "--server",
            &second.url(),
            "--allow-http-loopback",
            "--key-stdin",
        ],
        Some("ferl_ak_v1_NEW_SERVER_KEY_000000000\n".as_bytes()),
    );
    assert_eq!(run.status, 0, "{}", run.stderr);
    let after = harness
        .run(&["--output", "json", "profile", "show", "work"])
        .json_stdout();
    assert_ne!(after["data"]["profile"]["id"].as_str().expect("id"), old_id);
    assert_eq!(after["data"]["profile"]["server_origin"], second.url());
    let credentials =
        std::fs::read_to_string(harness.path("config.toml.credentials.toml")).expect("read");
    assert!(
        !credentials.contains(KEY),
        "the old key must not be carried to the new origin"
    );
}

#[test]
fn profile_set_key_replaces_only_the_saved_key() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let before = harness
        .run(&["--output", "json", "profile", "show", "work"])
        .json_stdout();
    let id = before["data"]["profile"]["id"].clone();
    let rotated = "ferl_ak_v1_ROTATED_0123456789abcdef";
    let run = harness.run_with(
        &["profile", "set-key", "work", "--key-stdin"],
        Some(format!("{rotated}\n").as_bytes()),
    );
    assert_eq!(run.status, 0, "{}", run.stderr);
    run.assert_no_secret();
    let after = harness
        .run(&["--output", "json", "profile", "show", "work"])
        .json_stdout();
    assert_eq!(after["data"]["profile"]["id"], id);
    let credentials =
        std::fs::read_to_string(harness.path("config.toml.credentials.toml")).expect("read");
    assert!(credentials.contains(rotated));
    assert!(!credentials.contains(KEY));
    assert_eq!(
        server.requests().len(),
        0,
        "set-key without --validate sends nothing"
    );
}

#[test]
fn profile_check_uses_the_saved_key_and_the_account_read_only() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let run = harness.run(&["--output", "json", "profile", "check", "work"]);
    run.assert_no_secret();
    assert_eq!(run.status, 0, "{}", run.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path(), "/api/v1/me");
    assert_eq!(
        requests[0].header("Authorization"),
        Some(&*format!("Bearer {KEY}"))
    );
    assert_eq!(run.json_stdout()["data"]["account"]["role"], "admin");
}

#[test]
fn wizard_is_refused_without_a_terminal() {
    let harness = Harness::new();
    let run = harness.run(&["profile"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("interactive terminal"));
    let use_run = harness.run(&["profile", "use"]);
    assert_eq!(use_run.status, 2);
}

#[test]
fn profile_show_without_a_default_is_a_configuration_error() {
    let harness = Harness::new();
    assert_eq!(harness.run(&["profile", "show"]).status, 3);
}

#[test]
fn stray_staging_files_are_ignored_and_never_selected() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    harness.write_private(".ferl-crashed.tmp", "partial garbage");
    let run = harness.run(&["--output", "json", "profile", "list"]);
    assert_eq!(run.status, 0);
    assert_eq!(
        run.json_stdout()["data"]["items"]
            .as_array()
            .expect("items")
            .len(),
        1
    );
}

#[test]
fn concurrent_writers_do_not_lose_committed_profiles() {
    let server = quiet_server();
    let harness = std::sync::Arc::new(Harness::new());
    assert_eq!(harness.add_profile("seed", &server.url(), KEY).status, 0);
    let url = server.url();
    let handles: Vec<_> = (0..6)
        .map(|i| {
            let harness = harness.clone();
            let url = url.clone();
            std::thread::spawn(move || harness.add_profile(&format!("p{i}"), &url, KEY).status)
        })
        .collect();
    let statuses: Vec<i32> = handles
        .into_iter()
        .map(|h| h.join().expect("join"))
        .collect();
    assert!(statuses.iter().all(|s| *s == 0), "statuses: {statuses:?}");
    let listing = harness
        .run(&["--output", "json", "profile", "list"])
        .json_stdout();
    let names: Vec<String> = listing["data"]["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["name"].as_str().expect("name").to_owned())
        .collect();
    assert_eq!(names.len(), 7, "every committed add is present: {names:?}");
    let credentials =
        std::fs::read_to_string(harness.path("config.toml.credentials.toml")).expect("read");
    assert_eq!(credentials.matches("api_key").count(), 7);
}

#[test]
fn human_output_is_plain_text_without_terminal_escapes() {
    let server = quiet_server();
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    let run = harness.run(&["profile", "list"]);
    assert_eq!(run.status, 0);
    assert!(!run.stdout.contains('\u{1b}'));
    assert!(run.stdout.contains("work"));
}
