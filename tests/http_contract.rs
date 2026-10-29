//! Deterministic HTTP fixtures: exact request shapes, status mapping, retries, mutation
//! outcomes, pagination, large cursors, and exact integers (business spec §§6–10).

mod common;

use common::{Harness, KEY, MockServer, Reply, account_json, error_json, link_json, page_json};

const CODE: &str = "a1b2c3d4";

fn created_reply(code: &str) -> Reply {
    Reply::json(
        201,
        &format!(r#"{{"link":{}}}"#, link_json(code, Some("Team docs"), "0")),
    )
}

fn profile_for(server: &MockServer) -> Harness {
    let harness = Harness::new();
    let added = harness.add_profile("work", &server.url(), KEY);
    assert_eq!(added.status, 0, "profile add failed: {}", added.stderr);
    harness
}

#[test]
fn create_sends_only_the_destination_and_prints_the_returned_url() {
    let server = MockServer::start(|_| created_reply(CODE));
    let harness = profile_for(&server);
    let run = harness.run(&[
        "--profile",
        "work",
        "https://example.com/docs?edition=2#intro",
    ]);
    run.assert_no_secret();
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, format!("https://s.example.com/{CODE}\n"));

    let posts = server.requests();
    assert_eq!(posts.len(), 1, "one POST only");
    let post = &posts[0];
    assert_eq!(post.method, "POST");
    assert_eq!(post.path(), "/api/v1/links");
    assert_eq!(
        post.header("Authorization"),
        Some(&*format!("Bearer {KEY}"))
    );
    assert_eq!(post.header("Content-Type"), Some("application/json"));
    assert_eq!(post.header("Accept"), Some("application/json"));
    let body = String::from_utf8(post.body.clone()).expect("utf8");
    assert_eq!(
        body,
        r#"{"destination_url":"https://example.com/docs?edition=2#intro"}"#
    );
}

#[test]
fn title_and_tags_are_sent_as_supplied_and_shorthand_matches_create() {
    let server = MockServer::start(|_| created_reply(CODE));
    let harness = profile_for(&server);
    let shorthand = harness.run(&[
        "--profile",
        "work",
        "https://example.com/x",
        "--title",
        "  Team docs ",
        "--tag",
        "Work",
        "--tag",
        "reference,internal",
        "--no-copy",
    ]);
    assert_eq!(shorthand.status, 0, "{}", shorthand.stderr);
    let explicit = harness.run(&[
        "--profile",
        "work",
        "create",
        "https://example.com/x",
        "--title",
        "  Team docs ",
        "--tag",
        "Work",
        "--tag",
        "reference,internal",
        "--no-copy",
    ]);
    assert_eq!(explicit.status, 0);
    let posts = server.requests();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0].body, posts[1].body,
        "shorthand and create must send identical bodies"
    );
    let body: serde_json::Value = serde_json::from_slice(&posts[0].body).expect("json");
    assert_eq!(body["title"], "  Team docs ");
    assert_eq!(
        body["tags"],
        serde_json::json!(["Work", "reference,internal"])
    );
    assert!(body.get("short_code").is_none());
    assert!(body.get("management_token").is_none());
}

#[test]
fn unauthorized_creation_is_exit_4_with_one_attempt_and_no_fallback() {
    let server = MockServer::start(|_| {
        Reply::json(
            401,
            &error_json(
                "invalid_credentials",
                "The supplied credentials are invalid.",
                "req-401",
            ),
        )
        .with_header("WWW-Authenticate", "Bearer")
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    run.assert_no_secret();
    assert_eq!(run.status, 4);
    assert!(run.stdout.is_empty());
    assert_eq!(server.count("POST"), 1, "no retry or anonymous fallback");
    assert_eq!(server.requests().len(), 1);
    let error = &run.json_stderr_lines()[0];
    assert_eq!(error["mutation_outcome"], "rejected");
    assert_eq!(error["http_status"], 401);
    assert_eq!(error["request_id"], "req-401");
}

#[test]
fn rate_limited_creation_reports_the_whole_second_wait() {
    let server = MockServer::start(|_| {
        Reply::json(
            429,
            &error_json("rate_limited", "Too many attempts.", "req-429"),
        )
        .with_header("Retry-After", "7")
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    assert_eq!(run.status, 8);
    assert_eq!(run.json_stderr_lines()[0]["retry_after_seconds"], 7);
    assert_eq!(server.count("POST"), 1, "429 is never retried for writes");
}

#[test]
fn dropped_create_acknowledgement_is_uncertain_and_not_replayed() {
    let server = MockServer::start(|_| Reply::Drop);
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    run.assert_no_secret();
    assert_eq!(run.status, 13, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty());
    assert_eq!(
        server.count("POST"),
        1,
        "exactly one POST despite the lost response"
    );
    let error = &run.json_stderr_lines()[0];
    assert_eq!(error["mutation_outcome"], "unknown");
    assert_eq!(error["exit_code"], 13);
}

#[test]
fn service_unavailable_storage_failure_on_create_is_uncertain_not_retried() {
    let server = MockServer::start(|_| {
        Reply::json(
            503,
            &error_json(
                "storage_unavailable",
                "The database is unavailable.",
                "req-s",
            ),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    assert_eq!(run.status, 13);
    assert_eq!(server.count("POST"), 1);
}

#[test]
fn code_generation_unavailable_is_a_definitive_server_rejection() {
    let server = MockServer::start(|_| {
        Reply::json(
            503,
            &error_json(
                "code_generation_unavailable",
                "No code was available.",
                "req-c",
            ),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    assert_eq!(run.status, 10);
    assert_eq!(run.json_stderr_lines()[0]["mutation_outcome"], "rejected");
    assert_eq!(server.count("POST"), 1);
}

#[test]
fn committed_creation_with_unusable_body_is_protocol_failure_with_committed_outcome() {
    let server = MockServer::start(|_| Reply::json(201, r#"{"link":{"id":"not-a-link"}}"#));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    assert_eq!(run.status, 11);
    assert!(run.stdout.is_empty());
    assert_eq!(run.json_stderr_lines()[0]["mutation_outcome"], "committed");
    assert_eq!(server.count("POST"), 1);
}

#[test]
fn anonymous_creation_shape_is_rejected_for_an_account() {
    let body = format!(
        r#"{{"link":{},"management_token":"ferl_mt_v1_nope"}}"#,
        link_json(CODE, None, "0")
    );
    let server = MockServer::start(move |_| Reply::json(201, &body));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/x", "--no-copy"]);
    assert_eq!(run.status, 11);
    assert!(!run.stderr.contains("ferl_mt_v1_nope"));
    assert!(!run.stdout.contains("ferl_mt_v1_nope"));
}

#[test]
fn reads_retry_transient_unavailable_responses_up_to_three_attempts() {
    let server =
        MockServer::start(|_| Reply::json(503, &error_json("storage_unavailable", "down", "r")));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 10);
    assert_eq!(
        server.requests().len(),
        3,
        "one attempt plus two bounded retries"
    );
    assert!(run.stdout.is_empty());
}

#[test]
fn reads_do_not_retry_internal_errors() {
    let server =
        MockServer::start(|_| Reply::json(500, &error_json("internal_error", "boom", "r")));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 10);
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn redirects_are_refused_and_never_followed() {
    let elsewhere = MockServer::start(|_| Reply::json(200, r#"{"items":[],"next_cursor":null}"#));
    let location = format!("{}/api/v1/links", elsewhere.url());
    let server = MockServer::start(move |_| Reply::Respond {
        status: 302,
        headers: vec![("Location".into(), location.clone())],
        body: Vec::new(),
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    run.assert_no_secret();
    assert_eq!(run.status, 11);
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        elsewhere.requests().len(),
        0,
        "credentials must never follow a redirect"
    );
}

#[test]
fn delete_sends_a_bodyless_request_and_accepts_an_empty_204() {
    let server = MockServer::start(|_| Reply::Respond {
        status: 204,
        headers: vec![],
        body: vec![],
    });
    let harness = profile_for(&server);
    let run = harness.run(&["--profile", "work", "delete", CODE, "--yes"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains(&format!("Deleted {CODE}.")));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "DELETE");
    assert_eq!(requests[0].target, format!("/api/v1/links/{CODE}"));
    assert!(requests[0].body.is_empty());
    assert!(
        requests[0]
            .header("Content-Length")
            .is_none_or(|v| v == "0")
    );
}

#[test]
fn repeated_delete_not_found_is_exit_6_not_success() {
    let server =
        MockServer::start(|_| Reply::json(404, &error_json("not_found", "Not found.", "req-404")));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["delete", CODE, "--yes"]);
    assert_eq!(run.status, 6);
    assert!(run.stdout.is_empty());
    assert_eq!(run.json_stderr_lines()[0]["mutation_outcome"], "rejected");
    assert_eq!(server.count("DELETE"), 1);
}

#[test]
fn delete_by_public_short_url_uses_the_bound_api_origin_with_one_request() {
    let server = MockServer::start(|_| Reply::Respond {
        status: 204,
        headers: vec![],
        body: vec![],
    });
    let harness = profile_for(&server);
    let short_url = format!("{}/{CODE}", server.url());
    let run = harness.run(&["--profile", "work", "delete", &short_url, "--yes"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        server.requests().len(),
        1,
        "no preflight or public redirect call"
    );
}

#[test]
fn list_query_is_encoded_once_and_defaults_are_own_scope_and_limit_50() {
    let server = MockServer::start(|_| Reply::json(200, &page_json(&[], None)));
    let harness = profile_for(&server);
    let text = "50%_off \\ & a+b,東京";
    let run = harness.run_json("work", &["list", "--query", &format!("  {text}  ")]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let request = &server.requests()[0];
    assert_eq!(request.method, "GET");
    assert!(request.body.is_empty(), "GET has no body");
    assert_eq!(request.query_values("q"), vec![text.to_owned()]);
    assert_eq!(request.query_values("scope"), vec!["own".to_owned()]);
    assert_eq!(request.query_values("limit"), vec!["50".to_owned()]);
    assert!(request.query_values("sort").is_empty());
    assert!(
        request.target.contains("q=50%25_off+%5C+%26+a%2Bb%2C"),
        "target: {}",
        request.target
    );
    let run_json = run.json_stdout();
    assert_eq!(run_json["meta"]["complete"], true);
}

#[test]
fn exact_integers_and_nulls_survive_the_json_output() {
    let links = vec![
        link_json("aaaa1111", None, "9007199254740993"),
        link_json("bbbb2222", Some("max"), "9223372036854775807"),
    ];
    let body = page_json(&links, None);
    let server = MockServer::start(move |_| Reply::json(200, &body));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(run.stdout.contains("9007199254740993"));
    assert!(run.stdout.contains("9223372036854775807"));
    let value = run.json_stdout();
    assert_eq!(value["data"]["items"][0]["title"], serde_json::Value::Null);
    assert_eq!(
        value["data"]["items"][0]["visit_count"],
        9007199254740993_i64
    );
    assert_eq!(
        value["data"]["items"][1]["visit_count"],
        9223372036854775807_i64
    );
    assert_eq!(
        value["data"]["items"][0]["tags"],
        serde_json::json!(["work"])
    );
}

#[test]
fn overflowing_or_fractional_counts_are_protocol_errors() {
    for bad in ["9223372036854775808", "1.5", "-1"] {
        let body = page_json(&[link_json("aaaa1111", None, bad)], None);
        let server = MockServer::start(move |_| Reply::json(200, &body));
        let harness = profile_for(&server);
        let run = harness.run_json("work", &["list"]);
        assert_eq!(run.status, 11, "visit_count {bad} must not succeed");
        assert!(run.stdout.is_empty());
    }
}

#[test]
fn malformed_success_json_is_protocol_error_with_empty_stdout() {
    let server = MockServer::start(|_| Reply::json(200, "{not json"));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 11);
    assert!(run.stdout.is_empty());
}

fn synthetic_links(prefix: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| link_json(&format!("{prefix}{i:04}"), None, &i.to_string()))
        .collect()
}

#[test]
fn fetch_all_follows_cursors_in_server_order_until_explicit_null() {
    let server = MockServer::start(|request| {
        let cursor = request.query_values("cursor").into_iter().next();
        match cursor.as_deref() {
            None => Reply::json(
                200,
                &page_json(&synthetic_links("p1x", 200), Some("cursor-2")),
            ),
            Some("cursor-2") => Reply::json(
                200,
                &page_json(&synthetic_links("p2x", 200), Some("cursor-3")),
            ),
            Some("cursor-3") => Reply::json(200, &page_json(&synthetic_links("p3x", 1), None)),
            other => Reply::json(
                400,
                &error_json("invalid_request", &format!("bad {other:?}"), "r"),
            ),
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all", "--limit", "200"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let value = run.json_stdout();
    let items = value["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 401);
    assert_eq!(items[0]["short_code"], "p1x0000");
    assert_eq!(items[400]["short_code"], "p3x0000");
    assert_eq!(value["meta"]["pages_fetched"], 3);
    assert_eq!(value["meta"]["complete"], true);
    assert_eq!(value["meta"]["stop_reason"], "end");
    assert_eq!(value["data"]["next_cursor"], serde_json::Value::Null);
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn default_listing_is_one_page_and_advertises_the_continuation() {
    let server = MockServer::start(|_| {
        Reply::json(200, &page_json(&synthetic_links("q", 2), Some("next-1")))
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--limit", "2"]);
    assert_eq!(run.status, 0);
    let value = run.json_stdout();
    assert_eq!(value["meta"]["complete"], false);
    assert_eq!(value["meta"]["truncated"], false);
    assert_eq!(value["meta"]["stop_reason"], "page");
    assert_eq!(value["data"]["next_cursor"], "next-1");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn an_empty_page_with_a_continuation_is_not_completion() {
    let server = MockServer::start(|request| {
        match request.query_values("cursor").first().map(String::as_str) {
            None => Reply::json(200, &page_json(&[], Some("go-on"))),
            Some(_) => Reply::json(200, &page_json(&synthetic_links("e", 1), None)),
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all"]);
    assert_eq!(run.status, 0);
    assert_eq!(
        run.json_stdout()["data"]["items"]
            .as_array()
            .expect("items")
            .len(),
        1
    );
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn later_page_failure_yields_a_valid_partial_result_with_exit_12() {
    let server = MockServer::start(|request| {
        match request.query_values("cursor").first().map(String::as_str) {
            None => Reply::json(200, &page_json(&synthetic_links("ok", 2), Some("page-two"))),
            Some(_) => Reply::json(500, &error_json("internal_error", "boom", "req-later")),
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all", "--limit", "2"]);
    run.assert_no_secret();
    assert_eq!(run.status, 12, "stderr: {}", run.stderr);
    let value = run.json_stdout();
    assert_eq!(value["data"]["items"].as_array().expect("items").len(), 2);
    assert_eq!(value["data"]["next_cursor"], "page-two");
    assert_eq!(value["meta"]["complete"], false);
    assert_eq!(value["meta"]["truncated"], true);
    assert_eq!(value["meta"]["stop_reason"], "error");
    assert_eq!(value["meta"]["pages_fetched"], 1);
    let error = &run.json_stderr_lines()[0];
    assert_eq!(error["partial"], true);
    assert_eq!(error["exit_code"], 12);
    assert_eq!(error["http_status"], 500);
    assert_eq!(error["error"]["kind"], "partial");
    assert_eq!(error["error"]["code"], "internal_error");
}

#[test]
fn first_page_failure_leaves_stdout_empty() {
    let server =
        MockServer::start(|_| Reply::json(503, &error_json("service_unavailable", "busy", "r")));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all"]);
    assert_eq!(run.status, 10);
    assert!(run.stdout.is_empty());
}

#[test]
fn a_repeated_continuation_stops_instead_of_looping() {
    let server =
        MockServer::start(|_| Reply::json(200, &page_json(&synthetic_links("r", 1), Some("same"))));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all"]);
    assert_eq!(run.status, 12);
    assert!(
        server.requests().len() <= 2,
        "loop must be stopped, saw {}",
        server.requests().len()
    );
}

#[test]
fn max_pages_bounds_fetch_all_with_honest_partial_output() {
    let server = MockServer::start(|request| {
        let index: usize = request
            .query_values("cursor")
            .first()
            .and_then(|c| c.strip_prefix('c'))
            .and_then(|c| c.parse().ok())
            .map_or(0, |c: usize| c + 1);
        Reply::json(
            200,
            &page_json(
                &synthetic_links(&format!("m{index}"), 1),
                Some(&format!("c{index}")),
            ),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run_json(
        "work",
        &["list", "--all", "--max-pages", "3", "--limit", "1"],
    );
    assert_eq!(run.status, 12);
    let value = run.json_stdout();
    assert_eq!(
        value["meta"]["stop_reason"], "page_limit",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(value["meta"]["pages_fetched"], 3);
    assert_eq!(value["data"]["items"].as_array().expect("items").len(), 3);
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn a_87387_byte_cursor_round_trips_through_file_request_and_saved_output() {
    let big = "k".repeat(87_387);
    let big_for_server = big.clone();
    let server = MockServer::start(move |request| {
        let incoming = request.query_values("cursor");
        if incoming.first().map(String::as_str) == Some(big_for_server.as_str()) {
            Reply::json(200, &page_json(&synthetic_links("end", 1), None))
        } else if incoming.is_empty() {
            Reply::json(
                200,
                &page_json(&synthetic_links("start", 1), Some(&big_for_server)),
            )
        } else {
            Reply::json(400, &error_json("invalid_request", "bad cursor", "r"))
        }
    });
    let harness = profile_for(&server);
    let first = harness.run(&[
        "--profile",
        "work",
        "--output",
        "json",
        "list",
        "--save-cursor",
        &harness.path("next.json").to_string_lossy(),
    ]);
    assert_eq!(first.status, 0, "{}", first.stderr);
    let saved = std::fs::read_to_string(harness.path("next.json")).expect("saved");
    let token: String = serde_json::from_str(saved.trim()).expect("json string");
    assert_eq!(token.len(), 87_387, "saved continuation must be exact");
    assert_eq!(token, big);
    let second = harness.run(&[
        "--profile",
        "work",
        "--output",
        "json",
        "list",
        "--cursor-file",
        &harness.path("next.json").to_string_lossy(),
    ]);
    assert_eq!(second.status, 0, "{}", second.stderr);
    assert_eq!(
        second.json_stdout()["data"]["items"][0]["short_code"],
        "end0000"
    );
    let sent = server.requests();
    assert!(
        sent[1].target.len() > 87_000,
        "request URL carried the full cursor"
    );
}

#[test]
fn a_saved_continuation_is_private_and_null_when_the_list_is_complete() {
    let server =
        MockServer::start(|_| Reply::json(200, &page_json(&synthetic_links("z", 1), None)));
    let harness = profile_for(&server);
    let save = harness.path("cursor.json");
    let run = harness.run(&[
        "--profile",
        "work",
        "list",
        "--save-cursor",
        &save.to_string_lossy(),
    ]);
    assert_eq!(run.status, 0);
    assert_eq!(
        std::fs::read_to_string(&save).expect("saved").trim(),
        "null"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&save).expect("meta").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn cursor_file_null_fails_before_any_request() {
    let server = MockServer::start(|_| Reply::json(200, &page_json(&[], None)));
    let harness = profile_for(&server);
    let file = harness.write_private("cursor.json", "null");
    let run = harness.run(&[
        "--profile",
        "work",
        "list",
        "--cursor-file",
        &file.to_string_lossy(),
    ]);
    assert_eq!(run.status, 2);
    assert_eq!(server.requests().len(), 0);
}

#[test]
fn profile_check_validates_with_the_account_read_only() {
    let server = MockServer::start(|_| Reply::json(200, &account_json("ferl_cli_admin", "admin")));
    let harness = profile_for(&server);
    let run = harness.run(&["--output", "json", "profile", "check", "work"]);
    run.assert_no_secret();
    assert_eq!(run.status, 0, "{}", run.stderr);
    let requests = server.requests();
    assert_eq!(
        requests.len(),
        1,
        "profile add without --validate is local; check is one read"
    );
    assert_eq!(requests[0].path(), "/api/v1/me");
    assert_eq!(
        run.json_stdout()["data"]["account"]["username"],
        "ferl_cli_admin"
    );
}

#[test]
fn explicit_timeouts_bound_the_operation_and_report_transport_failure() {
    let server = MockServer::start(|_| {
        Reply::Delay(
            std::time::Duration::from_secs(5),
            Box::new(Reply::json(200, &page_json(&[], None))),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run(&[
        "--profile",
        "work",
        "--timeout",
        "0.5",
        "--read-retries",
        "0",
        "list",
    ]);
    assert_eq!(run.status, 9, "stderr: {}", run.stderr);
    assert!(
        run.elapsed.as_secs_f64() < 3.0,
        "deadline must bound the operation: {:?}",
        run.elapsed
    );
}

#[test]
fn mutation_timeout_after_dispatch_is_uncertain_and_single_attempt() {
    let server = MockServer::start(|_| {
        Reply::Delay(
            std::time::Duration::from_secs(5),
            Box::new(created_reply(CODE)),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run(&[
        "--profile",
        "work",
        "--timeout",
        "0.5",
        "create",
        "https://example.com/t",
        "--no-copy",
    ]);
    assert_eq!(run.status, 13, "stderr: {}", run.stderr);
    assert_eq!(server.count("POST"), 1);
}

#[test]
fn a_dropped_read_connection_is_retried_once_and_then_succeeds() {
    let server = MockServer::start(|request| {
        if request.query_values("cursor").is_empty()
            && request.method == "GET"
            && server_first_drop()
        {
            Reply::Drop
        } else {
            Reply::json(200, &page_json(&synthetic_links("retry", 1), None))
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        server.requests().len(),
        2,
        "one dropped attempt and one successful retry"
    );
}

fn server_first_drop() -> bool {
    static DROPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    !DROPPED.swap(true, std::sync::atomic::Ordering::SeqCst)
}

fn run_with_profile_env(
    harness: &Harness,
    env_profile: Option<&str>,
    args: &[&str],
) -> common::Run {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ferl"));
    command
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", harness.dir.path().join("home"));
    if let Some(name) = env_profile {
        command.env("FERL_PROFILE", name);
    }
    command
        .arg("--config")
        .arg(&harness.config)
        .arg("--non-interactive")
        .args(args);
    command.stdin(std::process::Stdio::null());
    let output = command.output().expect("run");
    common::Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: std::time::Duration::from_secs(0),
    }
}

#[test]
fn explicit_flag_beats_environment_which_beats_the_saved_default() {
    let first =
        MockServer::start(|_| Reply::json(200, &page_json(&synthetic_links("aaa", 1), None)));
    let second =
        MockServer::start(|_| Reply::json(200, &page_json(&synthetic_links("bbb", 1), None)));
    let harness = Harness::new();
    assert_eq!(
        harness
            .add_profile("alpha", &first.url(), "ferl_ak_v1_ALPHA_KEY_0000000000")
            .status,
        0
    );
    let added = harness.run_with(
        &[
            "profile",
            "add",
            "beta",
            "--server",
            &second.url(),
            "--allow-http-loopback",
            "--key-stdin",
        ],
        Some(b"ferl_ak_v1_BETA_KEY_00000000000000\n"),
    );
    assert_eq!(added.status, 0, "{}", added.stderr);

    let by_default = run_with_profile_env(&harness, None, &["--output", "json", "list"]);
    assert_eq!(by_default.status, 0, "{}", by_default.stderr);
    assert_eq!(
        first.requests().len(),
        1,
        "the first profile is the default"
    );
    assert_eq!(second.requests().len(), 0);
    assert_eq!(
        first.requests()[0].header("Authorization"),
        Some("Bearer ferl_ak_v1_ALPHA_KEY_0000000000")
    );

    let by_env = run_with_profile_env(&harness, Some("beta"), &["--output", "json", "list"]);
    assert_eq!(by_env.status, 0, "{}", by_env.stderr);
    assert_eq!(
        second.requests().len(),
        1,
        "FERL_PROFILE overrides the default"
    );
    assert_eq!(
        second.requests()[0].header("Authorization"),
        Some("Bearer ferl_ak_v1_BETA_KEY_00000000000000")
    );

    let by_flag = run_with_profile_env(
        &harness,
        Some("beta"),
        &["--profile", "alpha", "--output", "json", "list"],
    );
    assert_eq!(by_flag.status, 0, "{}", by_flag.stderr);
    assert_eq!(
        first.requests().len(),
        2,
        "--profile overrides FERL_PROFILE"
    );
    assert_eq!(
        second.requests().len(),
        1,
        "no request reached the other server"
    );
}

#[test]
fn a_closed_output_pipe_after_a_committed_create_is_exit_14_with_committed_outcome() {
    let server = MockServer::start(|_| {
        Reply::json(
            201,
            &format!(r#"{{"link":{}}}"#, link_json("pipe0001", None, "0")),
        )
    });
    let harness = profile_for(&server);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ferl"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", harness.dir.path().join("home"))
        .arg("--config")
        .arg(&harness.config)
        .args([
            "--profile",
            "work",
            "create",
            "https://example.com/pipe",
            "--no-copy",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(14),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server.count("POST"),
        1,
        "an output failure never causes another creation"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Could not write the command output"));
}

#[test]
fn dropped_delete_acknowledgement_is_uncertain_and_not_replayed() {
    let server = MockServer::start(|_| Reply::Drop);
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["delete", CODE, "--yes"]);
    run.assert_no_secret();
    assert_eq!(run.status, 13, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty());
    assert_eq!(
        server.count("DELETE"),
        1,
        "one DELETE despite the lost acknowledgement"
    );
    assert_eq!(run.json_stderr_lines()[0]["mutation_outcome"], "unknown");
}

#[test]
fn json_creation_without_copy_flag_reports_a_skipped_clipboard_and_never_copies() {
    let server = MockServer::start(|_| created_reply(CODE));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["create", "https://example.com/json"]);
    run.assert_no_secret();
    assert_eq!(run.status, 0, "{}", run.stderr);
    let value = run.json_stdout();
    assert_eq!(value["command"], "create");
    assert_eq!(value["meta"]["clipboard"], "skipped");
    assert!(
        run.stderr.is_empty(),
        "no warning for a deliberately skipped copy: {}",
        run.stderr
    );
}

#[test]
fn list_rejections_map_to_documented_exit_codes_with_one_attempt_and_no_stdout() {
    let cases = [
        (400, "invalid_request", 2),
        (403, "forbidden", 5),
        (405, "method_not_allowed", 11),
        (413, "payload_too_large", 2),
        (415, "unsupported_media_type", 11),
    ];
    for (status, code, exit) in cases {
        let server = MockServer::start(move |_| {
            Reply::json(status, &error_json(code, "rejected", "req-reject"))
        });
        let harness = profile_for(&server);
        let run = harness.run_json("work", &["list"]);
        run.assert_no_secret();
        assert_eq!(run.status, exit, "HTTP {status}: stderr {}", run.stderr);
        assert!(run.stdout.is_empty(), "HTTP {status}: no partial output");
        assert_eq!(
            server.requests().len(),
            1,
            "HTTP {status}: a read rejection is not retried"
        );
        assert!(
            run.stderr.contains(code),
            "HTTP {status}: safe server code is shown"
        );
    }
}

#[test]
fn a_changed_context_cursor_is_refused_without_restarting_page_one() {
    let server = MockServer::start(|request| {
        if request.query_values("cursor").is_empty() {
            let links = synthetic_links("ctx", 2);
            Reply::json(200, &page_json(&links, Some("ctx-two")))
        } else {
            Reply::json(
                400,
                &error_json("invalid_cursor", "cursor no longer matches", "req-ctx"),
            )
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all"]);
    run.assert_no_secret();
    assert_eq!(
        run.status, 12,
        "accepted pages make a partial result: {}",
        run.stderr
    );
    let value = run.json_stdout();
    assert_eq!(value["data"]["items"].as_array().expect("items").len(), 2);
    assert_eq!(value["meta"]["stop_reason"], "error");
    assert_eq!(value["meta"]["complete"], false);
    assert_eq!(
        value["data"]["next_cursor"], "ctx-two",
        "continuation is the refused request"
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "no silent restart from page one");
    assert_eq!(requests[1].query_values("cursor"), ["ctx-two"]);
}

#[test]
fn a_changed_context_cursor_on_the_first_page_is_a_usage_failure_with_empty_stdout() {
    let server = MockServer::start(|_| {
        Reply::json(
            400,
            &error_json("invalid_cursor", "cursor no longer matches", "req-ctx"),
        )
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--cursor", "stale-token"]);
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty());
    let requests = server.requests();
    assert_eq!(
        requests.len(),
        1,
        "the refused cursor is never replaced by page one"
    );
    assert_eq!(requests[0].query_values("cursor"), ["stale-token"]);
}

#[test]
fn created_short_url_is_returned_exactly_when_the_public_origin_differs() {
    let server = MockServer::start(|_| created_reply(CODE));
    let harness = Harness::new();
    let added = harness.run_with(
        &[
            "profile",
            "add",
            "work",
            "--server",
            &server.url(),
            "--allow-http-loopback",
            "--public-origin",
            "https://short.example.test",
            "--key-stdin",
            "--default",
        ],
        Some(format!("{KEY}\n").as_bytes()),
    );
    assert_eq!(added.status, 0, "stderr: {}", added.stderr);
    let run = harness.run(&[
        "--profile",
        "work",
        "https://example.com/public",
        "--no-copy",
    ]);
    run.assert_no_secret();
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, format!("https://s.example.com/{CODE}\n"));
    assert_eq!(server.count("POST"), 1);
    assert_eq!(server.requests()[0].path(), "/api/v1/links");
}
