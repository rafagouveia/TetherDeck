use super::*;
use axum::response::IntoResponse;
use rust_ios_device_api::{
    ApiProblem, ElementProvider, UiActivateRequest, UiFindRequest, UiMatch, UiSelector,
};
use rust_ios_device_core::elements::ElementSession;
use std::{collections::HashMap, time::Duration};

pub struct CachedSession {
    selected: ElementProvider,
    session: ElementSession,
}
impl std::ops::Deref for CachedSession {
    type Target = ElementSession;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
impl std::ops::DerefMut for CachedSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}
pub type Sessions = Arc<tokio::sync::Mutex<HashMap<String, CachedSession>>>;
#[derive(serde::Deserialize, Default)]
pub struct SnapshotQuery {
    #[serde(default)]
    provider: ElementProvider,
}

fn problem(status: StatusCode, code: &str, message: &str, unknown: bool) -> Response {
    let problem = ApiProblem { code:code.into(),message:message.into(),request_id:String::new(),retryable:false,
        execution_state:if unknown {"unknown"} else {"not_started"}.into(),
        remediation:vec!["Obter uma consulta atual; não repetir uma ativação de resultado incerto. Conferir conexão, desbloqueio e Developer Mode.".into()] };
    let mut response = (status, Json(problem.clone())).into_response();
    response.extensions_mut().insert(problem);
    response
}
fn ax_problem(stage: &'static str, kind: &'static str) -> Response {
    let remediation = match stage {
        "service_not_advertised" => {
            "O iOS/DDI atual não anunciou o serviço AXAudit remoto esperado. Verifique Developer Mode e DDI compatível; WDA é um provedor alternativo independente, sem fallback de ativação AX."
        }
        "checkin" => {
            "O serviço AX foi encontrado, mas recusou ou não concluiu o check-in RSD. Verifique a compatibilidade de iOS/DDI e consulte novamente com o aparelho desbloqueado."
        }
        "capabilities" => {
            "O serviço AX não completou a negociação de capacidades esperada. Este protocolo privado pode variar com o iOS; a mensagem não comprova ausência de elementos na tela."
        }
        "configure" => {
            "O serviço AX conectou, mas não aceitou a configuração do inspetor nesta versão do iOS."
        }
        "snapshot" => {
            "A sessão AX abriu, mas a consulta não terminou ou o formato recebido não foi reconhecido. A travessia pode rolar o app; nenhuma ativação por elemento foi enviada."
        }
        "cleanup" => {
            "A consulta terminou, mas não foi possível confirmar o encerramento da observação AX. Encerre a sessão de inspeção antes de consultar novamente."
        }
        _ => {
            "Verifique conexão pareada, identidade do aparelho, registro RemotePairing, Developer Mode e DDI. A leitura AX não precisa de runner WDA ou certificado de assinatura."
        }
    };
    let problem = ApiProblem {
        code: format!("ax_{stage}_{kind}"),
        message: format!("A leitura iOS AX falhou na etapa {stage} ({kind})."),
        request_id: String::new(),
        retryable: false,
        execution_state: "not_started".into(),
        remediation: vec![remediation.into()],
    };
    let status = if kind == "timeout" {
        StatusCode::REQUEST_TIMEOUT
    } else {
        StatusCode::FAILED_DEPENDENCY
    };
    let mut response = (status, Json(problem.clone())).into_response();
    response.extensions_mut().insert(problem);
    response
}
pub(super) fn runner_remediation(stage: &str) -> &'static str {
    match stage {
        "runner_not_installed" => {
            "O Bundle ID configurado não foi encontrado no aparelho. Confirme o runner assinado e instalado para este iPhone; não instale ou repita ações automaticamente."
        }
        "runner_lookup" | "runner_lockdown" => {
            "O serviço de metadados do runner não completou a consulta. No Wi-Fi direto use InstallationProxy pelo shim RSD autenticado; a falha não comprova que o runner esteja ausente."
        }
        "runner_metadata_service" | "runner_metadata_connect" | "runner_metadata_checkin" => {
            "Verifique Developer Mode, DDI e o serviço InstallationProxy shim anunciado pelo iOS no túnel pareado. Não é necessário expor HTTP WDA na LAN."
        }
        "runner_metadata_path" | "runner_metadata_container" | "runner_metadata_executable" => {
            "Os metadados de caminho, container ou executável do runner não foram fornecidos no formato XCTest esperado. Confira a origem do IPA e o Bundle ID exato."
        }
        _ => {
            "Confira assinatura válida para este iPhone, Developer Mode, DDI e compatibilidade XCTest do runner. Consulte o estágio no diagnóstico; não repita ativações com resultado incerto."
        }
    }
}
fn runner_startup_problem(stage: &'static str, kind: &'static str) -> Response {
    let value = ApiProblem {
        code: format!("wda_{stage}_{kind}"),
        message: format!("O runner WDA não iniciou: etapa {stage} ({kind})."),
        request_id: String::new(),
        retryable: false,
        execution_state: "not_started".into(),
        remediation: vec![runner_remediation(stage).into()],
    };
    let mut response = (StatusCode::FAILED_DEPENDENCY, Json(value.clone())).into_response();
    response.extensions_mut().insert(value);
    response
}
fn device_error(error: rust_ios_device_core::DeviceError) -> Response {
    use rust_ios_device_core::DeviceError;
    match error {
        DeviceError::AxProviderFailed { stage, kind } => ax_problem(stage, kind),
        DeviceError::ElementNotActionable => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "element_not_actionable",
            "Este nó não expõe uma ativação suportada. Selecione o controle interativo pelo tipo e identificador.",
            false,
        ),
        DeviceError::ElementSessionLost => problem(
            StatusCode::CONFLICT,
            "element_session_lost",
            "A sessão WDA foi substituída. Consulte os elementos novamente antes de agir.",
            false,
        ),
        DeviceError::ElementProviderFailed { stage, .. } => problem(
            StatusCode::FAILED_DEPENDENCY,
            &format!("wda_{stage}_failed"),
            &format!(
                "O WDA falhou na etapa {stage}. Consulte o diagnóstico e obtenha uma nova observação."
            ),
            false,
        ),
        DeviceError::MissingDevice(_) => problem(
            StatusCode::FAILED_DEPENDENCY,
            "device_disconnected",
            "O usbmux não encontra este aparelho. Verifique o diagnóstico de conexão.",
            false,
        ),
        DeviceError::Timeout => problem(
            StatusCode::REQUEST_TIMEOUT,
            "element_query_timeout",
            "A consulta de elementos excedeu o prazo. Verifique o diagnóstico e consulte novamente.",
            false,
        ),
        DeviceError::StaleElement => problem(
            StatusCode::CONFLICT,
            "stale_element",
            "A referência expirou ou o elemento mudou. Consulte novamente.",
            false,
        ),
        DeviceError::ActionOutcomeUnknown => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "action_outcome_unknown",
            "A ativação foi enviada, mas o resultado é incerto.",
            true,
        ),
        DeviceError::ElementActivationUnsupported => problem(
            StatusCode::FAILED_DEPENDENCY,
            "wda_required",
            "A leitura AX está disponível, mas o controle deste app requer um runner WDA assinado e ativo.",
            false,
        ),
        DeviceError::WdaNetworkTunnelRequired => problem(
            StatusCode::PRECONDITION_FAILED,
            "wda_paired_tunnel_required",
            "WDA pela rede exige Wi-Fi direto pareado e runner com binding revisado. Uma entrada Network no usbmux não comprova um canal HTTP autenticado.",
            false,
        ),
        DeviceError::WdaTunnelBindingReviewRequired => problem(
            StatusCode::PRECONDITION_FAILED,
            "wda_wifi_binding_review_required",
            "O runner Wi-Fi precisa de revisão de binding HTTP e MJPEG ao túnel. Use AX para leitura sem runner ou revise este Bundle ID antes de iniciar WDA.",
            false,
        ),
        DeviceError::WdaUnavailable => problem(
            StatusCode::FAILED_DEPENDENCY,
            "wda_unavailable",
            "O WDA não respondeu. Conferir assinatura, instalação, Developer Mode e inicialização do runner.",
            false,
        ),
        _ => problem(
            StatusCode::FAILED_DEPENDENCY,
            "element_provider_unavailable",
            "O provedor de elementos não completou a consulta. Verificar diagnóstico, WDA, conexão e estado do aparelho.",
            false,
        ),
    }
}
fn valid_selector(selector: &UiSelector) -> bool {
    [&selector.identifier, &selector.label, &selector.role]
        .iter()
        .any(|v| v.as_ref().is_some_and(|s| !s.is_empty()))
        && [&selector.identifier, &selector.label, &selector.role]
            .iter()
            .all(|v| v.as_ref().is_none_or(|s| !s.is_empty() && s.len() <= 512))
}
fn matches(value: &rust_ios_device_api::UiElement, selector: &UiSelector) -> bool {
    let test = |actual: &str, expected: &str| {
        if selector.contains {
            actual.contains(expected)
        } else {
            actual == expected
        }
    };
    selector
        .role
        .as_ref()
        .is_none_or(|role| value.role.as_ref() == Some(role))
        && selector.identifier.as_ref().is_none_or(|id| {
            value
                .identifier
                .as_ref()
                .is_some_and(|actual| test(actual, id))
        })
        && selector
            .label
            .as_ref()
            .is_none_or(|label| test(&value.label, label))
}
async fn guard(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
) -> Result<tokio::sync::OwnedSemaphorePermit, Response> {
    if !authorized(headers, state) {
        return Err(problem(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Token ou versão inválidos.",
            false,
        ));
    }
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "invalid_device_id",
            "Identificador inválido.",
            false,
        ));
    }
    if state
        .device
        .as_ref()
        .is_some_and(|allowed| allowed.as_str() != id)
    {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "device_access_denied",
            "Aparelho fora do escopo autorizado.",
            false,
        ));
    }
    tokio::time::timeout(Duration::from_secs(2), state.gate.clone().acquire_owned())
        .await
        .map_err(|_| {
            problem(
                StatusCode::LOCKED,
                "device_busy",
                "Há outra operação em andamento.",
                false,
            )
        })?
        .map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "daemon_stopping",
                "O daemon está encerrando.",
                false,
            )
        })
}
fn provider_gate(state: &AppState, provider: ElementProvider) -> Result<(), Response> {
    if provider == ElementProvider::IosAx && state.require_wda {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "element_provider_disabled",
            "Este daemon exige WDA (--require-wda); a leitura AX está desabilitada nesta configuração.",
            false,
        ));
    }
    if provider == ElementProvider::Wda
        && rust_ios_device_core::wireless::id().is_some()
        && state.runner_bundle.is_some()
        && !state.wda_tunnel_binding_reviewed
    {
        return Err(device_error(
            rust_ios_device_core::DeviceError::WdaTunnelBindingReviewRequired,
        ));
    }
    Ok(())
}
async fn session<'a>(
    sessions: &'a mut HashMap<String, CachedSession>,
    id: &str,
    port: u16,
    require_wda: bool,
    selected: ElementProvider,
) -> Result<&'a mut ElementSession, Response> {
    sessions.retain(|key, value| key == id && value.selected == selected);
    if !sessions.contains_key(id) {
        sessions.insert(
            id.to_owned(),
            CachedSession {
                selected,
                session: ElementSession::connect_with_provider(id, port, require_wda, selected)
                    .await
                    .map_err(device_error)?,
            },
        );
    }
    Ok(&mut sessions.get_mut(id).expect("session inserted").session)
}
pub async fn snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<SnapshotQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let _permit = guard(&state, &headers, &id).await?;
    provider_gate(&state, query.provider)?;
    if query.provider == ElementProvider::Wda {
        if let Some((stage, kind)) = *state.runner_failure.lock().await {
            return Err(runner_startup_problem(stage, kind));
        }
    }
    let mut sessions = state.elements.lock().await;
    let result = tokio::time::timeout(Duration::from_secs(35), async {
        session(
            &mut sessions,
            &id,
            state.wda_port,
            state.require_wda,
            query.provider,
        )
        .await?
        .snapshot()
        .await
        .map_err(device_error)
    })
    .await
    .unwrap_or_else(|_| Err(device_error(rust_ios_device_core::DeviceError::Timeout)));
    match result {
        Ok(value) => Ok(Json(serde_json::to_value(value).expect("DTO"))),
        Err(error) => {
            sessions.remove(&id);
            Err(error)
        }
    }
}
pub async fn find(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<UiFindRequest>,
) -> Result<Json<UiMatch>, Response> {
    let _permit = guard(&state, &headers, &id).await?;
    provider_gate(&state, request.provider)?;
    if request.provider == ElementProvider::Wda {
        if let Some((stage, kind)) = *state.runner_failure.lock().await {
            return Err(runner_startup_problem(stage, kind));
        }
    }
    if !valid_selector(&request.selector) || request.timeout_ms > 30000 {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "invalid_selector",
            "Informe identifier, label e/ou role não vazios; timeout máximo de 30000 ms.",
            false,
        ));
    }
    let mut sessions = state.elements.lock().await;
    let deadline = tokio::time::Instant::now()
        + if request.timeout_ms == 0 {
            Duration::from_secs(35)
        } else {
            Duration::from_millis(request.timeout_ms)
        };
    let connected = tokio::time::timeout_at(
        deadline,
        session(
            &mut sessions,
            &id,
            state.wda_port,
            state.require_wda,
            request.provider,
        ),
    )
    .await;
    let session = match connected {
        Ok(value) => value?,
        Err(_) => {
            sessions.remove(&id);
            return Err(query_timeout());
        }
    };
    loop {
        let snapshot = match tokio::time::timeout_at(deadline, session.snapshot()).await {
            Ok(result) => match result {
                Ok(value) => value,
                Err(error) => {
                    sessions.remove(&id);
                    return Err(device_error(error));
                }
            },
            Err(_) => {
                sessions.remove(&id);
                return Err(query_timeout());
            }
        };
        let found: Vec<_> = snapshot
            .elements
            .into_iter()
            .filter(|e| matches(e, &request.selector))
            .collect();
        if found.len() > 1 {
            return Err(problem(
                StatusCode::CONFLICT,
                "ambiguous_element",
                "O seletor corresponde a mais de um elemento. Refine a consulta.",
                false,
            ));
        }
        if let Some(element) = found.into_iter().next() {
            return Ok(Json(UiMatch {
                device_id: id,
                snapshot_id: snapshot.snapshot_id,
                provider: snapshot.provider,
                complete: snapshot.complete,
                warnings: snapshot.warnings,
                activation_supported: snapshot.activation_supported,
                element,
            }));
        }
        if request.timeout_ms == 0 || tokio::time::Instant::now() >= deadline {
            return Err(problem(
                StatusCode::NOT_FOUND,
                "element_not_observed",
                "O elemento não foi observado na travessia acessível dentro do prazo.",
                false,
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
pub async fn activate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<UiActivateRequest>,
) -> Result<Json<serde_json::Value>, Response> {
    let _permit = guard(&state, &headers, &id).await?;
    if request.snapshot_id.is_empty()
        || request.element_id.is_empty()
        || request.snapshot_id.len() > 128
        || request.element_id.len() > 128
    {
        return Err(problem(
            StatusCode::BAD_REQUEST,
            "invalid_reference",
            "Referência inválida.",
            false,
        ));
    }
    let mut sessions = state.elements.lock().await;
    let Some(session) = sessions.get_mut(&id) else {
        return Err(problem(
            StatusCode::CONFLICT,
            "stale_element",
            "A sessão do elemento não existe mais.",
            false,
        ));
    };
    let provider = session.provider_name();
    let result = session
        .activate(&request.snapshot_id, &request.element_id)
        .await;
    if let Err(error) = result {
        sessions.remove(&id);
        return Err(device_error(error));
    }
    Ok(Json(
        serde_json::json!({"device_id":id,"outcome":"submitted","provider":provider,"verification":"Observe a fresh snapshot to verify the intended state change."}),
    ))
}

fn query_timeout() -> Response {
    problem(
        StatusCode::REQUEST_TIMEOUT,
        "element_query_timeout",
        "A consulta não completou dentro do prazo. Faça uma nova observação.",
        false,
    )
}
