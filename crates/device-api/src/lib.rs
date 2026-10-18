use serde::{Deserialize, Serialize};

pub const API_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElement {
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub value: Option<String>,
    pub element_id: String,
    pub identifier: Option<String>,
    pub label: String,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    pub bounds: Option<UiRect>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshot {
    #[serde(default)]
    pub app_bundle_id: Option<String>,
    #[serde(default)]
    pub bounds_units: Option<String>,
    #[serde(default)]
    pub activation_supported: bool,
    pub device_id: String,
    pub snapshot_id: String,
    pub provider: String,
    pub created_at_unix_ms: u64,
    pub complete: bool,
    pub elements: Vec<UiElement>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiSelector {
    pub role: Option<String>,
    pub identifier: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub contains: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiActivateRequest {
    pub snapshot_id: String,
    pub element_id: String,
}
/// Explicit accessibility backend. AX is inspection only and may scroll during traversal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementProvider {
    #[default]
    Auto,
    Wda,
    #[serde(alias = "ax", alias = "ios-ax")]
    IosAx,
}
impl ElementProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Wda => "wda",
            Self::IosAx => "ios_ax",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiFindRequest {
    #[serde(default)]
    pub provider: ElementProvider,
    pub selector: UiSelector,
    #[serde(default)]
    pub timeout_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMatch {
    pub device_id: String,
    pub snapshot_id: String,
    #[serde(default)]
    pub provider: String,
    /// Coverage of the returned source, not proof of every possible screen control.
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub activation_supported: bool,
    pub element: UiElement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSummary {
    /// Stable identifier used in daemon routes (USB UDID or RSD handshake UUID).
    pub device_id: String,
    /// Hardware UDID when the transport exposes it.
    pub udid: Option<String>,
    pub transport: String,
    /// Lockdown identity access state; `available` does not imply every service is available.
    #[serde(default)]
    pub identity_status: IdentityStatus,
    pub name: Option<String>,
    pub product_type: Option<String>,
    pub product_version: Option<String>,
    pub build_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStatus {
    Available,
    Locked,
    Unpaired,
    ServiceUnavailable,
}

impl Default for IdentityStatus {
    fn default() -> Self {
        Self::ServiceUnavailable
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HidAction {
    DeviceButton {
        button: DeviceButton,
    },
    Tap {
        x: u16,
        y: u16,
    },
    Press {
        x: u16,
        y: u16,
        duration_ms: u32,
    },
    Drag {
        start_x: u16,
        start_y: u16,
        end_x: u16,
        end_y: u16,
        duration_ms: u32,
    },
    TypeText {
        text: String,
    },
}

/// One ordered sample of a live single-finger gesture in physical screen pixels.
/// Moves replace pending positions in the desktop; they are never replayed after End.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchPhase {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TouchRequest {
    pub gesture_id: String,
    pub sequence: u64,
    pub phase: TouchPhase,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchResponse {
    pub gesture_id: String,
    pub sequence: u64,
    pub active: bool,
}

/// A complete device-button press, including release. The app switcher is one
/// explicit double-Home sequence, never a retry of an uncertain Home action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceButton {
    Home,
    AppSwitcher,
    VolumeUp,
    VolumeDown,
    Lock,
}

/// Machine-readable failure. An uncertain write must never be blindly replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiProblem {
    pub code: String,
    pub message: String,
    pub request_id: String,
    pub retryable: bool,
    pub execution_state: String,
    pub remediation: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Unknown,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub layer: String,
    pub status: CheckStatus,
    pub code: String,
    pub message: String,
    pub elapsed_ms: u64,
    pub remediation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub location: String,
    /// May be empty when the OS driver does not expose interface descriptors.
    pub interfaces: Vec<UsbInterface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub state: String,
    pub process_id: Option<u32>,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub api_version: u16,
    pub generated_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub platform: String,
    pub architecture: String,
    pub health: String,
    pub recovery_enabled: bool,
    pub service: ServiceInfo,
    pub checks: Vec<DiagnosticCheck>,
    pub devices: Vec<DeviceSummary>,
    pub usb_devices: Vec<UsbDevice>,
    #[serde(default)]
    pub usb_recovery_targets: Vec<UsbRecoveryTarget>,
    #[serde(default)]
    pub transport_history: Option<TransportHistory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransportHistory {
    pub started_at_unix_ms: u64,
    pub sample_interval_ms: u64,
    pub samples: u64,
    pub last_sample_at_unix_ms: u64,
    /// Bounded transitions observed by polling; not physical USB event timestamps.
    pub observations: Vec<TransportObservation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportObservation {
    pub observed_at_unix_ms: u64,
    pub code: String,
    pub device_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbRecoveryTarget {
    pub device_id: String,
    pub instance_id: String,
    pub parent_instance_id: String,
    pub driver_service: String,
    #[serde(default)]
    pub driver_provider: Option<String>,
    #[serde(default)]
    pub driver_version: Option<String>,
    #[serde(default)]
    pub driver_inf: Option<String>,
    pub problem_code: Option<u32>,
    pub power_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbRecoveryRequest {
    pub device_id: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbRecoveryReport {
    pub action: String,
    pub outcome: String,
    pub code: String,
    pub instance_id: String,
    pub diagnostics: DiagnosticReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub action: String,
    /// `completed` means service control completed, not that the phone was recovered.
    pub outcome: String,
    pub code: String,
    pub elapsed_ms: u64,
    pub before: ServiceInfo,
    pub after: ServiceInfo,
    pub diagnostics: DiagnosticReport,
}
