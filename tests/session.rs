use std::{
    collections::{BTreeMap, BTreeSet},
    process::Stdio,
    time::Duration,
};

use cps_block_ua::{MANIFEST, Policy, build_plugin};
use gateway_plugin_sdk::{
    CallContext, Capability, ErrorCode, Frame, Handshake, Manifest, Message, PROTOCOL_VERSION,
    PluginFault, Stage,
    call::{
        middleware::{
            HANDLE_METHOD, MiddlewareBodyFraming, MiddlewareBodyHandle, MiddlewareHeader,
            MiddlewareMount, MiddlewareNextRequest, MiddlewareNextResponse, MiddlewareRequestBody,
            MiddlewareRequestHead, MiddlewareResponseBody, MiddlewareResponseHead,
            MiddlewareTransport, NEXT_METHOD,
        },
        registration::Registration,
    },
    client::{PluginSession, SessionConfig, SessionError, read_frame, write_frame},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    process::{Child, Command},
    task::JoinHandle,
};

const ALLOWED_UA: &[u8] = b"codex_cli_rs/0.114.0 (Linux; x86_64)";
const UNKNOWN_UA: &[u8] = b"third-party-client/private-user-marker";

struct Host {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    completion: Completion,
}

enum Completion {
    Session(JoinHandle<Result<(), SessionError>>),
    Process(Child),
}

impl Host {
    async fn start(mode: &str) -> Self {
        let policy = Policy::from_value(json!({"mode": mode})).expect("valid test policy");
        let plugin = build_plugin(policy).expect("valid plugin manifest and handlers");
        let (host_io, plugin_io) = tokio::io::duplex(128 * 1024);
        let (reader, writer) = tokio::io::split(plugin_io);
        let task = tokio::spawn(async move {
            let session = PluginSession::accept(
                reader,
                writer,
                SessionConfig {
                    maximum_stream_chunk_bytes: 64 * 1024,
                    maximum_calls: 4,
                    maximum_callbacks: 4,
                    maximum_buffered_stream_chunks: 16,
                    handshake_timeout: Duration::from_secs(2),
                    maximum_call_timeout: Duration::from_secs(2),
                },
            )
            .await?;
            session.run(plugin).await
        });
        let (reader, writer) = tokio::io::split(host_io);
        let mut host = Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            completion: Completion::Session(task),
        };
        host.handshake(json!({"mode": mode})).await;
        host
    }

    async fn start_process(configuration: Value) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cps-block-ua"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("plugin binary starts");
        let mut host = Self {
            reader: Box::new(child.stdout.take().expect("piped plugin stdout")),
            writer: Box::new(child.stdin.take().expect("piped plugin stdin")),
            completion: Completion::Process(child),
        };
        host.handshake(configuration).await;
        host
    }

    async fn handshake(&mut self, configuration: Value) {
        let manifest = Manifest::from_author_slice(MANIFEST).expect("valid author manifest");
        self.control(Message::Hello {
            handshake: Handshake {
                protocol_version: PROTOCOL_VERSION,
                artifact_sha256: "a".repeat(64),
                plugin_id: manifest.plugin_id().expect("valid plugin identity"),
                instance_id: "test-instance".into(),
                generation: 1,
                incarnation: "test-incarnation".into(),
                configuration,
                permissions: manifest.permissions.into_iter().collect(),
                contributes: manifest.contributes,
            },
        })
        .await;
        let frame = self.receive().await;
        assert!(matches!(
            frame.message,
            Message::Ready {
                protocol_version: PROTOCOL_VERSION,
                ref incarnation,
            } if incarnation == "test-incarnation"
        ));
        assert!(frame.payload.is_empty());
    }

    async fn send(&mut self, frame: Frame) {
        write_frame(&mut self.writer, &frame)
            .await
            .expect("host frame written");
    }

    async fn control(&mut self, message: Message) {
        self.send(Frame::control(message)).await;
    }

    async fn receive(&mut self) -> Frame {
        tokio::time::timeout(Duration::from_secs(2), read_frame(&mut self.reader))
            .await
            .expect("plugin response timed out")
            .expect("valid plugin response frame")
    }

    async fn register(&mut self) -> Registration {
        let mut context = context(1);
        context.stage = Stage::Registration;
        context.request_id = None;
        self.control(Message::Call {
            id: 1,
            method: "plugin.register".into(),
            context,
            params: json!({}),
        })
        .await;
        let reply = self.receive().await;
        assert!(reply.payload.is_empty());
        let Message::Result { id: 1, result } = reply.message else {
            panic!("registration must return a result without invoking middleware")
        };
        serde_json::from_value(result).expect("valid registration")
    }

    async fn request(
        &mut self,
        id: u64,
        transport: MiddlewareTransport,
        ua: &[u8],
        payload: Option<&[u8]>,
    ) {
        let request = MiddlewareRequestHead {
            request_id: format!("request-{id}"),
            mount: MiddlewareMount::Request,
            attempt_index: None,
            operation: "generate".into(),
            protocol: "openai".into(),
            endpoint: "/v1/responses".into(),
            transport,
            provider: None,
            model: Some("gpt-5".into()),
            account_id: None,
            headers: vec![MiddlewareHeader {
                name: "user-agent".into(),
                value: ua.to_vec(),
            }],
            body_visible: payload.is_some(),
        };
        self.send(Frame {
            message: Message::Call {
                id,
                method: HANDLE_METHOD.into(),
                context: context(id),
                params: serde_json::to_value(request).expect("request serializes"),
            },
            payload: payload.unwrap_or_default().to_vec(),
        })
        .await;
    }

    async fn expect_next(&mut self, parent: u64) -> u64 {
        let frame = self.receive().await;
        let Message::Callback {
            id,
            parent_id,
            method,
            params,
        } = frame.message
        else {
            panic!("allowed request must invoke next")
        };
        assert_eq!(parent_id, parent);
        assert_eq!(method, NEXT_METHOD);
        let next: MiddlewareNextRequest =
            serde_json::from_value(params).expect("valid next request");
        assert_eq!(next.body, MiddlewareRequestBody::Preserve);
        assert!(next.protocol.is_none());
        assert!(next.header_mutations.is_empty());
        // Preserve 让宿主保留原始字节，不将隐藏或可见正文重新编码。
        assert!(frame.payload.is_empty());
        id
    }

    async fn finish_passthrough(
        &mut self,
        call_id: u64,
        callback_id: u64,
        framing: MiddlewareBodyFraming,
    ) {
        self.control(Message::Result {
            id: callback_id,
            result: serde_json::to_value(MiddlewareNextResponse {
                response: "upstream-response".into(),
                protocol: "openai".into(),
                status: 200,
                headers: vec![MiddlewareHeader {
                    name: "x-upstream-marker".into(),
                    value: b"unchanged".to_vec(),
                }],
                body: Some(MiddlewareBodyHandle {
                    handle: "opaque-upstream-body".into(),
                    framing,
                }),
            })
            .expect("next response serializes"),
        })
        .await;
        self.control(Message::Credit {
            id: call_id,
            bytes: 1,
            frames: 1,
        })
        .await;
        let initial = self.receive().await;
        let Message::Result { id, result } = initial.message else {
            panic!("opaque response must return directly without reading its body")
        };
        assert_eq!(id, call_id);
        assert!(initial.payload.is_empty());
        let head: MiddlewareResponseHead =
            serde_json::from_value(result).expect("valid middleware response");
        assert_eq!(head.response.as_deref(), Some("upstream-response"));
        assert!(head.protocol.is_none());
        assert!(head.status.is_none());
        assert!(head.header_mutations.is_empty());
        assert!(matches!(
            head.body,
            MiddlewareResponseBody::PassThrough { body }
                if body.handle == "opaque-upstream-body" && body.framing == framing
        ));
        let terminal = self.receive().await;
        assert!(matches!(terminal.message, Message::End { id, error: None } if id == call_id));
        assert!(terminal.payload.is_empty());
    }

    async fn expect_log(&mut self, parent: u64, mode: &str) -> u64 {
        let frame = self.receive().await;
        let Message::Callback {
            id,
            parent_id,
            method,
            params,
        } = frame.message
        else {
            panic!("UA policy decision must use host.log")
        };
        assert_eq!(parent_id, parent);
        assert_eq!(method, "host.log");
        assert!(frame.payload.is_empty());
        assert_eq!(params["event"], "ua_policy_decision");
        let fields = params["fields"].as_object().expect("structured log fields");
        assert_eq!(fields.len(), 3);
        assert_eq!(fields["mode"], mode);
        assert_eq!(fields["reason"], "unmatched_ua");
        assert_eq!(
            fields["decision"],
            if mode == "enforce" {
                "reject"
            } else {
                "would_reject"
            }
        );
        let encoded = params.to_string();
        assert!(!encoded.contains("private-user-marker"));
        assert!(!encoded.contains("third-party-client"));
        id
    }

    async fn expect_error(&mut self, call_id: u64, code: ErrorCode) {
        let frame = self.receive().await;
        let Message::Error { id, error } = frame.message else {
            panic!("rejected request must terminate without calling next")
        };
        assert_eq!(id, call_id);
        assert_eq!(error.code, code);
        assert!(frame.payload.is_empty());
    }

    async fn shutdown(mut self) {
        self.control(Message::Shutdown).await;
        match self.completion {
            Completion::Session(task) => {
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .expect("session shutdown timed out")
                    .expect("session task panicked")
                    .expect("session shutdown failed");
            }
            Completion::Process(mut child) => {
                let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
                    .await
                    .expect("plugin process shutdown timed out")
                    .expect("plugin process wait succeeded");
                assert!(status.success());
            }
        }
    }
}

fn context(id: u64) -> CallContext {
    CallContext {
        call_id: id,
        instance_id: "test-instance".into(),
        generation: 1,
        incarnation: "test-incarnation".into(),
        stage: Stage::Request,
        timeout_ms: 1500,
        resource_scope_id: format!("scope-{id}"),
        request_id: Some(format!("request-{id}")),
        attempt_id: None,
        account_id: None,
        credential_revision: None,
    }
}

#[tokio::test]
async fn registration_advertises_request_middleware_without_business_callbacks() {
    let mut host = Host::start("observe").await;
    let registration = host.register().await;
    let middleware = registration
        .contributes
        .get(&Capability::Middleware)
        .expect("middleware capability registered");
    assert_eq!(middleware.version, 1);
    assert_eq!(middleware.stages, vec![Stage::Request]);
    host.shutdown().await;
}

#[tokio::test]
async fn allowed_http_sse_and_websocket_preserve_request_bytes_and_opaque_response() {
    let transports = [
        (
            MiddlewareTransport::HttpJson,
            MiddlewareBodyFraming::JsonDocument,
        ),
        (
            MiddlewareTransport::HttpSse,
            MiddlewareBodyFraming::SseEvent,
        ),
        (
            MiddlewareTransport::WebSocket,
            MiddlewareBodyFraming::JsonDocument,
        ),
    ];
    let mut host = Host::start("enforce").await;
    host.register().await;
    let original: &[u8] = b"{ \"input\": [\"private-payload\"], \"number\": 1.00 }\n";
    let mut id = 3;
    for (transport, framing) in transports {
        for payload in [None, Some(original)] {
            host.request(id, transport, ALLOWED_UA, payload).await;
            let next_id = host.expect_next(id).await;
            host.finish_passthrough(id, next_id, framing).await;
            id += 2;
        }
    }
    host.shutdown().await;
}

#[tokio::test]
async fn enforcement_rejects_without_next_even_if_logging_fails() {
    let mut host = Host::start("enforce").await;
    host.register().await;
    for (id, logging_fails) in [(3, false), (5, true)] {
        host.request(id, MiddlewareTransport::WebSocket, UNKNOWN_UA, None)
            .await;
        let log_id = host.expect_log(id, "enforce").await;
        if logging_fails {
            host.control(Message::Error {
                id: log_id,
                error: PluginFault::new(ErrorCode::PermissionDenied, "logging unavailable"),
            })
            .await;
        } else {
            host.control(Message::Result {
                id: log_id,
                result: json!({"recorded": true}),
            })
            .await;
        }
        host.expect_error(id, ErrorCode::Rejected).await;
    }
    // 拒绝只结束当前调用，同一插件会话仍能服务后续合法请求。
    host.request(7, MiddlewareTransport::HttpJson, ALLOWED_UA, None)
        .await;
    let next_id = host.expect_next(7).await;
    host.finish_passthrough(7, next_id, MiddlewareBodyFraming::JsonDocument)
        .await;
    host.shutdown().await;
}

#[tokio::test]
async fn observe_logs_unknown_ua_then_preserves_the_original_request() {
    let mut host = Host::start("observe").await;
    host.register().await;
    host.request(3, MiddlewareTransport::HttpSse, UNKNOWN_UA, None)
        .await;
    let log_id = host.expect_log(3, "observe").await;
    host.control(Message::Result {
        id: log_id,
        result: json!({"recorded": false}),
    })
    .await;
    let next_id = host.expect_next(3).await;
    host.finish_passthrough(3, next_id, MiddlewareBodyFraming::SseEvent)
        .await;
    host.shutdown().await;
}

#[tokio::test]
async fn downstream_failure_is_preserved_instead_of_becoming_a_ua_denial() {
    let mut host = Host::start("enforce").await;
    host.register().await;
    host.request(3, MiddlewareTransport::HttpJson, ALLOWED_UA, None)
        .await;
    let next_id = host.expect_next(3).await;
    host.control(Message::Error {
        id: next_id,
        error: PluginFault::new(ErrorCode::Upstream, "upstream unavailable"),
    })
    .await;
    host.expect_error(3, ErrorCode::Upstream).await;
    host.shutdown().await;
}

#[tokio::test]
async fn cancellation_while_waiting_for_next_ends_the_request() {
    let mut host = Host::start("enforce").await;
    host.register().await;
    host.request(3, MiddlewareTransport::WebSocket, ALLOWED_UA, None)
        .await;
    let next_id = host.expect_next(3).await;
    host.control(Message::Cancel { id: 3 }).await;
    assert!(matches!(
        host.receive().await.message,
        Message::Cancelled { id: 3 }
    ));
    // 迟到的下游回调结果不能复活已取消请求，也不能破坏后续请求。
    host.control(Message::Result {
        id: next_id,
        result: json!({}),
    })
    .await;
    host.request(5, MiddlewareTransport::WebSocket, ALLOWED_UA, None)
        .await;
    let next_id = host.expect_next(5).await;
    host.finish_passthrough(5, next_id, MiddlewareBodyFraming::JsonDocument)
        .await;
    host.shutdown().await;
}

#[tokio::test]
async fn logging_timeout_preserves_enforce_and_observe_decisions() {
    for mode in ["enforce", "observe"] {
        let mut host = Host::start(mode).await;
        host.register().await;
        host.request(3, MiddlewareTransport::HttpJson, UNKNOWN_UA, None)
            .await;
        host.expect_log(3, mode).await;
        // 模拟宿主日志不响应；插件应在自己的短日志期限后继续原策略。
        if mode == "enforce" {
            host.expect_error(3, ErrorCode::Rejected).await;
        } else {
            let next_id = host.expect_next(3).await;
            host.finish_passthrough(3, next_id, MiddlewareBodyFraming::JsonDocument)
                .await;
        }
        host.shutdown().await;
    }
}

#[tokio::test]
async fn binary_stdio_reads_handshake_configuration_and_applies_custom_allowlist() {
    let mut host = Host::start_process(json!({
        "mode": "enforce",
        "allow_patterns": ["test-client/1\\.0"],
        "allow_models": false,
    }))
    .await;
    host.register().await;
    // 自定义规则替换默认规则，证明 main 使用了宿主握手配置。
    host.request(3, MiddlewareTransport::WebSocket, ALLOWED_UA, None)
        .await;
    let log_id = host.expect_log(3, "enforce").await;
    host.control(Message::Result {
        id: log_id,
        result: json!({"recorded": true}),
    })
    .await;
    host.expect_error(3, ErrorCode::Rejected).await;
    host.request(5, MiddlewareTransport::WebSocket, b"test-client/1.0", None)
        .await;
    let next_id = host.expect_next(5).await;
    host.finish_passthrough(5, next_id, MiddlewareBodyFraming::JsonDocument)
        .await;
    host.shutdown().await;
}

#[tokio::test]
async fn binary_concurrent_observe_timeouts_leave_capacity_for_every_next_callback() {
    let mut host = Host::start_process(json!({"mode": "observe"})).await;
    host.register().await;
    let call_ids: BTreeSet<_> = (0..32).map(|index| 3 + index * 2).collect();
    for &id in &call_ids {
        host.request(id, MiddlewareTransport::HttpSse, UNKNOWN_UA, None)
            .await;
    }

    let mut logs_seen = BTreeSet::new();
    let mut next_callbacks = BTreeMap::new();
    while next_callbacks.len() < call_ids.len() {
        let frame = host.receive().await;
        let Message::Callback {
            id,
            parent_id,
            method,
            params,
        } = frame.message
        else {
            panic!("log timeouts must not exhaust callback capacity or terminate requests")
        };
        assert!(call_ids.contains(&parent_id));
        assert!(frame.payload.is_empty());
        match method.as_str() {
            "host.log" => {
                assert!(logs_seen.insert(parent_id));
                assert_eq!(params["event"], "ua_policy_decision");
                // 故意挂起全部日志；SDK 在父调用结束前仍会保留这些回调。
            }
            NEXT_METHOD => {
                assert!(logs_seen.contains(&parent_id));
                let request: MiddlewareNextRequest =
                    serde_json::from_value(params).expect("valid next request");
                assert_eq!(request.body, MiddlewareRequestBody::Preserve);
                assert!(request.protocol.is_none());
                assert!(request.header_mutations.is_empty());
                assert!(next_callbacks.insert(parent_id, id).is_none());
            }
            other => panic!("unexpected callback: {other}"),
        }
    }
    assert_eq!(logs_seen, call_ids);
    // 在全部 next 都已发出之前不回收任何父调用，覆盖最大同时占用量。
    for (call_id, callback_id) in next_callbacks {
        host.finish_passthrough(call_id, callback_id, MiddlewareBodyFraming::SseEvent)
            .await;
    }
    host.shutdown().await;
}
