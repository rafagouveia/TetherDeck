use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Parser;
use rmcp::{
    ErrorData as McpError, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ImageContent},
    schemars, tool, tool_router,
    transport::stdio,
};
use rust_ios_device_client::{ClientError, DeviceClient};
use serde::Deserialize;

#[derive(Parser)]
#[command(about = "Optional stdio MCP adapter for the local Rust iOS device daemon")]
struct Args {
    #[arg(
        long,
        env = "RUST_IOS_DEVICE_URL",
        default_value = "http://127.0.0.1:18087"
    )]
    url: String,
    #[arg(long, env = "RUST_IOS_DEVICE_TOKEN")]
    token: String,
    /// Expose service recovery to agents. The daemon must also enable it.
    #[arg(long)]
    enable_recovery_tools: bool,
}

#[derive(Clone)]
struct IosMcp {
    client: DeviceClient,
    enable_recovery_tools: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeviceArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
}

#[derive(Debug, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ElementProvider {
    #[default]
    Auto,
    Wda,
    IosAx,
}
impl ElementProvider {
    fn wire(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Wda => "wda",
            Self::IosAx => "ios_ax",
        }
    }
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UiArgs {
    device_id: String,
    /// Choose wda for XCTest hierarchy/actions, ios_ax for flat inspection (may scroll), auto for fallback.
    #[serde(default)]
    provider: ElementProvider,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UsbRecoveryArgs {
    device_id: String,
    /// Exact instance_id from diagnose.usb_recovery_targets; no wildcards.
    instance_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LaunchArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
    /// Installed app bundle identifier, such as com.example.app.
    bundle_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FindElementArgs {
    #[serde(default)]
    provider: ElementProvider,
    device_id: String,
    role: Option<String>,
    identifier: Option<String>,
    label: Option<String>,
    #[serde(default)]
    contains: bool,
    #[serde(default)]
    timeout_ms: u64,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ActivateElementArgs {
    device_id: String,
    snapshot_id: String,
    element_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TapArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
    /// Horizontal display coordinate in pixels.
    x: u16,
    /// Vertical display coordinate in pixels.
    y: u16,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PressArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
    /// Horizontal display coordinate in pixels.
    x: u16,
    /// Vertical display coordinate in pixels.
    y: u16,
    /// Press duration, from 1 to 60000 milliseconds.
    #[schemars(range(min = 1, max = 60000))]
    duration_ms: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DragArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
    start_x: u16,
    start_y: u16,
    end_x: u16,
    end_y: u16,
    /// Drag duration, from 1 to 60000 milliseconds.
    #[schemars(range(min = 1, max = 60000))]
    duration_ms: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TypeTextArgs {
    /// Device identifier returned by list_devices.
    device_id: String,
    /// ASCII text supported by a US keyboard layout, up to 4096 bytes.
    #[schemars(length(max = 4096))]
    text: String,
}

#[tool_router(server_handler)]
impl IosMcp {
    #[tool(
        description = "Read native iOS accessibility elements and current references, without screenshot/OCR. AX traversal may scroll the app. Completeness and unavailable attributes are explicit.",
        annotations(read_only_hint = true)
    )]
    async fn ui_snapshot(&self, Parameters(args): Parameters<UiArgs>) -> CallToolResult {
        self.json_tool(
            reqwest::Method::GET,
            &format!(
                "/v1/devices/{}/ui?provider={}",
                url_segment(&args.device_id),
                args.provider.wire()
            ),
            None,
        )
        .await
    }
    #[tool(
        description = "Find exactly one accessible element by identifier and/or label. Exact match by default; ambiguity fails. Optional condition wait up to 30000 ms. Does not activate.",
        annotations(read_only_hint = true)
    )]
    async fn find_element(&self, Parameters(args): Parameters<FindElementArgs>) -> CallToolResult {
        self.json_tool(reqwest::Method::POST,&format!("/v1/devices/{}/ui/find",url_segment(&args.device_id)),Some(serde_json::json!({"provider":args.provider,"selector":{"identifier":args.identifier,"label":args.label,"role":args.role,"contains":args.contains},"timeout_ms":args.timeout_ms}))).await
    }
    #[tool(
        description = "Submit one activation of a current accessibility reference. Expired or changed references fail. Submitted does not verify a screen change; observe UI state. Never replay an uncertain result.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn activate_element(
        &self,
        Parameters(args): Parameters<ActivateElementArgs>,
    ) -> CallToolResult {
        self.json_tool(
            reqwest::Method::POST,
            &format!("/v1/devices/{}/ui/activate", url_segment(&args.device_id)),
            Some(serde_json::json!({"snapshot_id":args.snapshot_id,"element_id":args.element_id})),
        )
        .await
    }
    async fn json_tool(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> CallToolResult {
        match self.client.json_request(method, path, body.as_ref()).await {
            Ok(value)
                if matches!(path, "/v1/recovery/service" | "/v1/recovery/usb-interface")
                    && value["outcome"] != "completed" =>
            {
                CallToolResult::error(vec![ContentBlock::text(value.to_string())])
            }
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(problem_text(error))]),
        }
    }
    #[tool(
        description = "Read host/device diagnostics by layer, evidence, error codes and recovery steps. Does not capture the screen or restart services.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn diagnose(&self) -> CallToolResult {
        self.json_tool(reqwest::Method::GET, "/v1/diagnostics", None)
            .await
    }

    #[tool(
        description = "Read available capabilities, limits and recovery policy before choosing actions.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn capabilities(&self) -> CallToolResult {
        self.json_tool(reqwest::Method::GET, "/v1/capabilities", None)
            .await
    }

    #[tool(
        description = "Explicitly restart the host iOS multiplexer service and diagnose again. Disrupts ALL connected Apple devices. Requires enabled recovery in both MCP and daemon. Inspect outcome/code: completed does not prove device recovery. Never use in a retry loop.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn recover_service(&self) -> CallToolResult {
        if !self.enable_recovery_tools {
            return CallToolResult::error(vec![ContentBlock::text(
                "service_recovery_disabled: start the MCP adapter with --enable-recovery-tools and enable recovery on the daemon.",
            )]);
        }
        self.json_tool(reqwest::Method::POST, "/v1/recovery/service", None)
            .await
    }

    #[tool(
        description = "List attached iPhones and their available identity details.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn list_devices(&self) -> CallToolResult {
        self.json_tool(reqwest::Method::GET, "/v1/devices", None)
            .await
    }

    #[tool(
        description = "Restart only the XCTest runner configured by daemon --wda-runner. Requires --enable-recovery-tools, uses a 60-second cooldown, invalidates references, never replays gestures.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn recover_elements(&self) -> CallToolResult {
        if !self.enable_recovery_tools {
            return CallToolResult::error(vec![ContentBlock::text(
                "element_recovery_disabled: enable recovery tools and configure the daemon runner.",
            )]);
        }
        self.json_tool(reqwest::Method::POST, "/v1/recovery/elements", None)
            .await
    }

    #[tool(
        description = "Restart one exact, present Apple USB interface mapped to the daemon's scoped iPhone. Windows administrator only, explicit opt-in in both daemon and MCP, 60-second cooldown. Use diagnose.usb_recovery_targets, inspect outcome and diagnostics. Never reboots, reinstalls drivers or replays gestures.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn recover_usb_interface(
        &self,
        Parameters(args): Parameters<UsbRecoveryArgs>,
    ) -> CallToolResult {
        if !self.enable_recovery_tools {
            return CallToolResult::error(vec![ContentBlock::text(
                "usb_recovery_disabled: enable recovery tools and --allow-usb-interface-restart on the Windows daemon.",
            )]);
        }
        self.json_tool(
            reqwest::Method::POST,
            "/v1/recovery/usb-interface",
            Some(serde_json::json!({"device_id":args.device_id,"instance_id":args.instance_id})),
        )
        .await
    }

    #[tool(
        description = "Capture the current iPhone screen as a PNG image.",
        annotations(read_only_hint = true)
    )]
    async fn screenshot(
        &self,
        Parameters(args): Parameters<DeviceArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(match self.client.screenshot(&args.device_id).await {
            Ok(bytes) => CallToolResult::success(vec![ContentBlock::Image(ImageContent::new(
                STANDARD.encode(&bytes),
                "image/png",
            ))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(problem_text(error))]),
        })
    }
    #[tool(
        description = "Launch an installed iPhone app by bundle identifier. Do not retry a lost response without checking device state.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn launch_app(&self, Parameters(args): Parameters<LaunchArgs>) -> CallToolResult {
        let path = format!("/v1/devices/{}/apps", url_segment(&args.device_id));
        self.json_tool(
            reqwest::Method::POST,
            &path,
            Some(serde_json::json!({"bundle_id": args.bundle_id})),
        )
        .await
    }

    #[tool(
        description = "Tap the iPhone screen at display pixel coordinates. Do not replay an uncertain result.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn tap(&self, Parameters(args): Parameters<TapArgs>) -> CallToolResult {
        self.action(
            &args.device_id,
            serde_json::json!({"action":"tap","x":args.x,"y":args.y}),
        )
        .await
    }

    #[tool(
        description = "Press and hold the iPhone screen at display pixel coordinates.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn press(&self, Parameters(args): Parameters<PressArgs>) -> CallToolResult {
        self.action(&args.device_id, serde_json::json!({"action":"press","x":args.x,"y":args.y,"duration_ms":args.duration_ms})).await
    }

    #[tool(
        description = "Drag across the iPhone screen between display pixel coordinates.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn drag(&self, Parameters(args): Parameters<DragArgs>) -> CallToolResult {
        self.action(&args.device_id, serde_json::json!({"action":"drag","start_x":args.start_x,"start_y":args.start_y,"end_x":args.end_x,"end_y":args.end_y,"duration_ms":args.duration_ms})).await
    }

    #[tool(
        description = "Type ASCII text using the iPhone's US keyboard mapping. Requires a focused input; never replay an uncertain result.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn type_text(&self, Parameters(args): Parameters<TypeTextArgs>) -> CallToolResult {
        self.action(
            &args.device_id,
            serde_json::json!({"action":"type_text","text":args.text}),
        )
        .await
    }
}

impl IosMcp {
    async fn action(&self, device_id: &str, body: serde_json::Value) -> CallToolResult {
        let action = match serde_json::from_value::<rust_ios_device_api::HidAction>(body) {
            Ok(value) => value,
            Err(_) => return CallToolResult::error(vec![ContentBlock::text("invalid_action")]),
        };
        match self.client.action(device_id, &action).await {
            Ok(()) => CallToolResult::success(vec![ContentBlock::text(
                "Action submitted; observe state to verify the result.",
            )]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(problem_text(error))]),
        }
    }
}
fn problem_text(error: ClientError) -> String {
    error
        .problem
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok())
        .unwrap_or_else(|| {
            serde_json::json!({"code":error.code,"message":error.message}).to_string()
        })
}
fn url_segment(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn parse_daemon_url(input: &str) -> Result<url::Url> {
    let url = url::Url::parse(input).context("invalid daemon URL")?;
    anyhow::ensure!(
        url.scheme() == "http",
        "MCP adapter requires plain HTTP to the loopback daemon"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "daemon URL must not contain credentials"
    );
    anyhow::ensure!(
        url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
        "daemon URL must contain only an origin"
    );
    let ip = match url.host().context("daemon URL must have a host")? {
        url::Host::Ipv4(ip) => std::net::IpAddr::V4(ip),
        url::Host::Ipv6(ip) => std::net::IpAddr::V6(ip),
        url::Host::Domain(_) => anyhow::bail!("daemon host must be a loopback IP literal"),
    };
    anyhow::ensure!(
        ip.is_loopback(),
        "MCP adapter only connects to a loopback daemon"
    );
    Ok(url)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        !args.token.trim().is_empty(),
        "RUST_IOS_DEVICE_TOKEN is required"
    );
    let url = parse_daemon_url(&args.url)?;
    let service = IosMcp {
        client: DeviceClient::new(url.as_str(), &args.token)?,
        enable_recovery_tools: args.enable_recovery_tools,
    }
    .serve(stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_daemon_url;

    #[test]
    fn daemon_url_accepts_only_loopback_http_origins() {
        assert!(parse_daemon_url("http://127.0.0.1:18087").is_ok());
        assert!(parse_daemon_url("http://[::1]:18087/").is_ok());
        assert!(parse_daemon_url("https://127.0.0.1:18087").is_err());
        assert!(parse_daemon_url("http://user:secret@127.0.0.1:18087").is_err());
        assert!(parse_daemon_url("http://127.0.0.1:18087@evil.example").is_err());
        assert!(parse_daemon_url("http://192.168.1.20:18087").is_err());
        assert!(parse_daemon_url("http://127.0.0.1:18087/api").is_err());
    }
}
