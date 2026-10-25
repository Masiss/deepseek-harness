//! Headless native-client journeys against strict loopback HTTP fixtures.
//! These check wire compatibility, not Host persistence or native UI parity.

use dsh_client_core::{
    commands::{CommandResult, CommandSubmitAttachment, CommandsClient},
    goals::{CreateGoalRequest, EditGoalRequest, GoalActivation, GoalPhase, GoalRef, GoalsClient},
    marketplace::{MarketplaceClient, MarketplaceInstallOptions},
    office_to_pdf::OfficeToPdfClient,
    plugins::PluginsClient,
    session::{
        SessionAddress, SessionAddressKind, SessionPageRequest, SessionTurnWindow, SessionViewer,
    },
    sessions::{CreateSessionRequest, ForkSessionRequest, RenameSessionRequest, SessionsClient},
    settings::{
        SettingsClient, SettingsMutateRequest, SettingsPathOp, SettingsReplaceRequest,
        SettingsUpdateRequest,
    },
    terminal::{
        TerminalClient, TerminalCloseRequest, TerminalCreateRequest, TerminalRenameRequest,
    },
    transport::HttpTransport,
    workspace_files::{
        WorkspaceByteReadOptions, WorkspaceFileRange, WorkspaceFileScope, WorkspaceFilesClient,
    },
    Error,
};
use httptest::{matchers::*, responders::*, Expectation, Server, ServerBuilder};
use serde_json::{json, Value};

struct HostFixture {
    server: Server,
    transport: HttpTransport,
    next_id: u64,
}

impl HostFixture {
    fn new() -> Self {
        let server = ServerBuilder::new()
            .bind_addr(([127, 0, 0, 1], 0).into())
            .run()
            .unwrap();
        server.expect(
            Expectation::matching(all_of![
                request::method_path("GET", "/connect"),
                request::query("token=fixture-launch&interactions=answer"),
            ])
            .respond_with(
                status_code(302)
                    .append_header("Location", "/api/remote.mux")
                    .append_header("Set-Cookie", "dsh-auth-fixture=operator; Path=/; HttpOnly"),
            ),
        );
        let (_, transport) =
            HttpTransport::authenticate(&format!("{}?token=fixture-launch", server.url_str("/")))
                .unwrap();
        Self {
            server,
            transport,
            next_id: 1,
        }
    }

    fn expect(&mut self, method: &str, args: Value, result: Value) {
        let id = format!("native-{}", self.next_id);
        self.next_id += 1;
        self.server.expect(
            Expectation::matching(all_of![
                request::method_path("POST", format!("/api/{method}")),
                request::headers(contains(("cookie", "dsh-auth-fixture=operator"))),
                request::body(json_decoded(eq(json!({
                    "type":"client-request", "rpcId":id, "method":method,
                    "payload":{"args":args}
                })))),
            ])
            .respond_with(json_encoded(json!({
                "type":"server-response", "rpcId":id, "result":result
            }))),
        );
    }

    fn ok(&mut self, method: &str, args: Value, value: Value) {
        self.expect(method, args, json!({"ok":true,"value":value}));
    }

    fn binary(&mut self, method: &str, args: Value, mut value: Value, data: &[u8]) {
        let id = format!("native-{}", self.next_id);
        self.next_id += 1;
        value["data"] = Value::Null;
        let metadata = json!({
            "type":"server-response", "rpcId":id, "result":{"ok":true,"value":value},
            "attachments":[{"codec":"bytes","part":"pdf","path":["data"]}]
        });
        let mut body = format!("--fixture\r\nContent-Disposition: form-data; name=\"metadata\"\r\n\r\n{metadata}\r\n--fixture\r\nContent-Disposition: form-data; name=\"pdf\"; filename=\"bytes\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n--fixture--\r\n");
        self.server.expect(
            Expectation::matching(all_of![
                request::method_path("POST", format!("/api/{method}")),
                request::headers(contains(("cookie", "dsh-auth-fixture=operator"))),
                request::body(json_decoded(eq(json!({
                    "type":"client-request","rpcId":id,"method":method,"payload":{"args":args}
                })))),
            ])
            .respond_with(
                status_code(200)
                    .append_header("Content-Type", "multipart/form-data; boundary=fixture")
                    .body(body),
            ),
        );
    }
}

fn goal_view(revision: u64, phase: &str, activation: &str) -> Value {
    json!({
        "id":"goal-native", "revision":revision, "objective":"Finish native goal RPCs",
        "phase":phase, "maxGoalRounds":256, "roundsStarted":2,
        "createdAt":1792710000000_u64, "updatedAt":1792710000000_u64 + revision,
        "activation":activation
    })
}

#[test]
fn slash_commands_list_and_execute_with_named_agent_and_attachment_arguments() {
    let mut host = HostFixture::new();
    host.ok(
        "commands/list",
        json!({"agentId":"a1"}),
        json!([{
            "definitionId":"@deepseek-ai/dsh-plan-mode",
            "name":"plan",
            "description":"Enter or leave plan mode",
            "input":{"hint":"[off|message]","attachments":true}
        }]),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/plan","submittedAttachments":[]}),
        json!({"commandId":"cmd-1","result":{"kind":"success","text":"Plan mode on."}}),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/plan off","submittedAttachments":[{"type":"file","receiptId":"receipt-1"}]}),
        json!({"commandId":"cmd-2","result":{"kind":"error","text":"Attachments cannot accompany /plan off."}}),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/unknown","submittedAttachments":[]}),
        Value::Null,
    );

    let client = CommandsClient::new(&host.transport);
    assert_eq!(client.list("a1").unwrap()[0].name, "plan");
    assert!(matches!(
        client
            .execute("a1", "/plan", vec![])
            .unwrap()
            .unwrap()
            .result,
        CommandResult::Success { .. }
    ));
    assert_eq!(
        client
            .execute(
                "a1",
                "/plan off",
                vec![CommandSubmitAttachment::File {
                    receipt_id: "receipt-1".into(),
                }],
            )
            .unwrap()
            .unwrap()
            .result,
        CommandResult::Error {
            text: "Attachments cannot accompany /plan off.".into()
        }
    );
    assert_eq!(client.execute("a1", "/unknown", vec![]).unwrap(), None);
}

#[test]
fn human_commands_list_and_execute_with_session_agent_identity() {
    let mut host = HostFixture::new();
    host.ok(
        "commands/list",
        json!({"agentId":"a1"}),
        json!([{
            "definitionId":"@deepseek-ai/dsh-plan-mode",
            "name":"plan",
            "description":"Enter or leave plan mode",
            "input":{"hint":"[off|message]","attachments":true}
        }]),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/plan","submittedAttachments":[]}),
        json!({"commandId":"cmd-1","result":{"kind":"success","text":"Plan mode on."}}),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/plan off","submittedAttachments":[]}),
        json!({"commandId":"cmd-2","result":{"kind":"success","text":"Plan mode off."}}),
    );
    host.ok(
        "commands/execute",
        json!({"agentId":"a1","line":"/unknown","submittedAttachments":[]}),
        Value::Null,
    );

    let commands = CommandsClient::new(&host.transport);
    assert_eq!(commands.list("a1").unwrap()[0].name, "plan");
    assert!(matches!(
        commands
            .execute("a1", "/plan", vec![])
            .unwrap()
            .unwrap()
            .result,
        CommandResult::Success { .. }
    ));
    assert_eq!(
        commands
            .execute("a1", "/plan off", vec![])
            .unwrap()
            .unwrap()
            .result,
        CommandResult::Success {
            text: Some("Plan mode off.".into()),
            source_event_seq: None,
        }
    );
    assert_eq!(commands.execute("a1", "/unknown", vec![]).unwrap(), None);
}

#[test]
fn goals_lifecycle_uses_agent_identity_exact_revisions_and_create_acknowledgement() {
    let mut host = HostFixture::new();
    host.expect("goals/get", json!({"agentId":"a1"}), json!({"ok":true}));
    host.ok(
        "goals/create",
        json!({"agentId":"a1","request":{"objective":"Finish native goal RPCs"}}),
        json!({"ref":{"id":"goal-native","revision":1}}),
    );
    host.ok(
        "goals/get",
        json!({"agentId":"a1"}),
        goal_view(1, "active", "armed"),
    );
    host.ok(
        "goals/edit",
        json!({"agentId":"a1","ref":{"id":"goal-native","revision":1},"request":{"maxGoalRounds":256}}),
        goal_view(2, "active", "armed"),
    );
    host.ok(
        "goals/pause",
        json!({"agentId":"a1","ref":{"id":"goal-native","revision":2}}),
        goal_view(3, "paused", "disarmed"),
    );
    host.ok(
        "goals/resume",
        json!({"agentId":"a1","ref":{"id":"goal-native","revision":3}}),
        goal_view(4, "active", "armed"),
    );
    host.ok(
        "goals/complete",
        json!({"agentId":"a1","ref":{"id":"goal-native","revision":4}}),
        goal_view(5, "complete", "disarmed"),
    );
    host.ok(
        "goals/clear",
        json!({"agentId":"a1","ref":{"id":"goal-native","revision":5}}),
        json!({"id":"goal-native","revision":6}),
    );
    host.ok("goals/get", json!({"agentId":"a1"}), Value::Null);

    let goals = GoalsClient::new(&host.transport);
    assert_eq!(goals.get("a1").unwrap(), None);
    let created = goals
        .create(
            "a1",
            CreateGoalRequest {
                objective: "Finish native goal RPCs".into(),
                max_goal_rounds: None,
            },
        )
        .unwrap();
    assert_eq!(
        created.r#ref,
        GoalRef {
            id: "goal-native".into(),
            revision: 1
        }
    );
    let current = goals.get("a1").unwrap().unwrap();
    assert_eq!(current.id, created.r#ref.id);
    assert_eq!(current.revision, created.r#ref.revision);
    assert_eq!(current.phase, GoalPhase::Active);
    assert_eq!(current.activation, GoalActivation::Armed);
    assert_eq!(current.rounds_started, 2);
    assert_eq!(current.created_at, 1792710000000);
    let edited = goals
        .edit(
            "a1",
            &created.r#ref,
            EditGoalRequest {
                objective: None,
                max_goal_rounds: Some(256),
            },
        )
        .unwrap();
    assert_eq!(edited.revision, 2);
    let paused = goals
        .pause(
            "a1",
            &GoalRef {
                id: edited.id,
                revision: edited.revision,
            },
        )
        .unwrap();
    assert_eq!(paused.phase, GoalPhase::Paused);
    assert_eq!(paused.activation, GoalActivation::Disarmed);
    let resumed = goals
        .resume(
            "a1",
            &GoalRef {
                id: paused.id,
                revision: paused.revision,
            },
        )
        .unwrap();
    assert_eq!(resumed.phase, GoalPhase::Active);
    assert_eq!(resumed.activation, GoalActivation::Armed);
    let completed = goals
        .complete(
            "a1",
            &GoalRef {
                id: resumed.id,
                revision: resumed.revision,
            },
        )
        .unwrap();
    assert_eq!(completed.phase, GoalPhase::Complete);
    assert_eq!(completed.activation, GoalActivation::Disarmed);
    let tombstone = goals
        .clear(
            "a1",
            &GoalRef {
                id: completed.id,
                revision: completed.revision,
            },
        )
        .unwrap();
    assert_eq!(
        tombstone,
        GoalRef {
            id: "goal-native".into(),
            revision: 6
        }
    );
    assert_eq!(goals.get("a1").unwrap(), None);
}

#[test]
fn goals_blocked_view_and_stale_revision_error_preserve_host_state_without_retry() {
    let mut host = HostFixture::new();
    let mut blocked = goal_view(7, "blocked", "disarmed");
    blocked["blockedReason"] = json!({"code":"round-budget","message":"Increase the round cap"});
    host.ok("goals/get", json!({"agentId":"a2"}), blocked);
    host.expect(
        "goals/resume",
        json!({"agentId":"a2","ref":{"id":"goal-native","revision":6}}),
        json!({"ok":false,"error":{"code":"GOAL_STALE_REVISION","message":"goal revision changed","details":{"currentRevision":7}}}),
    );
    let goals = GoalsClient::new(&host.transport);
    let current = goals.get("a2").unwrap().unwrap();
    assert_eq!(current.phase, GoalPhase::Blocked);
    assert_eq!(current.activation, GoalActivation::Disarmed);
    let reason = current.blocked_reason.unwrap();
    assert_eq!(reason.code, "round-budget");
    assert_eq!(reason.message, "Increase the round cap");
    match goals.resume(
        "a2",
        &GoalRef {
            id: current.id,
            revision: 6,
        },
    ) {
        Err(Error::Remote {
            code,
            message,
            details,
        }) => {
            assert_eq!(code, "GOAL_STALE_REVISION");
            assert_eq!(message, "goal revision changed");
            assert_eq!(
                details,
                json!({"currentRevision":7}).as_object().unwrap().clone()
            );
        }
        result => panic!("expected stale goal revision failure, got {result:?}"),
    }
}

#[test]
fn goals_reject_malformed_success_payloads() {
    let mut host = HostFixture::new();
    host.ok(
        "goals/create",
        json!({"agentId":"a1","request":{"objective":"Finish native goal RPCs","maxGoalRounds":512}}),
        goal_view(1, "active", "armed"),
    );
    let mut malformed = goal_view(1, "active", "armed");
    malformed["activation"] = json!("pending");
    host.ok("goals/get", json!({"agentId":"a1"}), malformed);
    let goals = GoalsClient::new(&host.transport);
    assert!(matches!(
        goals.create(
            "a1",
            CreateGoalRequest {
                objective: "Finish native goal RPCs".into(),
                max_goal_rounds: Some(512),
            }
        ),
        Err(Error::InvalidEnvelope("JSON decoding"))
    ));
    assert!(matches!(
        goals.get("a1"),
        Err(Error::InvalidEnvelope("JSON decoding"))
    ));
}

#[test]
fn session_create_rename_list_page_search_and_fork_use_named_arguments() {
    let mut host = HostFixture::new();
    host.ok(
        "session/create",
        json!({"request":{"cwd":"/workspace","sessionId":"s1"}}),
        json!({"sessionId":"s1"}),
    );
    host.ok(
        "session/rename",
        json!({"request":{"sessionId":"s1","title":"Native probe"}}),
        json!({"title":"Native probe","seq":2}),
    );
    host.ok(
        "session/list",
        json!({"_request":{"cursor":"cursor-1"}}),
        json!({"items":[{
            "sessionId":"s1","agentAvailable":true,"updatedAt":1,"running":false,"blank":false,
            "projections":{"title":"Native probe"}
        }]}),
    );
    host.ok("session/page", json!({"request":{"address":{"kind":"session","sessionId":"s1"},"throughSeq":2}}), json!({
        "records":[{"type":"event","event":{"type":"session/title","seq":2,"time":1,"data":{"title":"Native probe"}}}],"hasMore":true
    }));
    host.ok("session/page", json!({"request":{"address":{"kind":"session","sessionId":"s1"},"throughSeq":2,"beforeSeq":1,"maxMessages":20,"turnWindow":{"minMessages":10,"minTurns":2}}}), json!({
        "records":[{"type":"event","event":{"type":"user/message","seq":1,"time":1,"data":{"message":{"content":"older"}}}}],"hasMore":false
    }));
    host.ok(
        "session/search",
        json!({"request":{"query":"Native"}}),
        json!({"items":[{"sessionId":"s1","snippet":"Native probe"}],"hasMore":false}),
    );
    host.ok(
        "session/fork",
        json!({"request":{"sessionId":"s1","atSeq":2,"allowMigration":false}}),
        json!({"sessionId":"s2"}),
    );
    let sessions = SessionsClient::new(&host.transport);
    let session = sessions
        .create(CreateSessionRequest {
            workspace_id: None,
            cwd: Some("/workspace".into()),
            session_id: Some("s1".into()),
            agent_preset: None,
        })
        .unwrap();
    assert_eq!(session.session_id, "s1");
    assert_eq!(
        sessions
            .rename(RenameSessionRequest {
                session_id: session.session_id.clone(),
                title: "Native probe".into()
            })
            .unwrap()
            .seq,
        2
    );
    let viewer = SessionViewer::new(&host.transport);
    assert_eq!(
        viewer.list(Some("cursor-1")).unwrap()[0]
            .projections
            .as_ref()
            .unwrap()["title"],
        "Native probe"
    );
    let page = viewer.page(&session.session_id, 2).unwrap();
    assert!(page.has_more);
    assert_eq!(page.records[0].event.seq, 2);
    let older = viewer
        .page_request(SessionPageRequest {
            address: SessionAddress {
                kind: SessionAddressKind::Session,
                session_id: session.session_id.clone(),
            },
            through_seq: 2,
            before_seq: Some(1),
            max_messages: Some(20),
            turn_window: Some(SessionTurnWindow {
                min_messages: 10,
                min_turns: 2,
            }),
        })
        .unwrap();
    assert!(!older.has_more);
    assert_eq!(older.records[0].event.seq, 1);
    assert_eq!(sessions.search("Native").unwrap().items[0].session_id, "s1");
    assert_eq!(
        sessions
            .fork(ForkSessionRequest {
                session_id: session.session_id,
                at_seq: Some(2),
                allow_migration: Some(false)
            })
            .unwrap()
            .session_id,
        "s2"
    );
}

fn namespace(revision: u64, value: Value) -> Value {
    json!({"autoGenerate":true,"ns":"probe","schema":{},"value":value,"user":value,
        "applies":"immediate","secrets":[],"revision":revision})
}

#[test]
fn settings_writes_preserve_revisions_and_report_conflict_without_retry() {
    let mut host = HostFixture::new();
    host.ok("settings/describe", json!({}), json!({"writable":true,"hasDocument":false,"namespaces":[namespace(3, json!({"old":true}))]}));
    host.ok(
        "settings/update",
        json!({"ns":"probe","patch":{"enabled":true},"expectedRevision":3}),
        namespace(4, json!({"old":true,"enabled":true})),
    );
    host.ok("settings/mutate", json!({"ns":"probe","ops":[{"op":"set","path":["nested","name"],"value":"native"},{"op":"unset","path":["old"]}],"expectedRevision":4}), namespace(5, json!({"nested":{"name":"native"},"enabled":true})));
    host.expect("settings/replace", json!({"ns":"probe","section":{},"expectedRevision":4}), json!({"ok":false,"error":{"code":"settings/conflict","message":"revision changed","details":{"revision":5}}}));
    host.ok(
        "settings/replace",
        json!({"ns":"probe","section":{},"expectedRevision":5}),
        namespace(6, json!({})),
    );
    let settings = SettingsClient::new(&host.transport);
    let initial = settings.describe().unwrap();
    assert!(initial.writable);
    let updated = settings
        .update(SettingsUpdateRequest {
            ns: "probe".into(),
            patch: serde_json::from_value(json!({"enabled":true})).unwrap(),
            expected_revision: Some(initial.namespaces[0].revision),
        })
        .unwrap();
    let mutated = settings
        .mutate(SettingsMutateRequest {
            ns: "probe".into(),
            expected_revision: Some(updated.revision),
            ops: vec![
                SettingsPathOp::Set {
                    path: vec!["nested".into(), "name".into()],
                    value: json!("native"),
                },
                SettingsPathOp::Unset {
                    path: vec!["old".into()],
                },
            ],
        })
        .unwrap();
    assert_eq!(mutated.value["nested"]["name"], "native");
    assert!(
        matches!(settings.replace(SettingsReplaceRequest { ns:"probe".into(),section:Default::default(),expected_revision:Some(4) }), Err(Error::Remote { code, .. }) if code == "settings/conflict")
    );
    assert_eq!(
        settings
            .replace(SettingsReplaceRequest {
                ns: "probe".into(),
                section: Default::default(),
                expected_revision: Some(mutated.revision)
            })
            .unwrap()
            .revision,
        6
    );
}

#[test]
fn terminal_lifecycle_keeps_agent_and_session_arguments_distinct() {
    let mut host = HostFixture::new();
    let shell = json!({"path":"/bin/sh","name":"sh","args":[]});
    let terminal = json!({"id":"t1","title":"Terminal","shell":shell,"cwd":"/workspace","cols":80,"rows":24,"state":"running","exitCode":null});
    host.ok("terminal/environment", json!({"agentId":"s1"}), json!({"cwd":"/workspace","maxInputBytes":1024,"maxBufferedBytes":4096,"maxCols":200,"maxRows":100,"scrollback":1000}));
    host.ok("terminal/shells", json!({"agentId":"s1"}), json!([shell]));
    host.ok(
        "terminal/create",
        json!({"agentId":"s1","request":{"id":"t1","cols":80,"rows":24}}),
        terminal.clone(),
    );
    host.ok(
        "terminal/list",
        json!({"sessionId":"s1"}),
        json!([terminal]),
    );
    host.expect(
        "terminal/rename",
        json!({"agentId":"s1","id":"t1","title":"Native PTY"}),
        json!({"ok":true}),
    );
    host.expect(
        "terminal/close",
        json!({"agentId":"s1","id":"t1"}),
        json!({"ok":true}),
    );
    host.ok("terminal/list", json!({"sessionId":"s1"}), json!([]));
    let client = TerminalClient::new(&host.transport, "s1");
    assert_eq!(client.environment().unwrap().max_input_bytes, 1024);
    assert_eq!(client.shells().unwrap()[0].path, "/bin/sh");
    assert_eq!(
        client
            .create(TerminalCreateRequest {
                id: "t1".into(),
                shell_path: None,
                cols: 80,
                rows: 24
            })
            .unwrap()
            .id,
        "t1"
    );
    assert_eq!(client.list("s1").unwrap().len(), 1);
    client
        .rename(TerminalRenameRequest {
            id: "t1".into(),
            title: "Native PTY".into(),
        })
        .unwrap();
    client
        .close(TerminalCloseRequest { id: "t1".into() })
        .unwrap();
    assert!(client.list("s1").unwrap().is_empty());
}

#[test]
fn plugin_and_marketplace_operations_preserve_application_failures() {
    let mut host = HostFixture::new();
    host.ok("pluginManager/listPlugins", json!({}), json!([{"entryId":"probe","moduleName":"@fixture/probe","enabled":true,"patchId":"probe","fiberPhase":"active"}]));
    host.ok(
        "pluginManager/listBundles",
        json!({}),
        json!([{"name":"@fixture/probe","enabled":true,"installed":true}]),
    );
    host.ok("pluginManager/setPluginEnabled", json!({"id":"probe","enabled":false}), json!({"changed":true,"application":"applied","stage":"enable","target":"probe","enabled":false}));
    host.ok("pluginManager/setBundleEnabled", json!({"name":"@fixture/probe","enabled":false}), json!({"changed":true,"application":"restart-required","stage":"enable","target":"@fixture/probe"}));
    host.ok("pluginManager/removeBundle", json!({"name":"@fixture/probe"}), json!({"changed":false,"application":"failed","stage":"remove","target":"@fixture/probe","error":{"code":"read-only"}}));
    host.ok(
        "pluginMarket/listIndexes",
        json!({}),
        json!([{ "url":"https://market.invalid/index.json" }]),
    );
    host.ok("pluginMarket/search", json!({"url":"https://market.invalid/index.json","query":{"text":"probe","sort":{"kind":"relevance"},"offset":0,"limit":20}}), json!({"items":[],"total":0}));
    host.ok(
        "pluginManager/inspect",
        json!({"spec":"file:/fixture/probe.tgz"}),
        json!({"name":"@fixture/probe"}),
    );
    host.ok(
        "pluginManager/installBundle",
        json!({"spec":"file:/fixture/probe.tgz","options":{}}),
        json!({"changed":true,"application":"restart-required","stage":"enable","target":"@fixture/probe"}),
    );
    let plugins = PluginsClient::new(&host.transport);
    assert_eq!(
        plugins.list_plugins().unwrap()[0].patch_id.as_deref(),
        Some("probe")
    );
    assert!(plugins.list_bundles().unwrap()[0].installed);
    assert!(plugins.set_plugin_enabled("probe", false).unwrap().changed);
    assert_eq!(
        plugins
            .set_bundle_enabled("@fixture/probe", false)
            .unwrap()
            .application,
        "restart-required"
    );
    assert_eq!(
        plugins
            .remove_bundle("@fixture/probe")
            .unwrap()
            .error
            .unwrap()
            .code,
        "read-only"
    );
    let market = MarketplaceClient::new(&host.transport);
    assert_eq!(
        market.list_indexes().unwrap()[0]["url"],
        "https://market.invalid/index.json"
    );
    assert_eq!(
        market
            .search("https://market.invalid/index.json", "probe")
            .unwrap()["total"],
        0
    );
    let options = MarketplaceInstallOptions::default();
    assert_eq!(
        market.inspect("file:/fixture/probe.tgz", &options).unwrap()["name"],
        "@fixture/probe"
    );
    let install = market
        .install_bundle("file:/fixture/probe.tgz", &options)
        .unwrap();
    assert_eq!(install["application"], "restart-required");
}

#[test]
fn marketplace_install_options_reach_inspect_and_install() {
    let mut host = HostFixture::new();
    let options = MarketplaceInstallOptions {
        registry: Some("http://127.0.0.1:31337/".into()),
    };
    host.ok(
        "pluginManager/inspect",
        json!({"spec":"@fixture/probe@1.0.0","options":{"registry":"http://127.0.0.1:31337/"}}),
        json!({"status":"accepted","name":"@fixture/probe","registry":"http://127.0.0.1:31337/"}),
    );
    host.ok(
        "pluginManager/installBundle",
        json!({"spec":"@fixture/probe@1.0.0","options":{"registry":"http://127.0.0.1:31337/"}}),
        json!({"application":"applied"}),
    );

    let market = MarketplaceClient::new(&host.transport);
    assert_eq!(
        market.inspect("@fixture/probe@1.0.0", &options).unwrap()["status"],
        "accepted"
    );
    assert_eq!(
        market
            .install_bundle("@fixture/probe@1.0.0", &options)
            .unwrap()["application"],
        "applied"
    );
}

#[test]
fn document_and_office_previews_decode_multipart_bytes_and_unavailable_errors() {
    let mut host = HostFixture::new();
    host.ok("workspaceFiles/read", json!({"workspaceFileScopeId":"s1","path":"notes.txt","range":{"offset":0,"limit":2}}), json!({"absolutePath":"/workspace/notes.txt","version":"v1","bytes":6,"offset":0,"text":"hello\n","lines":1,"eof":true}));
    host.binary("workspaceFiles/readBytes", json!({"workspaceFileScopeId":"s1","path":"image.png","options":{}}), json!({"absolutePath":"/workspace/image.png","version":"v1","bytes":8,"offset":0,"eof":true}), b"\x89PNG\r\n\x1a\n");
    host.binary(
        "officeToPdf/render",
        json!({"workspaceFileScopeId":"s1","path":"report.docx","priority":"foreground"}),
        json!({"absolutePath":"/workspace/report.docx","version":"v1","offset":0,"bytes":12,"eof":true,"generation":1,"missingFonts":["Calibri"]}),
        b"%PDF-1.7\n\0\xff",
    );
    host.expect("officeToPdf/render", json!({"workspaceFileScopeId":"s1","path":"missing.docx","priority":"foreground"}), json!({"ok":false,"error":{"code":"document-render/failed","message":"Office component is not installed","details":{"reason":"unavailable"}}}));
    let scope = WorkspaceFileScope {
        session_id: "s1".into(),
    };
    let files = WorkspaceFilesClient::new(&host.transport);
    assert_eq!(
        files
            .read(
                scope.clone(),
                "notes.txt",
                WorkspaceFileRange {
                    offset: Some(0),
                    limit: Some(2)
                }
            )
            .unwrap()
            .text,
        "hello\n"
    );
    assert_eq!(
        files
            .read_bytes(
                scope.clone(),
                "image.png",
                WorkspaceByteReadOptions {
                    range: None,
                    base_file: None
                }
            )
            .unwrap()
            .data,
        b"\x89PNG\r\n\x1a\n"
    );
    let office = OfficeToPdfClient::new(&host.transport);
    let preview = office
        .render(scope.clone(), "report.docx", "foreground")
        .unwrap();
    assert_eq!(preview.pdf, b"%PDF-1.7\n\0\xff");
    assert_eq!(preview.missing_fonts, ["Calibri"]);
    assert!(
        matches!(office.render(scope,"missing.docx","foreground"), Err(Error::Remote { code, details, .. }) if code == "document-render/failed" && details["reason"] == "unavailable")
    );
}
