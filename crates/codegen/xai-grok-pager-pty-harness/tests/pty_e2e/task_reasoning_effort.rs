// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;
use xai_grok_test_support::{Conversation, MockToolCall, Tool};

const CHILD_PROMPT: &str = "REASONING_EFFORT_CHILD_MARKER investigate the failing check";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "PTY e2e; run the owning pty_e2e_* Cargo test with --ignored (see Cargo.toml)"]
async fn task_reasoning_effort_is_sent_for_child() {
    for (effort, expected) in [
        (Some("high"), "high"),
        (None, "medium"),
        (Some("none"), "none"),
        (Some("NONE"), "none"),
        (Some("invalid-effort"), "medium"),
    ] {
        assert_child_effort(effort, expected).await;
    }
}

async fn assert_child_effort(effort: Option<&str>, expected: &str) {
    let content = ContentController::start_with_models(vec![
        MockModel::new("test-model")
            .with_api_backend("responses")
            .with_supports_reasoning_effort(true)
            .with_reasoning_efforts(vec![
                json!({ "id": "none", "value": "none", "label": "None" }),
                json!({ "id": "low", "value": "low", "label": "Low" }),
                json!({ "id": "medium", "value": "medium", "label": "Medium" }),
                json!({ "id": "high", "value": "high", "label": "High" }),
            ]),
    ])
    .await
    .expect("start content");
    let mut task_args = json!({
        "description": "reasoning child",
        "prompt": CHILD_PROMPT,
        "subagent_type": "general-purpose",
        "background": false,
    });
    if let Some(effort) = effort {
        task_args["reasoning_effort"] = json!(effort);
    }
    content.server().set_conversations(vec![
        Conversation::nth(1)
            .calls([MockToolCall::new(Tool::Task, task_args)])
            .reply("PARENT_REASONING_DONE"),
        Conversation::nth(2).reply("CHILD_REASONING_DONE"),
    ]);

    let binary = pager_binary().expect("resolve pager binary");
    let mut harness = PtyHarness::spawn_with_content_env_ops_in_dir(
        &binary,
        DEFAULT_ROWS,
        DEFAULT_COLS,
        &content,
        &["--yolo", "--trust", "--reasoning-effort", "medium"],
        &[],
        Some(content.home()),
    )
    .expect("spawn pager");
    harness
        .wait_for_text(WELCOME_SCREEN_SENTINEL, WELCOME_TIMEOUT)
        .expect("welcome");
    harness
        .inject_keys(format!("{PROMPT}\r").as_bytes())
        .expect("submit parent prompt");
    harness
        .wait_for_text("PARENT_REASONING_DONE", Duration::from_secs(60))
        .expect("parent spawned child");

    let bodies: Vec<_> = content
        .requests()
        .into_iter()
        .filter(|entry| {
            entry
                .header("x-grok-turn-idx")
                .is_some_and(|turn| !turn.trim().is_empty())
        })
        .filter_map(|entry| entry.body)
        .collect();
    let request_for = |prompt: &str| {
        bodies.iter().find(|body| {
            body["messages"]
                .as_array()
                .or_else(|| body["input"].as_array())
                .into_iter()
                .flatten()
                .any(|message| {
                    message["role"] == "user" && message["content"].to_string().contains(prompt)
                })
        })
    };
    let parent_request = request_for(PROMPT).expect("parent foreground inference request");
    assert_eq!(
        parent_request
            .pointer("/reasoning/effort")
            .and_then(|value| value.as_str()),
        Some("medium"),
        "parent uses its configured effort"
    );
    let child_request = request_for(CHILD_PROMPT).expect("child foreground inference request");
    assert_eq!(
        child_request
            .pointer("/reasoning/effort")
            .and_then(|value| value.as_str()),
        Some(expected),
        "child applies task reasoning_effort {effort:?} with parent effort medium"
    );

    harness.quit().expect("clean quit");
}
