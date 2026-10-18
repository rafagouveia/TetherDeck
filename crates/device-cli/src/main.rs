use clap::{Parser, Subcommand};
use rust_ios_device_api::HidAction;
use rust_ios_device_client::DeviceClient;
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    name = "rust-ios-device",
    about = "Remote client for the Rust iOS Device daemon"
)]
struct Args {
    #[arg(
        long,
        env = "RUST_IOS_DEVICE_URL",
        default_value = "http://127.0.0.1:18087"
    )]
    url: String,
    #[arg(long, env = "RUST_IOS_DEVICE_TOKEN")]
    token: String,
    /// Emit machine-readable action results and errors.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read native accessibility elements without capturing an image.
    Ui {
        device_id: String,
        #[arg(long, default_value = "auto", value_parser = parse_provider)]
        provider: rust_ios_device_api::ElementProvider,
    },
    /// Find exactly one element; wait up to timeout-ms for a match.
    FindElement {
        device_id: String,
        #[arg(long, default_value = "auto", value_parser = parse_provider)]
        provider: rust_ios_device_api::ElementProvider,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        identifier: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        contains: bool,
        #[arg(long, default_value_t = 0)]
        timeout_ms: u64,
    },
    /// Activate a current reference. This submits one action; inspect the next UI state.
    ActivateElement {
        device_id: String,
        snapshot_id: String,
        element_id: String,
    },
    Devices,
    /// Diagnose the daemon host by layer (read only).
    Doctor,
    /// Report available features, limits and recovery policy.
    Capabilities,
    /// Explicitly restart the host multiplexer service, if enabled on the daemon.
    RecoverService,
    /// Restart only the signed runner configured by the daemon's --wda-runner.
    RecoverElements,
    /// Restart one exact Apple USB interface from the diagnostic inventory (Windows administrator).
    RecoverUsbInterface {
        device_id: String,
        instance_id: String,
    },
    Tap {
        device_id: String,
        x: u16,
        y: u16,
    },
    Press {
        device_id: String,
        x: u16,
        y: u16,
        duration_ms: u32,
    },
    Drag {
        device_id: String,
        start_x: u16,
        start_y: u16,
        end_x: u16,
        end_y: u16,
        duration_ms: u32,
    },
    TypeText {
        device_id: String,
        text: String,
    },
    Launch {
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
        bundle_id: String,
    },
    Screenshot {
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let machine = args.json;
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if machine {
                if let Some(error) = error.downcast_ref::<rust_ios_device_client::ClientError>() {
                    eprintln!(
                        "{}",
                        serde_json::json!({"ok":false,"code":error.code,"message":error.message,"problem":error.problem})
                    );
                } else {
                    eprintln!(
                        "{}",
                        serde_json::json!({"ok":false,"code":"client_failed","message":error.to_string()})
                    );
                }
            } else {
                eprintln!("{error}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> anyhow::Result<()> {
    let client = DeviceClient::new(&args.url, &args.token)?;
    client.health().await?;
    match args.command {
        Command::Ui {
            device_id,
            provider,
        } => println!(
            "{}",
            serde_json::to_string_pretty(
                &client
                    .ui_snapshot_with_provider(&device_id, provider)
                    .await?
            )?
        ),
        Command::FindElement {
            device_id,
            provider,
            role,
            identifier,
            label,
            contains,
            timeout_ms,
        } => println!(
            "{}",
            serde_json::to_string_pretty(
                &client
                    .find_element(
                        &device_id,
                        &rust_ios_device_api::UiFindRequest {
                            provider,
                            selector: rust_ios_device_api::UiSelector {
                                role,
                                identifier,
                                label,
                                contains
                            },
                            timeout_ms
                        }
                    )
                    .await?
            )?
        ),
        Command::ActivateElement {
            device_id,
            snapshot_id,
            element_id,
        } => println!(
            "{}",
            client
                .activate_element(
                    &device_id,
                    &rust_ios_device_api::UiActivateRequest {
                        snapshot_id,
                        element_id
                    }
                )
                .await?
        ),
        Command::RecoverElements => println!("{}", client.recover_elements().await?),
        Command::RecoverUsbInterface {
            device_id,
            instance_id,
        } => {
            let report = client
                .recover_usb_interface(&rust_ios_device_api::UsbRecoveryRequest {
                    device_id,
                    instance_id,
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::ensure!(
                report.outcome == "completed",
                "USB recovery did not complete: {}",
                report.code
            );
        }
        Command::Doctor => println!(
            "{}",
            serde_json::to_string_pretty(&client.diagnostics().await?)?
        ),
        Command::Capabilities => println!(
            "{}",
            serde_json::to_string_pretty(&client.capabilities().await?)?
        ),
        Command::RecoverService => {
            let report = client.recover_service().await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::ensure!(
                report.outcome == "completed",
                "Service recovery failed: {}",
                report.code
            );
        }
        Command::Devices => {
            println!(
                "{}",
                serde_json::to_string_pretty(&client.devices().await?)?
            );
        }
        Command::Launch {
            device_id,
            bundle_id,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&client.launch(&device_id, &bundle_id).await?)?
            );
        }
        Command::Tap { device_id, x, y } => {
            send_hid_action(&client, args.json, device_id, HidAction::Tap { x, y }).await?;
        }
        Command::Press {
            device_id,
            x,
            y,
            duration_ms,
        } => {
            send_hid_action(
                &client,
                args.json,
                device_id,
                HidAction::Press { x, y, duration_ms },
            )
            .await?;
        }
        Command::Drag {
            device_id,
            start_x,
            start_y,
            end_x,
            end_y,
            duration_ms,
        } => {
            send_hid_action(
                &client,
                args.json,
                device_id,
                HidAction::Drag {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    duration_ms,
                },
            )
            .await?;
        }
        Command::TypeText { device_id, text } => {
            send_hid_action(&client, args.json, device_id, HidAction::TypeText { text }).await?;
        }
        Command::Screenshot { device_id, output } => {
            tokio::fs::write(&output, client.screenshot(&device_id).await?).await?;
            if args.json {
                println!("{}", serde_json::json!({"ok":true,"output":output}));
            } else {
                println!("Wrote {}", output.display());
            }
        }
    }
    Ok(())
}

async fn send_hid_action(
    client: &DeviceClient,
    machine: bool,
    device_id: String,
    action: HidAction,
) -> anyhow::Result<()> {
    client.action(&device_id, &action).await?;
    if machine {
        println!(
            "{}",
            serde_json::json!({"ok":true,"device_id":device_id,"execution_state":"completed"})
        );
    } else {
        println!("HID action sent to {device_id}");
    }
    Ok(())
}

fn parse_provider(value: &str) -> Result<rust_ios_device_api::ElementProvider, String> {
    match value {
        "auto" => Ok(rust_ios_device_api::ElementProvider::Auto),
        "wda" => Ok(rust_ios_device_api::ElementProvider::Wda),
        "ios_ax" | "ios-ax" | "ax" => Ok(rust_ios_device_api::ElementProvider::IosAx),
        _ => Err("Use auto, wda or ios_ax".into()),
    }
}
