//! Native AX inspector over authenticated CoreDevice. Protocol reference: go-ios (MIT).
use crate::DeviceError;
use idevice::{
    ReadWrite,
    provider::RsdProvider,
    services::dvt::{
        message::{AuxValue, Message},
        remote_server::RemoteServerClient,
    },
};
use plist::{Dictionary, Value};
use rust_ios_device_api::{UiElement, UiSnapshot};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
type AxResult<T> = Result<T, DeviceError>;
fn dict(items: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Dictionary(
        items
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Dictionary>(),
    )
}
fn pass(value: Value) -> Value {
    dict([("ObjectType", "passthrough".into()), ("Value", value)])
}
fn element_argument(token: &[u8]) -> Value {
    dict([
        ("ObjectType", "AXAuditElement_v1".into()),
        (
            "Value",
            pass(dict([(
                "PlatformElementValue_v1",
                pass(Value::Data(token.to_vec())),
            )])),
        ),
    ])
}
fn attribute_argument(name: &str) -> Value {
    dict([
        ("ObjectType", "AXAuditElementAttribute_v1".into()),
        (
            "Value",
            pass(dict([("AttributeNameValue_v1", pass(name.into()))])),
        ),
    ])
}
fn unwrapped(value: &Value) -> &Value {
    if let Some(d) = value.as_dictionary() {
        if d.contains_key("ObjectType") {
            if let Some(v) = d.get("Value") {
                return unwrapped(v);
            }
        }
    }
    value
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let value = unwrapped(value);
    if let Some(d) = value.as_dictionary() {
        if let Some(v) = d.get(key) {
            return Some(unwrapped(v));
        }
        for v in d.values() {
            if let Some(found) = field(v, key) {
                return Some(found);
            }
        }
    }
    if let Some(values) = value.as_array() {
        for v in values {
            if let Some(found) = field(v, key) {
                return Some(found);
            }
        }
    }
    None
}
fn ax_failure(stage: &'static str, error: DeviceError) -> DeviceError {
    // Only fixed classifications cross the API. Never retain protocol/audit payloads.
    let kind = match error {
        DeviceError::AxProviderFailed { kind, .. } => kind,
        DeviceError::Timeout | DeviceError::Idevice(idevice::IdeviceError::Timeout) => "timeout",
        DeviceError::WifiPairingRequired => "pairing_required",
        DeviceError::DeviceIdMismatch => "device_mismatch",
        DeviceError::Idevice(
            idevice::IdeviceError::UnexpectedResponse(_)
            | idevice::IdeviceError::Plist(_)
            | idevice::IdeviceError::Dvt(_)
            | idevice::IdeviceError::NotEnoughBytes(_, _)
            | idevice::IdeviceError::Utf8(_)
            | idevice::IdeviceError::Utf8Error
            | idevice::IdeviceError::MalformedCommand,
        ) => "protocol",
        DeviceError::Idevice(idevice::IdeviceError::DeviceLocked) => "locked",
        DeviceError::Idevice(idevice::IdeviceError::DeveloperModeNotEnabled) => {
            "developer_mode_required"
        }
        DeviceError::Idevice(idevice::IdeviceError::ImageNotMounted) => "ddi_required",
        DeviceError::Idevice(idevice::IdeviceError::InvalidHostID) => "pairing_required",
        DeviceError::Idevice(idevice::IdeviceError::ServiceNotFound) => "unsupported",
        DeviceError::RsdTransport(ref error) if error.kind() == std::io::ErrorKind::TimedOut => {
            "timeout"
        }
        DeviceError::Idevice(idevice::IdeviceError::Socket(ref error))
            if error.kind() == std::io::ErrorKind::TimedOut =>
        {
            "timeout"
        }
        _ => "unavailable",
    };
    DeviceError::AxProviderFailed { stage, kind }
}
fn bad_response() -> DeviceError {
    idevice::IdeviceError::UnexpectedResponse("AX response was not recognized".into()).into()
}
fn archived_argument(value: Value) -> AxResult<AuxValue> {
    let mut archive = ns_keyed_archive::encode::encode(value).map_err(|_| bad_response())?;
    if let Some(objects) = archive
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("$objects"))
        .and_then(Value::as_array_mut)
    {
        for object in objects {
            if let Some(class) = object.as_dictionary_mut() {
                if class.get("$classname").and_then(Value::as_string) == Some("NSDictionary") {
                    class.insert("$classname".into(), "NSMutableDictionary".into());
                    class.insert(
                        "$classes".into(),
                        Value::Array(vec![
                            "NSMutableDictionary".into(),
                            "NSDictionary".into(),
                            "NSObject".into(),
                        ]),
                    );
                }
            }
        }
    }
    let mut bytes = Vec::new();
    archive
        .to_writer_binary(&mut bytes)
        .map_err(|_| bad_response())?;
    Ok(AuxValue::Array(bytes))
}
fn aux_value(message: &Message) -> AxResult<Value> {
    match message.aux.as_ref().and_then(|a| a.values.first()) {
        Some(AuxValue::Array(bytes)) => {
            ns_keyed_archive::decode::from_bytes(bytes).map_err(|_| bad_response())
        }
        _ => Err(bad_response()),
    }
}
pub struct AxSession {
    remote: RemoteServerClient<Box<dyn ReadWrite>>,
    _tunnel: idevice::tcp::handle::AdapterHandle,
    device_id: String,
    snapshot_id: String,
}
impl AxSession {
    pub async fn connect(udid: String) -> AxResult<Self> {
        let mut stage = "transport";
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            let (mut tunnel, handshake) = super::usb_rsd_stack(udid.clone()).await?;
            stage = "service_not_advertised";
            let service = handshake
                .services
                .get("com.apple.accessibility.axAuditDaemon.remoteserver.shim.remote")
                .ok_or(idevice::IdeviceError::ServiceNotFound)?;
            stage = "service_connect";
            let stream = tunnel.connect_to_service_port(service.port).await?;
            let mut device = idevice::Idevice::new(stream, "rust-ios-device");
            stage = "checkin";
            device.rsd_checkin().await?;
            let remote = RemoteServerClient::new(
                device
                    .get_socket()
                    .ok_or(idevice::IdeviceError::NoEstablishedConnection)?,
            );
            let mut this = Self {
                remote,
                _tunnel: tunnel,
                device_id: udid,
                snapshot_id: String::new(),
            };
            stage = "capabilities";
            this.send(
                "_notifyOfPublishedCapabilities:",
                vec![dict([
                    ("com.apple.private.DTXBlockCompression", 0u64.into()),
                    ("com.apple.private.DTXConnection", 1u64.into()),
                ])],
                false,
            )
            .await?;
            let caps = this.call("deviceCapabilities", vec![]).await?;
            // go-ios validates a string-list response, without requiring a write
            // capability to inspect. Successful capability negotiation is not proof
            // that navigation will work; the bounded snapshot must still succeed.
            // https://github.com/danielpaulus/go-ios/blob/main/ios/accessibility/utils.go
            if !caps
                .as_array()
                .is_some_and(|values| values.iter().all(|value| value.as_string().is_some()))
            {
                return Err(bad_response());
            }
            stage = "configure";
            this.send("deviceInspectorShowVisuals:", vec![false.into()], false)
                .await?;
            this.send(
                "deviceInspectorSetMonitoredEventType:",
                vec![0u64.into()],
                false,
            )
            .await?;
            this.send("deviceSetAuditTargetPid:", vec![0u64.into()], false)
                .await?;
            this.send("deviceSetAppMonitoringEnabled:", vec![true.into()], false)
                .await?;
            this.send(
                "deviceInspectorShowIgnoredElements:",
                vec![false.into()],
                false,
            )
            .await?;
            Ok(this)
        })
        .await;
        match result {
            Ok(result) => result.map_err(|error| ax_failure(stage, error)),
            Err(_) => Err(DeviceError::AxProviderFailed {
                stage,
                kind: "timeout",
            }),
        }
    }
    async fn send(&mut self, method: &str, args: Vec<Value>, reply: bool) -> AxResult<u32> {
        let args = if args.is_empty() {
            None
        } else {
            Some(
                args.into_iter()
                    .map(archived_argument)
                    .collect::<AxResult<Vec<_>>>()?,
            )
        };
        Ok(self
            .remote
            .send_invocation(0, Some(method), args, reply)
            .await?)
    }
    async fn receive(&mut self) -> AxResult<Message> {
        let message = tokio::time::timeout(Duration::from_secs(3), self.remote.read_message(0))
            .await
            .map_err(|_| DeviceError::Timeout)??;
        self.remote.acknowledge(&message).await?;
        Ok(message)
    }
    async fn call(&mut self, method: &str, args: Vec<Value>) -> AxResult<Value> {
        let identifier = self.send(method, args, true).await?;
        for _ in 0..128 {
            let message = self.receive().await?;
            if message.message_header.conversation_index() > 0
                && message.message_header.identifier() == identifier
            {
                return Ok(message.data.unwrap_or_else(|| Value::String(String::new())));
            }
        }
        Err(bad_response())
    }
    async fn moved(&mut self, direction: u64) -> AxResult<Value> {
        self.send(
            "deviceInspectorMoveWithOptions:",
            vec![pass(dict([
                ("allowNonAX", pass(false.into())),
                ("direction", pass(direction.into())),
                ("includeContainers", pass(true.into())),
            ]))],
            false,
        )
        .await?;
        self.element_event().await
    }
    async fn element_event(&mut self) -> AxResult<Value> {
        for _ in 0..128 {
            let message = self.receive().await?;
            if message.data.as_ref().and_then(Value::as_string)
                == Some("hostInspectorCurrentElementChanged:")
            {
                return aux_value(&message);
            }
        }
        Err(bad_response())
    }
    async fn attribute(&mut self, token: &[u8], name: &str) -> AxResult<Value> {
        self.call(
            "deviceElement:valueForAttribute:",
            vec![element_argument(token), attribute_argument(name)],
        )
        .await
        .map(|v| unwrapped(&v).clone())
    }
    pub async fn snapshot(&mut self) -> AxResult<UiSnapshot> {
        let result = tokio::time::timeout(Duration::from_secs(25), async {
            self.send(
                "deviceInspectorSetMonitoredEventType:",
                vec![2u64.into()],
                false,
            )
            .await?;
            self.send(
                "deviceInspectorFocusOnElement:",
                vec!["$null".into()],
                false,
            )
            .await?;
            let _ = self.element_event().await;
            self.snapshot_inner().await
        })
        .await
        .map_err(|_| DeviceError::Timeout)
        .and_then(|v| v)
        .map_err(|error| ax_failure("snapshot", error));
        let cleanup = tokio::time::timeout(
            Duration::from_secs(1),
            self.send(
                "deviceInspectorSetMonitoredEventType:",
                vec![0u64.into()],
                false,
            ),
        )
        .await;
        let cleanup = cleanup
            .map_err(|_| DeviceError::Timeout)
            .and_then(|v| v)
            .map_err(|error| ax_failure("cleanup", error));
        // Preserve the primary failure even if stopping observation also fails.
        match result {
            Err(error) => Err(error),
            Ok(snapshot) => {
                cleanup?;
                Ok(snapshot)
            }
        }
    }
    async fn snapshot_inner(&mut self) -> AxResult<UiSnapshot> {
        self.snapshot_id = uuid::Uuid::new_v4().to_string();
        let start = Instant::now();
        let mut elements = Vec::new();
        let mut seen = Vec::<Vec<u8>>::new();
        let mut complete = false;
        for index in 0..200 {
            let notification = self.moved(if index == 0 { 5 } else { 4 }).await?;
            let Some(token) = field(&notification, "PlatformElementValue_v1")
                .and_then(Value::as_data)
                .map(<[u8]>::to_vec)
            else {
                break;
            };
            if seen.contains(&token) {
                complete = true;
                break;
            }
            seen.push(token.clone());
            let caption = field(&notification, "SpokenDescriptionValue_v1")
                .or_else(|| field(&notification, "CaptionTextValue_v1"))
                .and_then(Value::as_string)
                .unwrap_or("")
                .to_owned();
            let label = self
                .attribute(&token, "Label")
                .await?
                .as_string()
                .map(str::to_owned)
                .unwrap_or(caption);
            let identifier = self
                .attribute(&token, "Identifier")
                .await?
                .as_string()
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            let element_id = uuid::Uuid::new_v4().to_string();
            elements.push(UiElement {
                actions: vec![],
                parent_id: None,
                depth: 0,
                visible: None,
                value: None,
                element_id,
                identifier,
                label,
                role: None,
                enabled: None,
                bounds: None,
            });

            if start.elapsed() >= Duration::from_secs(15) {
                break;
            }
        }
        Ok(UiSnapshot { app_bundle_id:None,bounds_units:None,activation_supported:false,device_id:self.device_id.clone(),snapshot_id:self.snapshot_id.clone(),provider:"ios_ax".into(),created_at_unix_ms:SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,complete,elements,warnings:vec!["AX read only: use signed WDA for ordinary app control. Traversal may scroll the app. Bounds, hierarchy and roles are unknown; complete only indicates a traversal cycle.".into()] })
    }
}
pub async fn probe(udid: String) -> AxResult<UiSnapshot> {
    AxSession::connect(udid).await?.snapshot().await
}
