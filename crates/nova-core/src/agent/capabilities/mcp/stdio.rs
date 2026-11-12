use super::connection::{McpConnection, NovaMcpHandler, RmcpConnection};
use super::MCP_CONNECT_TIMEOUT;
use crate::core::Core;
use rmcp::service::ServiceExt;
use rmcp::transport::TokioChildProcess;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::timeout;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn parse_command_and_args(command: &str, args: &[String]) -> (String, Vec<String>) {
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
    (parsed_command, parsed_args)
}

fn spawn_child_transport(
    cmd_name: &str,
    cmd_args: &[String],
    env_map: &HashMap<String, String>,
) -> std::io::Result<TokioChildProcess> {
    let mut cmd = Command::new(cmd_name);
    cmd.args(cmd_args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    for (k, v) in env_map {
        cmd.env(k, v);
    }

    let (transport, _stderr) = TokioChildProcess::builder(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(transport)
}

pub(super) async fn connect_stdio(
    core: &Core,
    server_name: &str,
    command: &str,
    args: &[String],
    env_map: &HashMap<String, String>,
) -> Result<Arc<dyn McpConnection>, String> {
    let (parsed_command, parsed_args) = parse_command_and_args(command, args);

    let transport = match spawn_child_transport(&parsed_command, &parsed_args, env_map) {
        Ok(t) => t,
        Err(primary_err) => {
            #[cfg(windows)]
            {
                if primary_err.kind() == std::io::ErrorKind::NotFound {
                    let mut shell_args = vec!["/C".to_string(), parsed_command.clone()];
                    shell_args.extend(parsed_args.clone());
                    match spawn_child_transport("cmd", &shell_args, env_map) {
                        Ok(t) => t,
                        Err(shell_err) => {
                            return Err(format!(
                                "Failed to spawn MCP server: {} (and cmd fallback failed: {}). Hint: for Playwright MCP use command 'npx' with args '-y @playwright/mcp@latest'.",
                                primary_err, shell_err
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

    let handler = NovaMcpHandler::new(core.clone(), server_name);
    let running = timeout(MCP_CONNECT_TIMEOUT, handler.serve(transport))
        .await
        .map_err(|_| {
            "MCP server initialize timeout (30s). First-time npx install may be slow; please retry or pre-run `npx -y @playwright/mcp@latest --help` in terminal.".to_string()
        })?
        .map_err(|e| format!("MCP stdio initialize failed: {e}"))?;

    Ok(RmcpConnection::from_running_service(running))
}
