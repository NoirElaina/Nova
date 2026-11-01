use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use super::{McpResourceInfo, McpToolInfo, MCP_CONNECT_TIMEOUT};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) struct StdioMcpConnection {
    child: Child,
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
    next_id: u64,
}

impl StdioMcpConnection {
    async fn send_message(&mut self, value: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        self.writer.flush().await.map_err(|e| e.to_string())
    }

    async fn read_message(&mut self) -> Result<Value, String> {
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .read_line(&mut line)
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("MCP stdio stream closed".into());
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<Value>(line) {
                Ok(v) => return Ok(v),
                Err(_) => continue,
            }
        }
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.send_message(&req).await?;

        loop {
            let msg = self.read_message().await?;
            let msg_id = msg.get("id").and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
            });
            if msg_id != Some(id) {
                continue;
            }
            if let Some(err) = msg.get("error") {
                return Err(format!("MCP error: {}", err));
            }
            return Ok(msg.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send_message(&req).await
    }

    pub(super) async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, String> {
        let result = self.send_request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(|v| v.as_str())?.to_string();
                let description = t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let input_schema = t.get("inputSchema").cloned();
                Some(McpToolInfo {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    pub(super) async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        self.send_request(
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        )
        .await
    }

    pub(super) async fn list_resources(&mut self) -> Result<Vec<McpResourceInfo>, String> {
        let result = self.send_request("resources/list", json!({})).await?;
        let resources = result
            .get("resources")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(resources
            .into_iter()
            .filter_map(|r| {
                let uri = r.get("uri").and_then(|v| v.as_str())?.to_string();
                let name = r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&uri)
                    .to_string();
                let description = r
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let mime_type = r
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(McpResourceInfo {
                    uri,
                    name,
                    description,
                    mime_type,
                })
            })
            .collect())
    }

    pub(super) async fn read_resource(&mut self, uri: &str) -> Result<Value, String> {
        self.send_request("resources/read", json!({ "uri": uri }))
            .await
    }

    pub(super) async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
    }
}

pub(super) async fn connect_stdio(
    command: &str,
    args: &[String],
    env_map: &HashMap<String, String>,
) -> Result<StdioMcpConnection, String> {
    let mut parsed_command = command.trim().to_string();
    let mut parsed_args = args.to_vec();
    if parsed_args.is_empty() && parsed_command.contains(' ') {
        let mut parts = parsed_command
            .split_whitespace()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            parsed_command = parts.remove(0);
            parsed_args = parts;
        }
    }

    let spawn_once = |cmd_name: &str, cmd_args: &[String]| {
        let mut cmd = Command::new(cmd_name);
        cmd.args(cmd_args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        for (k, v) in env_map {
            cmd.env(k, v);
        }

        cmd.spawn()
    };

    let mut child = match spawn_once(&parsed_command, &parsed_args) {
        Ok(child) => child,
        Err(primary_err) => {
            #[cfg(windows)]
            {
                if primary_err.kind() == std::io::ErrorKind::NotFound {
                    let mut shell_args = vec!["/C".to_string(), parsed_command.clone()];
                    shell_args.extend(parsed_args.clone());
                    match spawn_once("cmd", &shell_args) {
                        Ok(child) => child,
                        Err(shell_err) => {
                            return Err(format!(
                                "Failed to spawn MCP server: {} (and cmd fallback failed: {}). Hint: for Playwright MCP use command 'npx' with args '-y @playwright/mcp@latest'.",
                                primary_err,
                                shell_err
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "Failed to spawn MCP server: {}. Hint: for Playwright MCP use command 'npx' with args '-y @playwright/mcp@latest'.",
                        primary_err
                    ));
                }
            }
            #[cfg(not(windows))]
            {
                return Err(format!(
                    "Failed to spawn MCP server: {}. Hint: for Playwright MCP use command 'npx' with args '-y @playwright/mcp@latest'.",
                    primary_err
                ));
            }
        }
    };

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Missing MCP stdin pipe".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Missing MCP stdout pipe".to_string())?;

    let mut conn = StdioMcpConnection {
        child,
        reader: BufReader::new(stdout),
        writer: stdin,
        next_id: 1,
    };

    let init_result = timeout(
        MCP_CONNECT_TIMEOUT,
        conn.send_request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "nova",
                    "version": "0.1.0"
                }
            }),
        ),
    )
    .await;

    match init_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            let _ = conn.shutdown().await;
            return Err("MCP server initialize timeout (30s). First-time npx install may be slow; please retry or pre-run `npx -y @playwright/mcp@latest --help` in terminal.".to_string());
        }
    }

    let _ = conn
        .send_notification("notifications/initialized", json!({}))
        .await;

    Ok(conn)
}
