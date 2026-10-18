//! Authenticated live screen. The capture actor never holds the command gate.
use super::*;
use axum::body::Bytes;
pub async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let permit = state
        .screen_gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::LOCKED)?;
    let target = state.rsd_target().await;
    if state.has_rsd() && target.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let rsd = target.map(|target| {
        (
            target.host,
            target.port,
            target.expected_rsd_id.unwrap_or_else(|| id.clone()),
        )
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, String>>(1);
    let _worker = std::thread::Builder::new().name("tetherdeck-screen".into()).stack_size(16*1024*1024).spawn(move || {
        let _permit = permit;
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move{
            let mut screen=match rust_ios_device_core::live::Screen::connect(id,rsd).await{
                Ok(screen)=>screen,
                Err(error)=>{
                    let message=if matches!(error,rust_ios_device_core::DeviceError::WifiPairingRequired){"A identidade Wi-Fi respondeu, mas falta o registro de pareamento remoto para captura e controle. Prepare ou importe esse pareamento antes de abrir a tela."}else{"Não foi possível abrir os serviços de desenvolvimento da tela. Confira desbloqueio, Modo de Desenvolvedor e pareamento remoto; a identidade Wi-Fi sozinha não confirma captura."};
                    tracing::warn!(error=%error,"live screen connection failed");let _=tx.send(Err(message.into())).await;return;
                }
            };
            loop{
                let frame=tokio::select!{_=tx.closed()=>break,result=screen.frame()=>match result{Ok(frame)=>frame,Err(error)=>{tracing::warn!(error=%error,"live frame failed");break;}}};
                if tx.send(Ok(frame)).await.is_err(){break;}
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
    }).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    let first = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let first = match first {
        Ok(frame) => frame,
        Err(message) => {
            let problem=rust_ios_device_api::ApiProblem{code:"screen_developer_services_unavailable".into(),message,request_id:String::new(),retryable:true,execution_state:"not_started".into(),remediation:vec!["Consultar o diagnóstico e a preparação Wi-Fi. A transmissão nunca troca para USB.".into()]};
            let mut response = Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&problem).unwrap()))
                .unwrap();
            response.extensions_mut().insert(problem);
            return Ok(response);
        }
    };

    let body = async_stream::try_stream! {
        let mut frame=first;
        loop{
            let header=format!("--tetherdeck-frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",frame.len());
            let mut part=Vec::with_capacity(header.len()+frame.len()+2);
            part.extend_from_slice(header.as_bytes());part.append(&mut frame);part.extend_from_slice(b"\r\n");
            yield Bytes::from(part);
            frame=rx.recv().await.ok_or_else(||std::io::Error::other("live capture disconnected"))?.map_err(std::io::Error::other)?;
        }
    };
    let body: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<Bytes, std::io::Error>> + Send>,
    > = Box::pin(body);
    Ok(Response::builder()
        .header(
            header::CONTENT_TYPE,
            "multipart/x-mixed-replace; boundary=tetherdeck-frame",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(body))
        .unwrap())
}
