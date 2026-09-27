use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::ai::{AiCompletionRequest, AiMessage, AiProvider, AiService, AiToolDefinition};
use serde_json::{json, Value};

struct Request {
    line: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

fn server(
    responses: Vec<(u16, &'static str, String)>,
) -> (AiService, thread::JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        responses.into_iter().map(|(status, content_type, response)| {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client never made expected request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = BufReader::new(&socket);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut headers = BTreeMap::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() { break; }
                let (key, value) = header.split_once(':').unwrap();
                headers.insert(key.to_ascii_lowercase(), value.trim().to_owned());
            }
            let mut bytes = Vec::new();
            if let Some(length) = headers.get("content-length") {
                bytes.resize(length.parse().unwrap(), 0);
                reader.read_exact(&mut bytes).unwrap();
            } else if headers.get("transfer-encoding").is_some_and(|value| value == "chunked") {
                loop {
                    let mut length = String::new();
                    reader.read_line(&mut length).unwrap();
                    let length = usize::from_str_radix(length.trim(), 16).unwrap();
                    if length == 0 { break; }
                    let offset = bytes.len();
                    bytes.resize(offset + length, 0);
                    reader.read_exact(&mut bytes[offset..]).unwrap();
                    reader.read_exact(&mut [0; 2]).unwrap();
                }
            }
            drop(reader);
            write!(socket, "HTTP/1.1 {status} Response\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
            // 测试 UTF-8 和 SSE 数据帧被拆分在多次网络写入中的情况。
            for chunk in response.as_bytes().chunks(7) { socket.write_all(chunk).unwrap(); }
            Request {
                line,
                headers,
                body: if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() },
            }
        }).collect()
    });
    (
        AiService::new(AiProvider {
            base_url,
            api_key: "  local-test-key  ".into(),
            model: "mock-claude".into(),
            protocol: "anthropic-messages".into(),
            stream: true,
            ..Default::default()
        }),
        handle,
    )
}

fn json_reply(body: Value) -> (u16, &'static str, String) {
    (200, "application/json", body.to_string())
}

fn event(value: Value) -> String {
    format!(
        "event: {}\ndata: {}\n\n",
        value["type"].as_str().unwrap(),
        value
    )
}

fn assert_auth(request: &Request) {
    assert_eq!(request.headers["x-api-key"], "local-test-key");
    assert_eq!(request.headers["anthropic-version"], "2023-06-01");
    assert!(!request.headers.contains_key("authorization"));
}

#[test]
fn messages_transport_supports_a_complete_tool_roundtrip() {
    let (service, server) = server(vec![
        json_reply(json!({"type":"message","role":"assistant","content":[
            {"type":"text","text":"我来查询"},
            {"type":"tool_use","id":"toolu_1","name":"lookup","input":{"query":"笔记"}}
        ],"stop_reason":"tool_use","usage":{"input_tokens":15,"output_tokens":8}})),
        json_reply(
            json!({"type":"message","role":"assistant","content":[{"type":"text","text":"已找到"}],"stop_reason":"end_turn","usage":{"input_tokens":25,"output_tokens":4}}),
        ),
    ]);
    let mut request = AiCompletionRequest {
        messages: vec![
            AiMessage::new("system", "Be helpful"),
            AiMessage::new("user", "查找笔记"),
        ],
        tools: vec![AiToolDefinition::function(
            "lookup",
            "Find a note",
            json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        )],
        max_tokens: Some(0),
        temperature: Some(0.2),
        ..Default::default()
    };
    let first = service.complete_once(&request).unwrap();
    assert_eq!(first.finish_reason, "tool_calls");
    assert_eq!(
        first.tool_calls[0].function.arguments,
        r#"{"query":"笔记"}"#
    );
    request.messages.push(AiMessage {
        tool_calls: Some(first.tool_calls),
        ..AiMessage::new("assistant", &first.content)
    });
    request
        .messages
        .push(AiMessage::tool_result("toolu_1", "lookup", "a.md"));
    let final_response = service.complete_once(&request).unwrap();
    assert_eq!(final_response.content, "已找到");
    assert_eq!(final_response.total_tokens, 29);
    let captured = server.join().unwrap();
    for request in &captured {
        assert_auth(request);
        assert!(request.line.starts_with("POST /v1/messages "));
        assert_eq!(request.body["model"], "mock-claude");
        assert_eq!(request.body["max_tokens"], 4096);
        assert!(request.body.get("temperature").is_none());
        assert_eq!(request.body["system"], "Be helpful");
    }
    assert_eq!(
        captured[0].body["tools"][0]["input_schema"]["required"],
        json!(["query"])
    );
    assert_eq!(
        captured[1].body["messages"][1]["content"][1]["type"],
        "tool_use"
    );
    assert_eq!(
        captured[1].body["messages"][2]["content"][0]["tool_use_id"],
        "toolu_1"
    );
}

#[test]
fn streaming_transport_assembles_text_tools_and_cumulative_usage() {
    let stream = [
        json!({"type":"message_start","message":{"usage":{"input_tokens":20,"output_tokens":1}}}),
        json!({"type":"ping"}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"中文回复"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_stream","name":"lookup","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"中文\"}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(event).collect::<String>();
    let (service, server) = server(vec![(200, "text/event-stream", stream)]);
    let mut deltas = String::new();
    let response = service
        .stream_complete(
            &AiCompletionRequest {
                messages: vec![AiMessage::new("user", "hi")],
                ..Default::default()
            },
            |chunk| {
                deltas.push_str(chunk.delta.as_deref().unwrap_or_default());
            },
        )
        .unwrap();
    assert_eq!(deltas, "中文回复");
    assert_eq!(
        response.tool_calls[0].function.arguments,
        r#"{"query":"中文"}"#
    );
    assert_eq!(response.total_tokens, 29);
    assert_eq!(response.usage_source, "provider");
    assert_auth(&server.join().unwrap()[0]);
}

#[test]
fn stream_errors_and_truncated_tool_input_are_not_successful_completions() {
    let streams = [
        event(json!({"type":"error","error":{"type":"overloaded_error","message":"Try later"}})),
        event(json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}})),
        [
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_bad","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            json!({"type":"message_stop"}),
        ].into_iter().map(event).collect::<String>(),
    ];
    for stream in streams {
        let (service, server) = server(vec![(200, "text/event-stream", stream)]);
        assert!(service
            .stream_complete(&AiCompletionRequest::default(), |_| {})
            .is_err());
        server.join().unwrap();
    }
}

#[test]
fn model_listing_follows_cursor_pages_with_anthropic_auth() {
    let (service, server) = server(vec![
        json_reply(json!({"data":[{"id":"model-a"}],"has_more":true,"last_id":"model/a"})),
        json_reply(
            json!({"data":[{"id":"model-b"},{"id":"model-a"}],"has_more":false,"last_id":"model-b"}),
        ),
    ]);
    assert_eq!(service.list_models().unwrap(), vec!["model-a", "model-b"]);
    let captured = server.join().unwrap();
    assert!(captured[0].line.starts_with("GET /v1/models "));
    assert!(captured[1]
        .line
        .starts_with("GET /v1/models?after_id=model%2Fa "));
    captured.iter().for_each(assert_auth);
}

#[test]
fn http_errors_keep_status_and_provider_message() {
    let (service, server) = server(vec![(
        401,
        "application/json",
        json!({"type":"error","error":{"type":"authentication_error","message":"invalid api key"}})
            .to_string(),
    )]);
    let error = service
        .complete_once(&AiCompletionRequest::default())
        .unwrap_err();
    assert_eq!(error.status_code, Some(401));
    assert!(error.message.contains("invalid api key"));
    server.join().unwrap();
}

#[test]
fn cancelled_anthropic_requests_do_not_start_network_io() {
    let service = AiService::with_cancel(
        AiProvider {
            protocol: "anthropic-messages".into(),
            base_url: "http://127.0.0.1:9".into(),
            ..Default::default()
        },
        Arc::new(AtomicBool::new(true)),
    );
    assert_eq!(
        service
            .complete_once(&AiCompletionRequest::default())
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(service.list_models().unwrap_err().code, "CANCELLED");
}
