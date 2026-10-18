use super::*;
use axum::{extract::Request, middleware::Next, response::IntoResponse};
use rust_ios_device_api::{ApiProblem, DiagnosticReport, RecoveryReport};
use std::time::{Duration, Instant};

pub async fn capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let failure = *state.runner_failure.lock().await;
    Ok(Json(serde_json::json!({
        "api_version": API_VERSION, "platform": std::env::consts::OS,
        "features": {"live_touch":{"supported":true,"requires_active_video":true,"coordinate_units":"physical_pixels","lease_ms":750,"ordered_sequences":true,"transport":"active CoreDevice media session; same availability as screen.video"},"live_screen":{"implemented":true,"endpoint":"GET /v1/devices/{device_id}/screen.video","encoding":"HEVC Annex-B access units with TetherDeck wire header v1","target_fps":60,"readiness":"probe with a live stream; actual FPS depends on device, decoder and network","command_gate_independent":true,"shared_hid_session":true,"automatic_reconnect":false},"legacy_screen":{"endpoint":"GET /v1/devices/{device_id}/screen.mjpeg","encoding":"JPEG captures","max_fps":10},"screenshot_png": true, "hid": ["tap","press","drag","type_text","key_press","device_button"], "device_buttons":["home","app_switcher","volume_up","volume_down","lock"], "launch_app": true,
            "diagnostics": true, "service_restart": state.allow_service_restart && rust_ios_device_diagnostics::managed_endpoint(),
            "ui_elements": {"implemented":true,"last_startup_failure":failure.map(|(stage,kind)|serde_json::json!({"stage":stage,"kind":kind})),"readiness":"probe with diagnostics or UI query","providers":["wda","ios_ax"],"wda_port":state.wda_port,"require_wda":state.require_wda,"runner_configured":state.runner_bundle.is_some(),"runner_start_allowed":state.runner_bundle.is_some() && (rust_ios_device_core::wireless::id().is_none() || state.wda_tunnel_binding_reviewed),"wifi_runner_binding_reviewed":state.wda_tunnel_binding_reviewed,"wifi_runner_review_scope":state.runner_bundle.as_deref(),"transport":"USB usbmux or explicitly started WDA inside paired Wi-Fi TLS/RSD tunnel; never LAN HTTP","provider_selection":["auto","wda","ios_ax"],"provider_capabilities":{"ios_ax":{"inspection":true,"activation":false,"hierarchy":false,"bounds":false,"traversal_may_scroll":true,"requires_wda":false},"wda":{"inspection":true,"activation":true,"hierarchy":true,"bounds_units":"points","requires_signed_runner":true,"wifi_requires":"reviewed runner honoring USE_IP for HTTP and MJPEG; RemotePairing record; advertised testmanagerd/DVT services","wifi_validation":"experimental; validate runner and device/iOS combination","lan_http_fallback":false}},"discovery":"GET /v1/devices/{device_id}/ui","requires":"paired, unlocked device with Developer Mode/DDI; signed WDA runner for activation","coverage":"WDA accessibility tree; AX flat inspection may scroll app","reference_ttl_seconds":30,"activation_verification":"observe fresh state after submission"}},
        "transport": {"direct_paired_wifi": rust_ios_device_core::wireless::id().is_some(),"usbmux": true, "rsd_configured": state.has_rsd(), "rsd_discovery": state.rsd_discovery.is_some(), "rsd_endpoint_refresh":if state.rsd_discovery.is_some() {"trusted local tunneld before each request; verify scoped UDID and pin RSD UUID"} else {"static configuration"}},
        "usb_interface_restart": cfg!(windows) && state.allow_usb_interface_restart && rust_ios_device_diagnostics::managed_endpoint(),
        "limits": {"text_bytes":4096, "text_layout":"US hardware layout; ASCII and supported Latin accents","key_press_modifiers":"Ctrl=1, Shift=2, Alt=4, Command=8","bottom_edge_gestures":true, "gesture_duration_ms":[1,60000]},
        "recovery": {"automatic_action_replay":false, "restart_cooldown_seconds":60, "scope":"host service; affects all connected Apple devices"},
        "diagnostics_scope": "host USB inventory plus devices allowed by --device"
    })))
}

pub async fn diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DiagnosticReport>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _permit = acquire_device(&state).await?;
    let mut report = rust_ios_device_diagnostics::diagnose(
        state.rsd,
        state.device.as_deref().map(String::as_str),
        state.allow_service_restart,
    )
    .await;
    enrich_diagnostics(&state, &mut report).await;
    if rust_ios_device_core::wireless::id().is_some()
        && state.runner_bundle.is_some()
        && !state.wda_tunnel_binding_reviewed
    {
        report.checks.push(rust_ios_device_api::DiagnosticCheck {
            layer: "elements".into(), status: rust_ios_device_api::CheckStatus::Warn,
            code: "wda_wifi_binding_review_required".into(), elapsed_ms: 0,
            message: "AX está disponível para inspeção sem runner. O WDA Wi-Fi não foi iniciado: falta revisar o binding HTTP e MJPEG deste runner ao túnel.".into(),
            remediation: vec!["Revisar a origem do runner e o suporte a USE_IP em HTTP e MJPEG (WDA 16.15.3 ou posterior compatível). A revisão vale somente para o Bundle ID configurado; não valida um IPA arbitrário.".into()],
        });
    }
    if let Some((stage, kind)) = *state.runner_failure.lock().await {
        report.checks.push(rust_ios_device_api::DiagnosticCheck {
            layer: "elements".into(), status: rust_ios_device_api::CheckStatus::Warn,
            code: format!("wda_{stage}_{kind}"), elapsed_ms: 0,
            message: format!("O runner WDA falhou na etapa {stage} ({kind}). A leitura AX e o vídeo são serviços independentes."),
            remediation: vec![elements::runner_remediation(stage).into()],
        });
    }
    for device in report.devices.iter().filter(|d| d.udid.is_some()).take(3) {
        let start = Instant::now();
        let ready = rust_ios_device_core::elements::probe_wda(&device.device_id, state.wda_port)
            .await
            .is_ok();
        report.elapsed_ms += start.elapsed().as_millis() as u64;
        report.checks.push(rust_ios_device_api::DiagnosticCheck {
            layer:"elements".into(),status:if ready {rust_ios_device_api::CheckStatus::Pass}else{rust_ios_device_api::CheckStatus::Warn},
            code:if ready {"wda_ready"}else{"wda_unavailable"}.into(),
            message:if ready {"O WebDriverAgent respondeu na porta configurada."} else {"A conexão com o aparelho foi confirmada, mas o WDA não respondeu. Captura e HID podem funcionar independentemente."}.into(),
            elapsed_ms:start.elapsed().as_millis() as u64,
            remediation:if ready {vec![]}else{vec!["Conferir runner instalado, assinatura de desenvolvimento válida, Developer Mode e DDI. Com --wda-runner configurado, usar Reiniciar automação.".into()]},
        });
    }
    Ok(Json(report))
}

pub async fn recover(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RecoveryReport>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !state.allow_service_restart || !rust_ios_device_diagnostics::managed_endpoint() {
        return Err(StatusCode::FORBIDDEN);
    }
    let _permit = acquire_device(&state).await?;
    reserve_recovery(&state.last_recovery).await?;
    state.elements.lock().await.clear();
    state.runner.lock().await.take();
    let mut report = rust_ios_device_diagnostics::recover(
        state.rsd,
        state.device.as_deref().map(String::as_str),
    )
    .await;
    enrich_diagnostics(&state, &mut report.diagnostics).await;
    Ok(Json(report))
}

pub async fn recover_elements(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let (Some(bundle), Some(device)) = (&state.runner_bundle, &state.device) else {
        return Err(StatusCode::FORBIDDEN);
    };
    if rust_ios_device_core::wireless::id().is_some() && !state.wda_tunnel_binding_reviewed {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    let _permit = acquire_device(&state).await?;
    reserve_recovery(&state.last_recovery).await?;
    state.elements.lock().await.clear();
    let mut owned = state.runner.lock().await;
    owned.take();
    *state.runner_failure.lock().await = None;
    let runner = match rust_ios_device_core::elements::start_runner_with_binding_review(
        device,
        bundle,
        state.wda_port,
        state.wda_tunnel_binding_reviewed,
    )
    .await
    {
        Ok(runner) => runner,
        Err(error) => {
            let (stage, kind) = rust_ios_device_core::elements::startup_failure(&error);
            *state.runner_failure.lock().await = Some((stage, kind));
            tracing::warn!(stage, kind, "WDA recovery failed");
            return Err(StatusCode::FAILED_DEPENDENCY);
        }
    };
    *owned = Some(runner);
    Ok(Json(
        serde_json::json!({"outcome":"ready","provider":"wda","automatic_action_replay":false}),
    ))
}

pub async fn recover_usb(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<rust_ios_device_api::UsbRecoveryRequest>,
) -> Result<Json<rust_ios_device_api::UsbRecoveryReport>, Response> {
    let failure = |status, code: &str, message: &str, unknown| {
        let problem = ApiProblem { code: code.into(), message: message.into(), request_id: String::new(), retryable: false,
            execution_state: if unknown { "unknown" } else { "not_started" }.into(),
            remediation: vec!["Consultar diagnóstico após a reenumeração. Não repetir gestos pendentes nem reiniciar o computador automaticamente.".into()] };
        let mut response = (status, Json(problem.clone())).into_response();
        response.extensions_mut().insert(problem);
        response
    };
    if !authorized(&headers, &state) {
        return Err(failure(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Token ou versão inválidos.",
            false,
        ));
    }
    if !cfg!(windows)
        || !state.allow_usb_interface_restart
        || !rust_ios_device_diagnostics::managed_endpoint()
    {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "usb_recovery_disabled",
            "Recuperação USB não habilitada neste daemon Windows.",
            false,
        ));
    }
    if state.device.as_deref().map(String::as_str) != Some(request.device_id.as_str()) {
        return Err(failure(
            StatusCode::FORBIDDEN,
            "device_access_denied",
            "Aparelho fora do escopo autorizado.",
            false,
        ));
    }
    if request.instance_id.len() > 1024 || request.instance_id.is_empty() {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "invalid_usb_instance",
            "Informe a instância exata presente no diagnóstico.",
            false,
        ));
    }
    let _permit = acquire_device(&state)
        .await
        .map_err(|s| failure(s, "device_busy", "Outra operação está em andamento.", false))?;
    // Validate again immediately before changing host state. The client cannot choose another USB device.
    let targets = rust_ios_device_diagnostics::usb_recovery::inventory()
        .await
        .map_err(|_| {
            failure(
                StatusCode::FAILED_DEPENDENCY,
                "usb_inventory_unavailable",
                "Não foi possível confirmar a interface atual.",
                false,
            )
        })?;
    if !targets
        .iter()
        .any(|t| t.device_id == request.device_id && t.instance_id == request.instance_id)
    {
        return Err(failure(
            StatusCode::CONFLICT,
            "usb_target_not_present",
            "A interface não pertence ao aparelho autorizado ou não está mais presente.",
            false,
        ));
    }
    reserve_recovery(&state.last_recovery).await.map_err(|s| {
        failure(
            s,
            "recovery_cooldown",
            "Aguardar 60 segundos entre recuperações.",
            false,
        )
    })?;
    state.elements.lock().await.clear();
    state.runner.lock().await.take();
    let mut report = rust_ios_device_diagnostics::recover_usb_interface(
        &request.device_id,
        &request.instance_id,
        state.rsd,
        state.allow_service_restart,
    )
    .await;
    enrich_diagnostics(&state, &mut report.diagnostics).await;
    Ok(Json(report))
}

async fn enrich_diagnostics(state: &AppState, report: &mut DiagnosticReport) {
    report.transport_history = Some(state.transport_history.lock().await.clone());
    if let Some(discovery) = &state.rsd_discovery {
        discovery.augment_diagnostics(report).await;
    }
}

async fn reserve_recovery(
    last_recovery: &tokio::sync::Mutex<Option<Instant>>,
) -> Result<(), StatusCode> {
    let mut last = last_recovery.lock().await;
    if last.is_some_and(|time| time.elapsed() < Duration::from_secs(60)) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    *last = Some(Instant::now());
    Ok(())
}

pub async fn envelope(request: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let write = request.method() == axum::http::Method::POST;
    let recovery = request.uri().path() == "/v1/recovery/service";
    let element_recovery = request.uri().path() == "/v1/recovery/elements";
    let mut response = next.run(request).await;
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let (code, message, hint) = match status {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => (
                "invalid_request",
                "Parâmetros inválidos.",
                "Consultar /v1/capabilities e conferir a requisição.",
            ),
            StatusCode::METHOD_NOT_ALLOWED => (
                "method_not_allowed",
                "Método HTTP não permitido nesta rota.",
                "Conferir o método documentado para a rota.",
            ),
            StatusCode::PAYLOAD_TOO_LARGE => (
                "payload_too_large",
                "O conteúdo excede o limite da requisição.",
                "Consultar os limites em /v1/capabilities.",
            ),
            StatusCode::UNAUTHORIZED => (
                "authentication_or_version_required",
                "Token ausente/inválido ou versão do protocolo incompatível.",
                "Conferir token e X-Protocol-Version; /v1/health informa a versão.",
            ),
            StatusCode::FORBIDDEN if recovery => (
                "service_recovery_disabled",
                "O daemon não habilitou recuperação do serviço.",
                "No host, iniciar o daemon com --allow-service-restart ou executar --recover-service localmente.",
            ),
            StatusCode::FORBIDDEN if element_recovery => (
                "element_recovery_disabled",
                "Não há runner configurado para recuperação.",
                "Iniciar o daemon com --wda-runner e --device.",
            ),
            StatusCode::FAILED_DEPENDENCY if element_recovery => (
                "runner_restart_failed",
                "O runner não concluiu a inicialização.",
                "Verificar instalação, assinatura de desenvolvimento, Developer Mode/DDI e diagnóstico antes de repetir.",
            ),
            StatusCode::LOCKED => (
                "device_busy",
                "Há outra operação em andamento; esta requisição não foi iniciada.",
                "Aguardar a operação atual; não reiniciar o serviço por causa de uma fila ocupada.",
            ),
            StatusCode::FORBIDDEN => (
                "device_access_denied",
                "O aparelho não está no escopo autorizado.",
                "Usar o identificador permitido pelo daemon.",
            ),
            StatusCode::NOT_FOUND => (
                "device_or_route_not_found",
                "Aparelho ou rota não encontrado.",
                "Atualizar a listagem; identificadores antigos não devem ser reutilizados sem verificação.",
            ),
            StatusCode::TOO_MANY_REQUESTS => (
                "recovery_cooldown",
                "A recuperação foi solicitada recentemente.",
                "Aguardar 60 segundos entre solicitações; consultar diagnóstico antes de repetir.",
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE => (
                "invalid_media",
                "Tipo de conteúdo inválido na requisição ou na captura.",
                "Usar application/json para ações; se a falha for na captura, executar diagnóstico.",
            ),
            _ => (
                "device_service_unavailable",
                "O daemon não pôde completar a operação ou está ocupado.",
                "Executar /v1/diagnostics; não repetir ações cujo resultado é incerto.",
            ),
        };
        let uncertain = write && status.is_server_error();
        response = (
            status,
            Json(
                response
                    .extensions()
                    .get::<ApiProblem>()
                    .cloned()
                    .map(|mut problem| {
                        problem.request_id = request_id.clone();
                        problem
                    })
                    .unwrap_or(ApiProblem {
                        code: code.into(),
                        message: message.into(),
                        request_id: request_id.clone(),
                        retryable: !write && status.is_server_error(),
                        execution_state: if uncertain { "unknown" } else { "not_started" }.into(),
                        remediation: vec![hint.into()],
                    }),
            ),
        )
            .into_response();
    }
    response
        .headers_mut()
        .insert("x-request-id", request_id.parse().expect("UUID header"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn errors_distinguish_safe_read_retry_from_uncertain_write() {
        for (method, status, uncertain, retryable) in [
            ("POST", StatusCode::SERVICE_UNAVAILABLE, true, false),
            ("GET", StatusCode::SERVICE_UNAVAILABLE, false, true),
            ("POST", StatusCode::BAD_REQUEST, false, false),
        ] {
            let app = Router::new()
                .route(
                    "/v1/devices/test/actions",
                    axum::routing::any(move || async move { status }),
                )
                .layer(axum::middleware::from_fn(envelope));
            let response = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/v1/devices/test/actions")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let request_id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let bytes = axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap();
            let problem: ApiProblem = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(problem.request_id, request_id);
            assert_eq!(problem.retryable, retryable);
            assert_eq!(problem.execution_state == "unknown", uncertain);
        }
    }

    #[tokio::test]
    async fn concurrent_recovery_requests_reserve_only_one_slot() {
        let last = tokio::sync::Mutex::new(None);
        let (first, second) = tokio::join!(reserve_recovery(&last), reserve_recovery(&last));
        assert!(first.is_ok());
        assert_eq!(second.unwrap_err(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn disabled_recovery_returns_before_touching_service() {
        let state = AppState {
            screen_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            video_control: Arc::new(tokio::sync::Mutex::new(None)),
            rsd_discovery: None,
            transport_history: Arc::new(tokio::sync::Mutex::new(Default::default())),
            runner_bundle: None,
            runner: Arc::new(tokio::sync::Mutex::new(None)),
            wda_port: 8100,
            require_wda: false,
            elements: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            token: Arc::new("test".into()),
            device: None,
            rsd: None,
            gate: Arc::new(tokio::sync::Semaphore::new(1)),
            allow_service_restart: false,
            allow_usb_interface_restart: false,
            last_recovery: Arc::new(tokio::sync::Mutex::new(None)),
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test".parse().unwrap());
        headers.insert("x-protocol-version", "1".parse().unwrap());
        let result = recover(State(state), headers).await;
        assert_eq!(result.unwrap_err(), StatusCode::FORBIDDEN);
    }
}
