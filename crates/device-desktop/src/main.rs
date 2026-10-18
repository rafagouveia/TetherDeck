#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use gpui_kit::AppContext;
mod backend;
mod desktop;
mod icons;
mod settings;
mod i18n;
mod remote;
mod runner_import;
mod window_chrome;

struct BrandAssets;
impl gpui_kit::AssetSource for BrandAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == "brand/tetherdeck.png" {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../../assets/brand/tetherdeck-icon-256.png"
            ))));
        }
        gpui_kit::assets::AllAssets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut files = gpui_kit::assets::AllAssets.list(path)?;
        if "brand/tetherdeck.png".starts_with(path) {
            files.push("brand/tetherdeck.png".into());
        }
        Ok(files)
    }
}

fn main() {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => std::sync::Arc::new(runtime),
        Err(error) => {
            eprintln!("Não foi possível iniciar os trabalhadores: {error}");
            return;
        }
    };
    gpui_kit::application()
        .with_assets(BrandAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let options = gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(
                    gpui_kit::Bounds::centered(
                        None,
                        gpui_kit::size(gpui_kit::px(1250.), gpui_kit::px(820.)),
                        cx,
                    ),
                )),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("TetherDeck".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_min_size: Some(gpui_kit::size(gpui_kit::px(720.), gpui_kit::px(520.))),
                ..Default::default()
            };
            if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| desktop::Desktop::new(window, cx, runtime.clone()))
            }) {
                eprintln!("Não foi possível abrir o TetherDeck: {error}");
                cx.quit();
            }
        });
}
