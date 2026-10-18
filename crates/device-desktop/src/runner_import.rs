//! Bounded local inspection and explicit USB installation of a pre-signed WDA IPA.
//! No signing keys are accepted, and no archive entry is extracted to host disk.
use async_zip::base::read::mem::ZipFileReader;
use cms::{content_info::ContentInfo, signed_data::SignedData};
use der::{Decode, asn1::OctetString};
use futures_lite::io::AsyncReadExt as _;
use idevice::{
    IdeviceService,
    afc::{AfcClient, opcode::AfcFopenMode},
    installation_proxy::InstallationProxyClient,
    provider::UsbmuxdProvider,
    usbmuxd::{Connection, UsbmuxdAddr},
};
use plist::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::io::AsyncReadExt as _;

const MAX_IPA: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_ENTRY: u64 = 128 * 1024 * 1024;
const MAX_METADATA: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;

#[derive(Clone)]
pub struct RunnerSummary {
    pub bundle_id: String,
    pub app_name: String,
    pub team_id: String,
    pub expires_at: String,
    pub package_bytes: u64,
    pub target_udid: String,
}

pub struct PreparedRunner {
    pub summary: RunnerSummary,
    archive: ZipFileReader,
    expires: SystemTime,
}

pub struct InstallResult {
    pub bundle_id: String,
    pub cleanup_warning: bool,
}

fn bad_archive() -> String {
    "IPA inválido, ambíguo ou fora dos limites de segurança.".into()
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .as_dictionary()
        .and_then(|d| d.get(name))
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty() && s.len() <= 255 && !s.chars().any(char::is_control))
        .ok_or_else(|| format!("Metadado obrigatório ausente ou inválido: {name}."))
}
fn normalized_udid(id: &str) -> String {
    id.replace('-', "").to_ascii_lowercase()
}
fn valid_bundle(id: &str) -> bool {
    id.len() <= 255
        && id.contains('.')
        && id.split('.').all(|s| !s.is_empty())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.starts_with('/')
        && !name.contains("//")
        && !name.contains(['\\', ':', '\0'])
        && !name.chars().any(char::is_control)
        && name
            .trim_end_matches('/')
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && s.len() <= 255)
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(bad_archive)?
            .try_into()
            .map_err(|_| bad_archive())?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(bad_archive)?
            .try_into()
            .map_err(|_| bad_archive())?,
    ))
}

// Reject multi-disk/ZIP64 before the ZIP parser allocates its entry table. WDA
// packages below 256 MiB do not need either format. Exactly one terminal EOCD.
fn central_directory(bytes: &[u8]) -> Result<(usize, usize), String> {
    if bytes.len() < 22 {
        return Err(bad_archive());
    }
    let lower = bytes.len().saturating_sub(22 + 65535);
    let end = (lower..=bytes.len() - 22)
        .rev()
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, i + 20).is_ok_and(|n| i + 22 + usize::from(n) == bytes.len())
        })
        .ok_or_else(bad_archive)?;
    let count = usize::from(u16_at(bytes, end + 10)?);
    let directory = u32_at(bytes, end + 16)? as usize;
    let size = u32_at(bytes, end + 12)? as usize;
    if count == 0
        || count > MAX_ENTRIES
        || u16_at(bytes, end + 8)? as usize != count
        || u16_at(bytes, end + 4)? != 0
        || u16_at(bytes, end + 6)? != 0
        || directory.checked_add(size) != Some(end)
        || (end >= 20 && bytes.get(end - 20..end - 16) == Some(b"PK\x06\x07"))
    {
        return Err(bad_archive());
    }
    Ok((directory, count))
}

fn profile_metadata(
    bytes: &[u8],
    bundle: &str,
    udid: &str,
) -> Result<(String, SystemTime, String), String> {
    let content = ContentInfo::from_der(bytes)
        .map_err(|_| "Perfil embarcado não é um CMS DER válido.".to_owned())?;
    if content.content_type.to_string() != "1.2.840.113549.1.7.2" {
        return Err("O perfil não contém CMS SignedData.".into());
    }
    let signed: SignedData = content
        .content
        .decode_as()
        .map_err(|_| "Estrutura SignedData do perfil inválida.".to_owned())?;
    if signed.signer_infos.0.is_empty()
        || signed.encap_content_info.econtent_type.to_string() != "1.2.840.113549.1.7.1"
    {
        return Err("O perfil não contém assinante e conteúdo CMS esperados.".into());
    }
    let data: OctetString = signed
        .encap_content_info
        .econtent
        .as_ref()
        .ok_or("Perfil sem conteúdo.")?
        .decode_as()
        .map_err(|_| "Conteúdo CMS do perfil inválido.".to_owned())?;
    if data.as_bytes().len() as u64 > MAX_METADATA {
        return Err(bad_archive());
    }
    let profile = Value::from_reader(std::io::Cursor::new(data.as_bytes()))
        .map_err(|_| "Plist do perfil inválido.".to_owned())?;
    let dict = profile.as_dictionary().ok_or("Perfil sem dicionário.")?;
    let expiry = dict
        .get("ExpirationDate")
        .and_then(Value::as_date)
        .ok_or("Perfil sem validade.")?;
    let expires = SystemTime::from(expiry);
    if expires <= SystemTime::now() {
        return Err("O perfil de desenvolvimento está expirado.".into());
    }
    if !dict
        .get("Platform")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_string() == Some("iOS")))
    {
        return Err("O perfil não autoriza a plataforma iOS.".into());
    }
    if !dict
        .get("ProvisionedDevices")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter().any(|v| {
                v.as_string()
                    .is_some_and(|s| normalized_udid(s) == normalized_udid(udid))
            })
        })
    {
        return Err("O perfil não inclui o UDID do iPhone selecionado.".into());
    }
    if !dict
        .get("DeveloperCertificates")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Err("Perfil sem certificado de desenvolvimento.".into());
    }
    let team = dict
        .get("TeamIdentifier")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 1)
        .and_then(|a| a[0].as_string())
        .filter(|s| s.len() == 10 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .ok_or("TeamIdentifier do perfil inválido.")?
        .to_owned();
    let entitlements = dict.get("Entitlements").ok_or("Perfil sem entitlements.")?;
    if entitlements
        .as_dictionary()
        .and_then(|d| d.get("get-task-allow"))
        .and_then(Value::as_boolean)
        != Some(true)
    {
        return Err("O runner exige perfil de desenvolvimento com get-task-allow=true.".into());
    }
    if field(entitlements, "com.apple.developer.team-identifier")? != team {
        return Err("Team ID inconsistente no perfil.".into());
    }
    let app_id = field(entitlements, "application-identifier")?;
    let (prefix, pattern) = app_id.split_once('.').ok_or("App ID do perfil inválido.")?;
    if !dict
        .get("ApplicationIdentifierPrefix")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_string() == Some(prefix)))
    {
        return Err("Prefixo do App ID inconsistente no perfil.".into());
    }
    let matches = pattern == bundle
        || pattern == "*"
        || pattern
            .strip_suffix(".*")
            .is_some_and(|p| bundle.starts_with(&format!("{p}.")));
    if !matches {
        return Err("O perfil não autoriza o Bundle ID deste runner.".into());
    }
    Ok((team, expires, expiry.to_xml_format()))
}

pub async fn prepare(path: PathBuf, target_udid: String) -> Result<Arc<PreparedRunner>, String> {
    rust_ios_device_client::validate_device_id(&target_udid).map_err(|e| e.to_string())?;
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("ipa"))
    {
        return Err("Escolha um arquivo .ipa local com caminho absoluto.".into());
    }
    let original = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| "Não foi possível consultar o IPA escolhido.".to_owned())?;
    if !original.is_file() || original.file_type().is_symlink() {
        return Err("Escolha um arquivo IPA regular, sem link simbólico.".into());
    }
    let path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| "Não foi possível resolver o IPA escolhido.".to_owned())?;
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("Escolha um IPA em disco local; caminhos de rede não são aceitos.".into());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "Não foi possível abrir o IPA escolhido.".to_owned())?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| "Não foi possível consultar o tamanho do IPA.".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_IPA {
        return Err("O IPA deve ter entre 1 byte e 256 MiB.".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_IPA + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Não foi possível ler o IPA escolhido.".to_owned())?;
    if bytes.len() as u64 > MAX_IPA {
        return Err(bad_archive());
    }
    let (central, count) = central_directory(&bytes)?;
    let archive = ZipFileReader::new(bytes).await.map_err(|_| bad_archive())?;
    if archive.file().entries().len() != count {
        return Err(bad_archive());
    }
    let mut names = HashSet::new();
    let mut exact_names = HashSet::new();
    let mut root = None::<String>;
    let mut expanded = 0u64;
    let mut ranges = Vec::with_capacity(count);
    for entry in archive.file().entries() {
        let name = entry.filename().as_str().map_err(|_| bad_archive())?;
        if !safe_name(name) || !names.insert(name.trim_end_matches('/').to_ascii_lowercase()) {
            return Err(bad_archive());
        }
        exact_names.insert(name.to_owned());
        if name != "Payload/" {
            let rest = name.strip_prefix("Payload/").ok_or_else(bad_archive)?;
            let app = rest
                .split('/')
                .next()
                .filter(|s| s.ends_with(".app"))
                .ok_or_else(bad_archive)?;
            let app_root = format!("Payload/{app}");
            if name == app_root {
                return Err(bad_archive());
            }
            if root.as_ref().is_some_and(|r| r != &app_root) {
                return Err("O IPA deve conter exatamente um aplicativo runner.".into());
            }
            root = Some(app_root);
        }
        let lower_name = name.to_ascii_lowercase();
        if [".p12", ".p8", ".pem", ".key"]
            .iter()
            .any(|extension| lower_name.ends_with(extension))
        {
            return Err("O IPA contém um arquivo de chave ou certificado privado; remova-o antes de importar.".into());
        }
        if name.ends_with('/') && entry.uncompressed_size() != 0 {
            return Err(bad_archive());
        }
        if let Some(mode) = entry.unix_permissions() {
            let kind = mode & 0o170000;
            if !matches!(kind, 0 | 0o040000 | 0o100000)
                || (kind == 0o040000 && !name.ends_with('/'))
                || (kind == 0o100000 && name.ends_with('/'))
            {
                return Err("IPA com links ou arquivos especiais não é aceito.".into());
            }
        }
        if entry.uncompressed_size() > MAX_ENTRY
            || !matches!(
                entry.compression(),
                async_zip::Compression::Stored | async_zip::Compression::Deflate
            )
            || (entry.uncompressed_size() > 1024 * 1024
                && entry.uncompressed_size() > entry.compressed_size().saturating_mul(200))
        {
            return Err(bad_archive());
        }
        expanded = expanded
            .checked_add(entry.uncompressed_size())
            .ok_or_else(bad_archive)?;
        if expanded > MAX_EXPANDED {
            return Err(bad_archive());
        }
        let at = usize::try_from(entry.header_offset()).map_err(|_| bad_archive())?;
        let data = archive.data();
        if at > central.saturating_sub(30) {
            return Err(bad_archive());
        }
        if data.get(at..at + 4) != Some(b"PK\x03\x04") || u16_at(data, at + 6)? & 1 != 0 {
            return Err(bad_archive());
        }
        let name_len = usize::from(u16_at(data, at + 26)?);
        let extra_len = usize::from(u16_at(data, at + 28)?);
        let start = at
            .checked_add(30 + name_len + extra_len)
            .ok_or_else(bad_archive)?;
        let end = start
            .checked_add(usize::try_from(entry.compressed_size()).map_err(|_| bad_archive())?)
            .ok_or_else(bad_archive)?;
        let method = match entry.compression() {
            async_zip::Compression::Stored => 0,
            _ => 8,
        };
        if data.get(at + 30..at + 30 + name_len) != Some(name.as_bytes())
            || u16_at(data, at + 8)? != method
            || end > central
        {
            return Err(bad_archive());
        }
        ranges.push((at, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
        return Err(bad_archive());
    }
    let root = root.ok_or_else(bad_archive)?;
    let top_info = format!("{root}/Info.plist");
    let profile_name = format!("{root}/embedded.mobileprovision");
    if !exact_names.contains(&format!("{root}/_CodeSignature/CodeResources")) {
        return Err("IPA sem recursos de assinatura embarcados.".into());
    }
    let mut relevant = BTreeMap::new();
    let mut metadata_total = 0u64;
    let mut buffer = [0u8; 65536];
    for (index, entry) in archive.file().entries().iter().enumerate() {
        let name = entry.filename().as_str().map_err(|_| bad_archive())?;
        let keep = name == top_info
            || name == profile_name
            || (name.starts_with(&format!("{root}/PlugIns/"))
                && name.ends_with(".xctest/Info.plist"));
        if keep && entry.uncompressed_size() > MAX_METADATA {
            return Err(bad_archive());
        }
        if keep {
            metadata_total += entry.uncompressed_size();
            if metadata_total > 16 * 1024 * 1024 || relevant.len() >= 18 {
                return Err(bad_archive());
            }
        }
        let mut reader = archive
            .reader_with_entry(index)
            .await
            .map_err(|_| bad_archive())?;
        let mut actual = 0u64;
        let mut metadata = Vec::new();
        loop {
            let n = reader.read(&mut buffer).await.map_err(|_| bad_archive())?;
            if n == 0 {
                break;
            }
            actual += n as u64;
            if actual > entry.uncompressed_size() || actual > MAX_ENTRY {
                return Err(bad_archive());
            }
            if keep {
                metadata.extend_from_slice(&buffer[..n]);
            }
            tokio::task::yield_now().await;
        }
        if actual != entry.uncompressed_size() || reader.compute_hash() != entry.crc32() {
            return Err("IPA com tamanho ou CRC inconsistente.".into());
        }
        if keep {
            relevant.insert(name.to_owned(), metadata);
        }
    }
    let info: Value = plist::from_bytes(
        relevant
            .get(&top_info)
            .ok_or("Info.plist do runner ausente.")?,
    )
    .map_err(|_| "Info.plist do runner inválido.")?;
    let bundle = field(&info, "CFBundleIdentifier")?.to_owned();
    if !valid_bundle(&bundle)
        || !bundle.ends_with(".xctrunner")
        || field(&info, "CFBundlePackageType")? != "APPL"
    {
        return Err(
            "Escolha um WebDriverAgent XCTest runner com Bundle ID terminado em .xctrunner.".into(),
        );
    }
    if !info
        .as_dictionary()
        .and_then(|d| d.get("CFBundleSupportedPlatforms"))
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_string() == Some("iPhoneOS")))
    {
        return Err("O IPA precisa ser compilado para iPhone físico, não simulador.".into());
    }
    let executable = field(&info, "CFBundleExecutable")?;
    if executable.contains(['/', '\\', ':'])
        || !exact_names.contains(&format!("{root}/{executable}"))
    {
        return Err("Executável do runner ausente ou inválido.".into());
    }
    let has_wda = relevant
        .iter()
        .filter(|(name, _)| name.ends_with(".xctest/Info.plist"))
        .any(|(_, data)| {
            plist::from_bytes::<Value>(data).ok().is_some_and(|p| {
                field(&p, "CFBundleExecutable").is_ok_and(|s| s == "WebDriverAgentRunner")
                    && field(&p, "CFBundlePackageType").is_ok_and(|s| s == "BNDL")
            })
        });
    if !has_wda {
        return Err(
            "O IPA não contém o plug-in de teste WebDriverAgentRunner.xctest esperado.".into(),
        );
    }
    let (team, expires, expires_at) = profile_metadata(
        relevant
            .get(&profile_name)
            .ok_or("Perfil de desenvolvimento embarcado ausente.")?,
        &bundle,
        &target_udid,
    )?;
    let app_name = field(&info, "CFBundleDisplayName")
        .or_else(|_| field(&info, "CFBundleName"))
        .unwrap_or("WebDriverAgent")
        .to_owned();
    Ok(Arc::new(PreparedRunner {
        summary: RunnerSummary {
            bundle_id: bundle,
            app_name,
            team_id: team,
            expires_at,
            package_bytes: archive.data().len() as u64,
            target_udid,
        },
        archive,
        expires,
    }))
}

pub async fn install(package: Arc<PreparedRunner>) -> Result<InstallResult, String> {
    if package.expires <= SystemTime::now() {
        return Err("O perfil expirou; prepare outro IPA.".into());
    }
    let addr = UsbmuxdAddr::default();
    let device = tokio::time::timeout(Duration::from_secs(8), async {
        let mut mux = addr
            .connect(0)
            .await
            .map_err(|_| "Serviço Apple USB indisponível.")?;
        let mut devices = mux
            .get_devices()
            .await
            .map_err(|_| "Não foi possível enumerar aparelhos USB.")?
            .into_iter()
            .filter(|d| {
                d.connection_type == Connection::Usb
                    && d.udid.eq_ignore_ascii_case(&package.summary.target_udid)
            });
        let device = devices
            .next()
            .ok_or("Conecte por USB o iPhone selecionado, desbloqueado e pareado.")?;
        if devices.next().is_some() {
            return Err("Identidade USB ambígua; atualize o diagnóstico.");
        }
        Ok(device)
    })
    .await
    .map_err(|_| "A consulta USB excedeu o prazo.")??;
    let provider = UsbmuxdProvider {
        addr,
        tag: 1,
        device_id: device.device_id,
        udid: device.udid,
        label: "TetherDeck runner installation".into(),
    };
    let remote = format!(
        "PublicStaging/tetherdeck-{}.ipa",
        uuid::Uuid::new_v4().simple()
    );
    let result = tokio::time::timeout(Duration::from_secs(180),async {
        let mut afc = AfcClient::connect(&provider).await.map_err(|e| format!("USB não pareado ou serviço AFC indisponível: {e}"))?;
        if afc.get_file_info("PublicStaging").await.is_err() { afc.mk_dir("PublicStaging").await.map_err(|e| format!("Não foi possível preparar o envio: {e}"))?; }
        let mut file = afc.open(&remote,AfcFopenMode::WrOnly).await.map_err(|e| format!("Não foi possível iniciar o envio: {e}"))?;
        let upload = async {
            for chunk in package.archive.data().chunks(1024*1024) { file.write_entire(chunk).await.map_err(|e| format!("Envio USB interrompido: {e}"))?; }
            Ok::<(),String>(())
        }.await;
        let close = file.close().await;
        upload?;
        close.map_err(|e| format!("O envio não foi confirmado: {e}"))?;
        let mut installer = InstallationProxyClient::connect(&provider).await.map_err(|e| format!("Serviço de instalação indisponível: {e}"))?;
        let options = Value::Dictionary(plist::Dictionary::from_iter([("PackageType",Value::String("Developer".into()))]));
        installer.install(&remote,Some(options)).await.map_err(|e| format!("O iOS recusou ou interrompeu a instalação: {e}. Confira assinatura, perfil e Modo Desenvolvedor."))?;
        let apps = installer.get_apps(Some("User"),Some(vec![package.summary.bundle_id.clone()])).await.map_err(|e| format!("Instalação reportada, mas consulta final falhou: {e}. Verifique o aparelho antes de repetir."))?;
        if !apps.contains_key(&package.summary.bundle_id) { return Err("O iOS não confirmou o Bundle ID instalado. Verifique o aparelho antes de repetir.".into()); }
        Ok::<(),String>(())
    }).await.unwrap_or_else(|_| Err("A instalação excedeu o prazo; o resultado pode ser incerto. Confira o aparelho antes de repetir.".into()));
    // Cleanup only our randomly named staging file, using a fresh AFC channel
    // even if upload/install was cancelled. Never remove directories or apps.
    let cleanup = tokio::time::timeout(Duration::from_secs(8), async {
        let mut afc = AfcClient::connect(&provider).await?;
        afc.remove(&remote).await
    })
    .await;
    let cleanup_warning = !matches!(cleanup, Ok(Ok(())));
    result.map_err(|error| {
        if cleanup_warning {
            format!("{error} O arquivo temporário no iPhone pode ter permanecido.")
        } else {
            error
        }
    })?;
    Ok(InstallResult {
        bundle_id: package.summary.bundle_id.clone(),
        cleanup_warning,
    })
}
