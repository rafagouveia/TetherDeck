//! GPUI Kit device workspace. All operations delegate to the shared backend model.
use crate::{
    backend::{Config, Model, Transport},
    i18n::tr,
    icons::{self, CaptionIcon},
    remote,
    settings::{LanguagePreference, Preferences, ThemePreference},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    self, Collapsible, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    collapsible::Collapsible as Reveal,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle},
    group_box::{GroupBox, GroupBoxVariants},
    input::{Input, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarFooter, SidebarHeader, SidebarItem, SidebarMenu,
        SidebarMenuItem,
    },
    status_bar::StatusBar,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_ios_device_api::{CheckStatus, ElementProvider, IdentityStatus};
use std::{
    ops::Range,
    sync::{Arc, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) mod theme;
actions!(tetherdeck, [ToggleSidebar]);

/// Process-only presentation preference. It never changes device/session data.
pub(crate) fn presentation_active() -> bool {
    static ACTIVE: OnceLock<bool> = OnceLock::new();
    *ACTIVE.get_or_init(|| std::env::var("TETHERDECK_PRESENTATION").is_ok_and(|value| value == "1"))
}

pub(crate) fn presentation_device_name(name: &str) -> String {
    if presentation_active() {
        tr("iPhone de demonstração")
    } else {
        name.to_owned()
    }
}

pub(crate) fn presentation_transport(transport: &str) -> String {
    if !presentation_active() {
        return tr(transport);
    }
    if transport.starts_with("network:") {
        "Wi-Fi".into()
    } else if transport.starts_with("rsd:") {
        tr("Túnel RSD")
    } else {
        // Session labels are already address-free; discovery transports are not.
        tr(transport)
    }
}

fn presentation_identifier(identifier: &str) -> String {
    if presentation_active() {
        tr("Oculto na apresentação")
    } else {
        identifier.to_owned()
    }
}

/// Sanitize only rendered application notices, never model events or element content.
fn presentation_notice(source: &str, model: &Model, config: &Config) -> String {
    if !presentation_active() {
        return source.to_owned();
    }
    let mut replacements = Vec::new();
    let hidden = tr("Oculto na apresentação");
    let alias = tr("iPhone de demonstração");
    let mut add = |value: &str, replacement: &str| {
        if !value.is_empty() {
            replacements.push((value.to_owned(), replacement.to_owned()));
        }
    };
    if let Some(session) = &model.session {
        add(&session.name, &alias);
        add(&session.device_id, &hidden);
        add(&session.origin, &hidden);
    }
    if let Some(report) = &model.report {
        for device in &report.devices {
            if let Some(name) = &device.name {
                add(name, &alias);
            }
            add(&device.device_id, &hidden);
            if let Some(udid) = &device.udid {
                add(udid, &hidden);
            }
            if device.transport.starts_with("network:") || device.transport.starts_with("rsd:") {
                add(
                    &device.transport,
                    &presentation_transport(&device.transport),
                );
                if let Some((_, address)) = device.transport.split_once(':') {
                    add(address, &hidden);
                }
            }
        }
    }
    add(&config.udid, &hidden);
    add(&config.host, &hidden);
    add(&config.tunneld_origin, &hidden);
    replacements.sort_by_key(|(value, _)| std::cmp::Reverse(value.len()));
    replacements
        .into_iter()
        .fold(source.to_owned(), |text, (value, replacement)| {
            text.replace(&value, &replacement)
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Connect,
    Elements,
    Diagnostics,
    Integrations,
    Settings,
}
impl Page {
    fn title(self) -> String {
        tr(match self {
            Self::Connect => "Conectar",
            Self::Elements => "Elementos",
            Self::Diagnostics => "Diagnóstico",
            Self::Integrations => "Integrações",
            Self::Settings => "Configurações",
        })
    }
    fn icon(self) -> IconName {
        match self {
            Self::Connect => IconName::Plug,
            Self::Elements => IconName::ScanLine,
            Self::Diagnostics => IconName::Activity,
            Self::Integrations => IconName::Blocks,
            Self::Settings => IconName::Settings2,
        }
    }
}

/// Keep the native sidebar/menu semantics while allowing a 64px compact rail.
/// The outer panel owns width motion, so resizing and collapse share one tree.
#[derive(Clone)]
struct NavigationSection {
    label: &'static str,
    menu: SidebarMenu,
    openness: f32,
}
impl Collapsible for NavigationSection {
    fn is_collapsed(&self) -> bool {
        self.openness < 0.02
    }
    fn collapsed(self, _: bool) -> Self {
        self
    }
}
impl SidebarItem for NavigationSection {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8. * self.openness))
            .mb(px(24. * self.openness))
            .child(
                div()
                    .h(px(24. * self.openness))
                    .overflow_hidden()
                    .opacity(sidebar_label_opacity(self.openness))
                    .px(px(8.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::muted())
                    .whitespace_nowrap()
                    .child(tr(self.label)),
            )
            .child(
                self.menu
                    .collapsed(self.openness < 0.02)
                    .render(id, window, cx),
            )
    }
}

#[derive(Clone)]
struct SidebarNavigation {
    sections: Vec<NavigationSection>,
}
impl Collapsible for SidebarNavigation {
    fn is_collapsed(&self) -> bool {
        false
    }
    fn collapsed(self, _: bool) -> Self {
        self
    }
}
impl SidebarItem for SidebarNavigation {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id = id.into();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(
                self.sections
                    .into_iter()
                    .enumerate()
                    .map(|(index, section)| {
                        section
                            .render(SharedString::from(format!("{id}-{index}")), window, cx)
                            .into_any_element()
                    }),
            )
    }
}

pub(crate) fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .h(px(32.))
        .min_h(px(32.))
        .rounded(px(6.))
        .px(px(12.))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), theme::color(0xffffff).opacity(0.035)).inset(),
            BoxShadow::new(px(0.), px(1.), theme::color(0x000000).opacity(0.16)),
        ])
        .font_weight(FontWeight::MEDIUM)
        .flex_none()
}

pub(crate) fn primary_button(id: impl Into<ElementId>) -> Button {
    button(id)
        .primary()
        .h(px(36.))
        .min_h(px(36.))
        .border_1()
        .border_color(theme::accent().opacity(0.38))
}

fn sidebar_label_opacity(openness: f32) -> f32 {
    let progress = ((openness - 0.38) / 0.62).clamp(0., 1.);
    progress * progress * (3. - 2. * progress)
}

/// ProductType mapping verified against DeviceKit's maintained identifier table:
/// https://github.com/devicekit/DeviceKit/blob/master/Source/Device.generated.swift
/// Unknown hardware deliberately retains the identifier returned by the device.
pub(crate) fn device_model(product_type: Option<&str>) -> &str {
    match product_type {
        Some("iPhone12,1") => "iPhone 11",
        Some("iPhone12,3") => "iPhone 11 Pro",
        Some("iPhone12,5") => "iPhone 11 Pro Max",
        Some("iPhone12,8") => "iPhone SE (2ª geração)",
        Some("iPhone13,1") => "iPhone 12 mini",
        Some("iPhone13,2") => "iPhone 12",
        Some("iPhone13,3") => "iPhone 12 Pro",
        Some("iPhone13,4") => "iPhone 12 Pro Max",
        Some("iPhone14,2") => "iPhone 13 Pro",
        Some("iPhone14,3") => "iPhone 13 Pro Max",
        Some("iPhone14,4") => "iPhone 13 mini",
        Some("iPhone14,5") => "iPhone 13",
        Some("iPhone14,6") => "iPhone SE (3ª geração)",
        Some("iPhone14,7") => "iPhone 14",
        Some("iPhone14,8") => "iPhone 14 Plus",
        Some("iPhone15,2") => "iPhone 14 Pro",
        Some("iPhone15,3") => "iPhone 14 Pro Max",
        Some("iPhone15,4") => "iPhone 15",
        Some("iPhone15,5") => "iPhone 15 Plus",
        Some("iPhone16,1") => "iPhone 15 Pro",
        Some("iPhone16,2") => "iPhone 15 Pro Max",
        Some("iPhone17,1") => "iPhone 16 Pro",
        Some("iPhone17,2") => "iPhone 16 Pro Max",
        Some("iPhone17,3") => "iPhone 16",
        Some("iPhone17,4") => "iPhone 16 Plus",
        Some("iPhone17,5") => "iPhone 16e",
        Some(identifier) if !identifier.trim().is_empty() => identifier,
        _ => match crate::i18n::current_language() {
            crate::i18n::Language::Portuguese => "Modelo não informado",
            crate::i18n::Language::English => "Model unavailable",
            crate::i18n::Language::Spanish => "Modelo no disponible",
        },
    }
}

fn session_status(connected: bool) -> Div {
    let color = if connected {
        theme::success()
    } else {
        theme::muted()
    };
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(8.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(6.))
        .bg(color.opacity(0.08))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(div().size(px(6.)).rounded_full().bg(color))
        .child(tr(if connected {
            "Conectado"
        } else {
            "Sem sessão"
        }))
}

pub struct Desktop {
    preferences: Preferences,
    preferences_error: Option<String>,
    discovery_task: Option<Task<()>>,
    discovery_delay: u64,
    next_discovery: Instant,
    discovery_config: Config,
    model: Entity<Model>,
    runtime: Arc<tokio::runtime::Runtime>,
    page: Page,
    collapsed: bool,
    sidebar_resize: Entity<ResizableState>,
    sidebar_expanded_width: Pixels,
    advanced: bool,
    activity_open: bool,
    config: Config,
    udid: Entity<InputState>,
    host: Entity<InputState>,
    tunnel: Entity<InputState>,
    runner: Entity<InputState>,
    filter: Entity<InputState>,
    origin: Entity<InputState>,
    token: Entity<InputState>,
    selected_element: Option<String>,
    element_rows: Vec<usize>,
    remote_window: Option<AnyWindowHandle>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

fn input(
    window: &mut Window,
    cx: &mut Context<Desktop>,
    placeholder: &str,
    value: String,
    masked: bool,
) -> Entity<InputState> {
    let placeholder = tr(placeholder);
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(masked);
        state.set_value(value, window, cx);
        state
    })
}
fn hint_raw(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(text.into())
}
fn hint(text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div()
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(tr(text.as_ref()))
}
fn input_frame(state: &Entity<InputState>, input: Input, cx: &App) -> AnyElement {
    div()
        .id(("input-frame", state.entity_id()))
        .track_focus(&state.read(cx).focus_handle(cx))
        .w_full()
        .min_h(px(36.))
        .max_h(px(36.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::background())
        .focus(|style| style.border_color(theme::accent()))
        .child(
            input
                .bordered(false)
                .focus_bordered(false)
                .min_h(px(34.))
                .max_h(px(34.))
                .text_size(px(14.))
                .bg(theme::background()),
        )
        .into_any_element()
}
fn field(state: &Entity<InputState>, label: &str, disabled: bool, cx: &App) -> Div {
    div()
        .w_full()
        .max_w(px(560.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(hint(label.to_owned()))
        .child(input_frame(
            state,
            Input::new(state).aria_label(tr(label)).disabled(disabled),
            cx,
        ))
}
fn section(id: &'static str, icon: IconName, title: &str) -> GroupBox {
    GroupBox::new()
        .id(id)
        .outline()
        .title(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(theme::text())
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(icons::icon(icon).size(px(16.)).text_color(theme::accent()))
                .child(tr(title)),
        )
        .content_style(
            StyleRefinement::default()
                .bg(theme::surface())
                .p(px(20.))
                .gap(px(16.))
                .rounded(px(6.)),
        )
}
fn empty(icon: IconName, title: &str, description: &str) -> Empty {
    Empty::new()
        .flex_none()
        .items_start()
        .justify_start()
        .p(px(16.))
        .header(
            EmptyHeader::new()
                .items_start()
                .text_left()
                .media(
                    EmptyMedia::new()
                        .child(icons::icon(icon).size(px(24.)).text_color(theme::accent())),
                )
                .title(EmptyTitle::new().text_size(px(14.)).child(tr(title)))
                .description(
                    EmptyDescription::new()
                        .text_size(px(12.))
                        .child(tr(description)),
                ),
        )
}

impl Desktop {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        let (preferences, mut preferences_error) = Preferences::load();
        if preferences_error.is_none() {
            preferences_error = preferences.save().err();
        }
        crate::i18n::apply_language(preferences.language);
        theme::apply(preferences.theme, window.appearance(), cx);
        cx.set_reduce_motion(preferences.reduced_motion);
        cx.bind_keys([KeyBinding::new("ctrl-b", ToggleSidebar, Some("TetherDeck"))]);
        let config = Config::default();
        let model = cx.new(|cx| Model::new(runtime.clone(), cx));
        let udid = input(
            window,
            cx,
            "UDID do iPhone pareado",
            config.udid.clone(),
            presentation_active(),
        );
        let host = input(
            window,
            cx,
            "Descoberta automática",
            config.host.clone(),
            presentation_active(),
        );
        let tunnel = input(
            window,
            cx,
            "http://127.0.0.1:49151",
            config.tunneld_origin.clone(),
            presentation_active(),
        );
        let runner = input(
            window,
            cx,
            "Bundle ID do WebDriverAgent",
            config.runner_bundle.clone(),
            false,
        );
        let filter = input(
            window,
            cx,
            "Filtrar nome, identificador ou tipo",
            String::new(),
            false,
        );
        let origin = input(
            window,
            cx,
            "http://127.0.0.1:18087",
            "http://127.0.0.1:18087".into(),
            false,
        );
        let token = input(window, cx, "Token do servidor", String::new(), true);
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, model, window, cx| {
                if let Some(bundle) = model.read(cx).installed_runner_bundle.clone() {
                    if this.config.runner_bundle != bundle {
                        this.runner
                            .update(cx, |input, cx| input.set_value(bundle.clone(), window, cx));
                        this.config.runner_bundle = bundle;
                    }
                }
                if this.model.read(cx).session.is_some() {
                    this.model.update(cx, |model, _| model.cancel_discovery());
                } else {
                    this.close_remote(cx);
                }
                cx.notify();
            }),
            cx.observe(&filter, |_, _, cx| cx.notify()),
            cx.observe(&udid, |this, _, cx| this.discovery_config_changed(cx)),
            cx.observe(&host, |this, _, cx| this.discovery_config_changed(cx)),
            cx.observe(&tunnel, |this, _, cx| this.discovery_config_changed(cx)),
            cx.observe(&runner, |this, _, cx| {
                let bundle = this.runner.read(cx).value().trim().to_owned();
                if this.config.runner_tunnel_binding_reviewed_bundle != bundle {
                    this.config.runner_tunnel_binding_reviewed_bundle.clear();
                }
                this.discovery_config_changed(cx);
                cx.notify();
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let startup_token = std::env::var("RUST_IOS_DEVICE_TOKEN").unwrap_or_default();
        if !startup_token.trim().is_empty() {
            let startup_origin = std::env::var("RUST_IOS_DEVICE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:18087".into());
            model.update(cx, |model, cx| {
                model.connect_existing(startup_origin, startup_token, cx)
            });
        }
        let mut desktop = Self {
            preferences,
            preferences_error,
            discovery_task: None,
            discovery_delay: 5,
            next_discovery: Instant::now(),
            discovery_config: config.clone(),
            model,
            runtime,
            page: Page::Connect,
            collapsed: false,
            sidebar_resize: cx.new(|_| ResizableState::default()),
            sidebar_expanded_width: px(240.),
            advanced: false,
            activity_open: false,
            config,
            udid,
            host,
            tunnel,
            runner,
            filter,
            origin,
            token,
            selected_element: None,
            element_rows: Vec::new(),
            remote_window: None,
            focus,
            _subscriptions: subscriptions,
        };
        desktop
            ._subscriptions
            .push(cx.observe_window_appearance(window, |this, window, cx| {
                if this.preferences.theme == ThemePreference::System {
                    theme::apply(this.preferences.theme, window.appearance(), cx);
                    cx.notify();
                }
            }));
        desktop.start_discovery(window, cx);
        desktop._subscriptions.push(crate::window_state::observe("main", window, cx));
        desktop
            ._subscriptions
            .push(cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.model.update(cx, |model, _| model.cancel_discovery());
                }
            }));
        desktop._subscriptions.push(cx.observe_window_visibility(
            window,
            |this, visibility, _, cx| {
                if !visibility.is_visible() {
                    this.model.update(cx, |model, _| model.cancel_discovery());
                }
            },
        ));
        desktop
    }
    fn configure(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Preferences),
    ) {
        change(&mut self.preferences);
        if !self.preferences.auto_discover {
            self.model.update(cx, |model, _| model.cancel_discovery());
        }
        crate::i18n::apply_language(self.preferences.language);
        theme::apply(self.preferences.theme, window.appearance(), cx);
        cx.set_reduce_motion(self.preferences.reduced_motion);
        self.preferences_error = self.preferences.save().err();
        self.next_discovery = Instant::now();
        for (state, placeholder) in [
            (&self.udid, "UDID do iPhone pareado"),
            (&self.host, "Descoberta automática"),
            (&self.runner, "Bundle ID do WebDriverAgent"),
            (&self.filter, "Filtrar nome, identificador ou tipo"),
            (&self.token, "Token do servidor"),
        ] {
            state.update(cx, |state, cx| {
                state.set_placeholder(tr(placeholder), window, cx);
            });
        }
        cx.notify();
    }
    fn discovery_config_changed(&mut self, cx: &mut Context<Self>) {
        let config = self.draft(cx);
        // Input entities also notify on focus/caret changes. Only value changes
        // invalidate an observation or restart the discovery backoff.
        if config == self.discovery_config {
            return;
        }
        self.discovery_config = config.clone();
        self.discovery_delay = 5;
        self.next_discovery = Instant::now() + Duration::from_secs(5);
        self.model
            .update(cx, |model, _| model.discovery_config_changed(&config));
    }
    fn start_discovery(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.discovery_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let model = this.model.read(cx);
                        if !this.preferences.auto_discover
                            || this.page != Page::Connect
                            || !window.is_visible()
                            || !window.is_window_active()
                            || model.session.is_some()
                        {
                            this.model.update(cx, |model, _| model.cancel_discovery());
                            return;
                        }
                        if model.busy || Instant::now() < this.next_discovery {
                            return;
                        }
                        this.discovery_delay = if model.discovery_failed {
                            (this.discovery_delay * 2).min(30)
                        } else {
                            5
                        };
                        this.next_discovery =
                            Instant::now() + Duration::from_secs(this.discovery_delay);
                        let config = this.draft(cx);
                        this.model
                            .update(cx, |model, cx| model.search_automatically(config, cx));
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
    fn caption(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let maximize_icon = if window.is_maximized() {
            CaptionIcon::Restore
        } else {
            CaptionIcon::Maximize
        };
        let maximize_label = tr(if window.is_maximized() {
            "Restaurar tamanho da janela"
        } else {
            "Maximizar janela"
        });
        div()
            .h(px(36.))
            .min_h(px(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("main-window-drag")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .window_control_area(WindowControlArea::Drag)
                    .child(img("brand/tetherdeck.png").size(px(16.)))
                    .child(div().text_size(px(12.)).child("TetherDeck")),
            )
            .child(
                icons::caption_button(
                    "main-minimize",
                    CaptionIcon::Minimize,
                    px(46.),
                    px(36.),
                    tr("Minimizar janela"),
                )
                .on_click(cx.listener(|_, _, window, _| {
                    if let Err(error) = crate::window_chrome::minimize(window) {
                        eprintln!("window minimize: {error}");
                    }
                })),
            )
            .child(
                icons::caption_button(
                    "main-maximize",
                    maximize_icon,
                    px(46.),
                    px(36.),
                    maximize_label,
                )
                .on_click(cx.listener(|_, _, window, _| {
                    if let Err(error) = crate::window_chrome::maximize_restore(window) {
                        eprintln!("window maximize: {error}");
                    }
                })),
            )
            .child(
                icons::caption_button(
                    "main-close",
                    CaptionIcon::Close,
                    px(46.),
                    px(36.),
                    tr("Fechar"),
                )
                .on_click(cx.listener(|_, _, window, _| {
                    if let Err(error) = crate::window_chrome::close(window) {
                        eprintln!("window close: {error}");
                    }
                })),
            )
    }
    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut themes = div().flex().flex_wrap().gap(px(8.));
        for (theme, label) in [
            (ThemePreference::System, "Sistema"),
            (ThemePreference::Dark, "Escuro"),
            (ThemePreference::Light, "Claro"),
        ] {
            themes = themes.child(
                button(format!("theme-{label}"))
                    .label(tr(label))
                    .selected(self.preferences.theme == theme)
                    .when(self.preferences.theme == theme, |button| {
                        button
                            .bg(theme::accent().opacity(0.15))
                            .border_1()
                            .border_color(theme::accent().opacity(0.45))
                            .text_color(theme::accent())
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.configure(window, cx, |preferences| preferences.theme = theme)
                    })),
            );
        }
        let mut languages = div().flex().flex_wrap().gap(px(8.));
        for (language, label) in [
            (LanguagePreference::System, "Sistema"),
            (LanguagePreference::Portuguese, "Português"),
            (LanguagePreference::English, "English"),
            (LanguagePreference::Spanish, "Español"),
        ] {
            languages = languages.child(
                button(format!("language-{label}"))
                    .label(tr(label))
                    .selected(self.preferences.language == language)
                    .when(self.preferences.language == language, |button| {
                        button
                            .bg(theme::accent().opacity(0.15))
                            .border_1()
                            .border_color(theme::accent().opacity(0.45))
                            .text_color(theme::accent())
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.configure(window, cx, |preferences| preferences.language = language)
                    })),
            );
        }
        let mut page = div().flex().flex_col().gap(px(16.))
            .child(self.page_title("Personalize o aplicativo.","Tema, idioma e movimento são aplicados imediatamente e salvos neste computador."))
            .child(section("settings-appearance",IconName::Palette,"Aparência")
                .child(hint("Tema")).child(themes).child(hint("Idioma")).child(languages)
                .child(Checkbox::new("reduce-motion").label(tr("Reduzir animações")).checked(self.preferences.reduced_motion)
                    .on_click(cx.listener(|this, checked, window, cx| this.configure(window,cx,|preferences|preferences.reduced_motion=*checked))))
                .child(hint("Mantém transições discretas sem animações contínuas.")))
            .child(section("settings-discovery",IconName::Search,"Descoberta de aparelhos")
                .child(Checkbox::new("auto-discovery").label(tr("Buscar automaticamente")).checked(self.preferences.auto_discover)
                    .on_click(cx.listener(|this,checked,window,cx| this.configure(window,cx,|preferences|preferences.auto_discover=*checked))))
                .child(hint("Busca somente enquanto Conectar está visível e sem sessão. Operações em andamento pausam novas buscas; falhas aumentam o intervalo.")));
        page = if let Some(error) = &self.preferences_error {
            page.child(
                hint(format!(
                    "{}: {}",
                    tr("Não foi possível salvar as preferências"),
                    tr(error)
                ))
                .text_color(theme::error()),
            )
        } else {
            page.child(hint("Preferências salvas neste computador."))
        };
        page.into_any_element()
    }
    fn draft(&self, cx: &App) -> Config {
        Config {
            transport: self.config.transport,
            element_provider: self.config.element_provider,
            udid: self.udid.read(cx).value().trim().to_owned(),
            host: self.host.read(cx).value().trim().to_owned(),
            tunneld_origin: self.tunnel.read(cx).value().trim().to_owned(),
            runner_bundle: self.runner.read(cx).value().trim().to_owned(),
            runner_tunnel_binding_reviewed_bundle: self
                .config
                .runner_tunnel_binding_reviewed_bundle
                .clone(),
            allow_usb_recovery: self.config.allow_usb_recovery,
            allow_service_recovery: self.config.allow_service_recovery,
        }
    }
    fn close_remote(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.remote_window.take() {
            if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
                eprintln!("Janela remota já encerrada: {error}");
            }
        }
    }
    fn open_remote(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.remote_window {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.remote_window = None;
        }
        let session = self.model.read(cx).session.clone();
        if let Some(session) = session {
            match remote::open(session, self.runtime.clone(), cx) {
                Ok(handle) => self.remote_window = Some(handle),
                Err(error) => self.model.update(cx, |model, cx| {
                    model.error = Some(format!("Não foi possível abrir a tela: {error}"));
                    cx.notify();
                }),
            }
        }
    }
    fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        if page != Page::Connect {
            self.model.update(cx, |model, _| model.cancel_discovery());
        }
        cx.notify();
    }
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }
    fn sidebar(&self, openness: f32, cx: &mut Context<Self>) -> AnyElement {
        let menu = |pages: &[Page], cx: &mut Context<Self>| {
            SidebarMenu::new()
                .gap(px(4.))
                .children(pages.iter().copied().map(|page| {
                    SidebarMenuItem::new(page.title())
                        .icon(icons::icon(page.icon()).size(px(18.)))
                        .min_h(px(44.))
                        .max_h(px(44.))
                        .label_style(
                            StyleRefinement::default()
                                .text_size(px(14.))
                                .opacity(sidebar_label_opacity(openness)),
                        )
                        .active(self.page == page)
                        .suffix({
                            let active = self.page == page;
                            move |_, _| {
                                div()
                                    .w(px(3.))
                                    .h(px(14.))
                                    .rounded_full()
                                    .bg(theme::accent())
                                    .opacity(if active { openness } else { 0. })
                            }
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)))
                }))
        };
        let workspace = menu(&[Page::Connect, Page::Elements], cx);
        let tools = menu(&[Page::Diagnostics, Page::Integrations, Page::Settings], cx);
        let name = self
            .model
            .read(cx)
            .session
            .as_ref()
            .map(|session| presentation_device_name(&session.name));
        Sidebar::new("device-sidebar")
            .w_full()
            .collapsible(SidebarCollapsible::None)
            .header(
                SidebarHeader::new()
                    .h(px(52.))
                    .min_h(px(52.))
                    .justify_start()
                    .gap(px(12.))
                    .overflow_hidden()
                    .child(img("brand/tetherdeck.png").size(px(28.)).flex_shrink_0())
                    .child(
                        div()
                            .opacity(sidebar_label_opacity(openness))
                            .whitespace_nowrap()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .child("TetherDeck"),
                    ),
            )
            .child(SidebarNavigation {
                sections: vec![
                    NavigationSection {
                        label: "WORKSPACE",
                        menu: workspace,
                        openness,
                    },
                    NavigationSection {
                        label: "FERRAMENTAS",
                        menu: tools,
                        openness,
                    },
                ],
            })
            .when(openness > 0.01, |sidebar| {
                sidebar.footer(
                    SidebarFooter::new()
                        .flex_col()
                        .items_start()
                        .overflow_hidden()
                        .h(px(44. * openness))
                        .min_h(px(0.))
                        .opacity(sidebar_label_opacity(openness))
                        .child(hint("SESSÃO"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .child(name.unwrap_or_else(|| tr("Nenhum aparelho ativo"))),
                        ),
                )
            })
            .into_any_element()
    }
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let model = self.model.read(cx);
        let connected = model.session.is_some();
        let busy = model.busy;
        div()
            .h(px(52.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(16.))
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::background())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        button("toggle-sidebar")
                            .ghost()
                            .shadow_none()
                            .w(px(40.))
                            .h(px(36.))
                            .min_h(px(36.))
                            .px(px(0.))
                            .child(icons::panel_left())
                            .tooltip(tr(if self.collapsed {
                                "Expandir barra lateral · Ctrl+B"
                            } else {
                                "Recolher barra lateral · Ctrl+B"
                            }))
                            .accessibility_label(tr(if self.collapsed {
                                "Expandir barra lateral · Ctrl+B"
                            } else {
                                "Recolher barra lateral · Ctrl+B"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar(cx);
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.page.title()),
                    )
                    .child(session_status(connected)),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(12.))
                    .when(connected, |this| {
                        this.child(
                            button("open-live-global")
                                .h(px(36.))
                                .min_h(px(36.))
                                .icon(icons::icon(IconName::Monitor))
                                .label(tr("Abrir tela"))
                                .disabled(!connected || busy)
                                .on_click(cx.listener(|this, _, _, cx| this.open_remote(cx))),
                        )
                    }),
            )
    }
    fn page_title(&self, title: &str, description: &str) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .mb(px(24.))
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr(title)),
            )
            .child(hint(description.to_owned()))
    }
    fn render_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let busy = model.busy;
        if let Some(session) = model.session.clone() {
            return div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(self.page_title(
                    "Conexão ativa.",
                    "Acesse a tela remota e as ferramentas deste aparelho.",
                ))
                .child(
                    section("active-session", IconName::Smartphone, "Aparelho conectado")
                        .child(div().flex().items_center().gap(px(16.))
                            .child(div().size(px(56.)).flex_shrink_0().rounded(px(12.))
                                .bg(theme::accent().opacity(0.10)).border_1().border_color(theme::accent().opacity(0.14))
                                .flex().items_center().justify_center()
                                .child(icons::icon(IconName::Smartphone).size(px(28.)).text_color(theme::accent())))
                            .child(div().flex().flex_col().flex_1().min_w_0().gap(px(4.))
                                .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD)
                                    .truncate().child(presentation_device_name(&session.name)))
                                .child(div().text_size(px(14.)).text_color(theme::muted()).truncate()
                                    .child(device_model(session.product_type.as_deref()).to_owned()))))
                        .child(div().flex().flex_wrap().items_center().justify_between().gap(px(12.))
                            .child(div().flex().items_center().gap(px(8.))
                                .child(icons::icon(match session.transport { Transport::Usb => IconName::Usb, Transport::Wifi => IconName::Wifi, Transport::Rsd => IconName::Network }).size(px(16.)).text_color(theme::muted()))
                                .child(hint(format!("{} · {}", presentation_transport(session.transport_label()),tr("Identidade verificada")))))
                            .child(button("disconnect").ghost().icon(icons::icon(IconName::Unplug)).label(tr("Desconectar"))
                                .disabled(busy).on_click(cx.listener(|this, _, _, cx| {
                                    this.close_remote(cx);
                                    this.model.update(cx, |model, cx| model.disconnect(cx));
                                }))))
                )
                .child(section("active-tools", IconName::Layers, "Ferramentas da sessão")
                    .child(hint("Inspecione a interface ou consulte a saúde da conexão. A tela permanece em sua própria janela."))
                    .child(div().flex().flex_wrap().gap(px(8.))
                        .child(button("go-elements").icon(icons::icon(IconName::ScanLine)).label(tr("Inspecionar elementos"))
                            .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Elements, cx))))
                        .child(button("go-diagnostics").icon(icons::icon(IconName::Activity)).label(tr("Diagnóstico da conexão"))
                            .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Diagnostics, cx))))))
                .into_any_element();
        }
        let mode = self.config.transport;
        let mut mode_tabs = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .p(px(4.))
            .bg(theme::surface())
            .rounded(px(8.));
        for (transport, icon, label) in [
            (Transport::Usb, IconName::Usb, "USB"),
            (Transport::Wifi, IconName::Wifi, "Wi-Fi"),
            (Transport::Rsd, IconName::Network, "Túnel RSD"),
        ] {
            mode_tabs = mode_tabs.child(
                button(format!("transport-{label}"))
                    .ghost()
                    .icon(icons::icon(icon))
                    .label(tr(label))
                    .selected(mode == transport)
                    .when(mode == transport, |button| {
                        button
                            .bg(theme::accent().opacity(0.12))
                            .border_1()
                            .border_color(theme::accent().opacity(0.35))
                            .text_color(theme::accent())
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config.transport = transport;
                        this.discovery_config_changed(cx);
                        this.model.update(cx, |model, cx| {
                            model.report = None;
                            cx.notify();
                        });
                        cx.notify();
                    })),
            );
        }
        let mut transport=section("transport-config",if mode==Transport::Usb{IconName::Usb}else{IconName::Wifi},"Conexão")
            .child(hint(match mode{Transport::Usb=>"Conecte o cabo de dados e desbloqueie o iPhone. Confirme a confiança no aparelho se solicitada.",Transport::Wifi=>"Mesma rede do computador, iPhone desbloqueado e previamente pareado. O cabo não mantém esta conexão.",Transport::Rsd=>"Use o serviço de túneis já ativo neste computador e o UDID do aparelho."}));
        if mode != Transport::Usb {
            transport = transport.child(field(&self.udid, "UDID do aparelho", busy, cx));
        }
        if mode == Transport::Wifi {
            transport = transport.child(field(&self.host, "IP do iPhone (opcional)", busy, cx));
        }
        if mode == Transport::Rsd {
            transport = transport.child(field(&self.tunnel, "Serviço de túneis local", busy, cx));
        }
        transport = transport.child(
            primary_button("search-devices")
                .icon(icons::icon(IconName::Search))
                .label(tr("Buscar aparelho"))
                .disabled(busy)
                .loading(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.next_discovery = Instant::now() + Duration::from_secs(5);
                    let config = this.draft(cx);
                    this.model.update(cx, |model, cx| model.search(config, cx));
                })),
        );
        transport = transport.child(
            Checkbox::new("connect-auto-discovery")
                .label(tr("Buscar automaticamente"))
                .checked(self.preferences.auto_discover)
                .on_click(cx.listener(|this, checked, window, cx| {
                    this.configure(window, cx, |preferences| {
                        preferences.auto_discover = *checked
                    })
                })),
        );
        let options = self.session_options(cx);
        let wide = window.bounds().size.width > px(1020.);
        let top = div()
            .flex()
            .gap(px(16.))
            .when(!wide, |this| this.flex_col())
            .child(div().flex_1().min_w_0().child(transport))
            .child(div().flex_1().min_w_0().child(options));
        let devices = self.connection_devices(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(self.page_title(
                "Conecte seu iPhone.",
                "Escolha o transporte, encontre o aparelho e abra uma sessão local.",
            ))
            .child(mode_tabs)
            .child(top)
            .child(devices)
            .into_any_element()
    }
    fn choose_runner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target_udid = self.udid.read(cx).value().trim().to_owned();
        if target_udid.is_empty() {
            self.model.update(cx, |model, cx| {
                model.error = Some("Informe o UDID do iPhone antes de importar o runner.".into());
                cx.notify();
            });
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Selecionar runner iOS assinado (.ipa)").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = match selection.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = this.update_in(cx, |this, _, cx| match result {
                Ok(Some(paths)) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.config.runner_tunnel_binding_reviewed_bundle.clear();
                        this.model
                            .update(cx, |model, cx| model.prepare_runner(path, target_udid, cx));
                    }
                }
                Ok(None) => {}
                Err(error) => this.model.update(cx, |model, cx| {
                    model.error = Some(format!("Não foi possível abrir o arquivo: {error}"));
                    cx.notify();
                }),
            }) {
                eprintln!("runner file selection view closed: {error}");
            }
        })
        .detach();
    }
    fn runner_import(&self, cx: &mut Context<Self>) -> Div {
        let model = self.model.read(cx);
        let busy = model.busy;
        let package = model.runner_package.clone();
        let installed = model.installed_runner_bundle.is_some();
        let mut panel = div().flex().flex_col().gap(px(12.)).p(px(12.))
            .bg(theme::background()).rounded(px(6.)).border_1().border_color(theme::border())
            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(tr("Runner de automação")))
            .child(hint("Importe um .ipa já assinado para este iPhone. A instalação usa o USB e depende de o iOS aceitar a assinatura."))
            .child(button("choose-runner-ipa").icon(icons::icon(IconName::Upload)).label(tr("Selecionar .ipa assinado"))
                .disabled(busy).on_click(cx.listener(|this, _, window, cx| this.choose_runner(window, cx))));
        if let Some(package) = package {
            let summary = &package.summary;
            panel = panel
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(summary.app_name.clone()),
                        )
                        .child(hint(summary.bundle_id.clone()).truncate())
                        .child(hint(format!(
                            "{} {} · {:.1} MiB",
                            tr("Equipe"),
                            summary.team_id,
                            summary.package_bytes as f64 / 1_048_576.
                        )))
                        .child(hint(format!(
                            "{} {}",
                            tr("Perfil: expira em"),
                            summary.expires_at
                        )))
                        .child(
                            hint(format!(
                                "{}: {}",
                                tr("iPhone autorizado"),
                                presentation_identifier(&summary.target_udid)
                            ))
                            .truncate(),
                        ),
                )
                .child(
                    button("install-runner-ipa")
                        .tooltip(tr(
                            "Metadados conferidos; o iOS confirma a assinatura na instalação.",
                        ))
                        .icon(icons::icon(IconName::Download))
                        .label(tr("Instalar no iPhone por USB"))
                        .disabled(busy)
                        .loading(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.config.runner_tunnel_binding_reviewed_bundle.clear();
                            this.model.update(cx, |model, cx| model.install_runner(cx))
                        })),
                );
        }
        if installed {
            panel = panel.child(
                hint("Runner instalado. Reconecte a sessão para ativar a automação.")
                    .text_color(theme::success()),
            );
        }
        panel
    }
    fn session_options(&self, cx: &mut Context<Self>) -> GroupBox {
        let busy = self.model.read(cx).busy;
        let editable = !busy && self.model.read(cx).session.is_none();
        let runner_bundle = self.runner.read(cx).value().trim().to_owned();
        let reviewed = !runner_bundle.is_empty()
            && self.config.runner_tunnel_binding_reviewed_bundle == runner_bundle;
        section("session-options",IconName::Settings2,"Opções da sessão")
            .child(hint("Use AX para inspecionar sem runner ou WDA assinado para ativar elementos. Vídeo e toque têm uma conexão independente."))
            .child(button("session-advanced").icon(icons::icon(IconName::ChevronDown)).label(tr(if self.advanced{"Ocultar opções avançadas"}else{"Configurar automação e recuperação"}))
                .on_click(cx.listener(|this,_,_,cx|{this.advanced=!this.advanced;cx.notify();})))
            .child(Reveal::new().motion_id("session-details").open(self.advanced).content(div().flex().flex_col().gap(px(12.))
                .child(self.runner_import(cx))
                .child(field(&self.runner, "Bundle ID do WebDriverAgent", !editable, cx))
                .child(Checkbox::new("runner-wifi-reviewed").label(tr("Runner preparado para Wi-Fi isolado"))
                    .checked(reviewed).disabled(!editable || runner_bundle.is_empty())
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.config.runner_tunnel_binding_reviewed_bundle = if *checked { this.runner.read(cx).value().trim().to_owned() } else { String::new() };
                        cx.notify();
                    })))
                .child(hint("Confirme somente um runner revisado que respeite USE_IP nos servidores HTTP e MJPEG. Nome do IPA não comprova isso; WDA 16.15.3+ é uma referência conhecida."))
                .child(Checkbox::new("allow-usb").label(tr("Permitir recuperação USB")).checked(self.config.allow_usb_recovery).disabled(!editable)
                    .on_click(cx.listener(|this,checked,_,cx|{this.config.allow_usb_recovery=*checked;cx.notify();})))
                .child(Checkbox::new("allow-service").label(tr("Permitir reinício do serviço Apple")).checked(self.config.allow_service_recovery).disabled(!editable)
                    .on_click(cx.listener(|this,checked,_,cx|{this.config.allow_service_recovery=*checked;cx.notify();})))
                .child(hint("Recuperações exigem privilégios do sistema e são solicitadas manualmente. Não repetem ações no iPhone."))
                .when(self.config.transport==Transport::Wifi,|this|this.child(button("enable-wifi").label(tr("Preparar Wi-Fi pelo USB pareado")).disabled(!editable)
                    .on_click(cx.listener(|this,_,_,cx|{let udid=this.udid.read(cx).value().trim().to_owned();this.model.update(cx,|model,cx|model.enable_wifi(udid,cx));}))))))
    }
    fn connection_devices(&self, cx: &mut Context<Self>) -> GroupBox {
        let model = self.model.read(cx);
        let busy = model.busy;
        let devices = model
            .report
            .as_ref()
            .map(|report| report.devices.clone())
            .unwrap_or_default();
        let devices: Vec<_> = devices
            .into_iter()
            .filter(|device| match self.config.transport {
                Transport::Usb => device.transport == "usb",
                Transport::Wifi => device.transport.starts_with("network:"),
                Transport::Rsd => device.transport.starts_with("rsd:"),
            })
            .collect();
        let mut content = section("found-devices", IconName::Smartphone, "Aparelhos");
        if devices.is_empty() {
            content=content.child(empty(IconName::Smartphone,if busy{"Buscando aparelho…"}else{"Nenhum aparelho disponível"},"Desbloqueie o iPhone e faça uma busca no transporte selecionado. O diagnóstico detalha cada camada da conexão."));
        }
        for device in devices {
            let ready = device.identity_status == IdentityStatus::Available;
            content = content.child(
                div()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(presentation_device_name(
                                        device.name.as_deref().unwrap_or("iPhone"),
                                    )),
                            )
                            .child(
                                hint(format!(
                                    "{} · {} · {}",
                                    device_model(device.product_type.as_deref()),
                                    presentation_transport(&device.transport),
                                    if ready {
                                        "Identidade verificada"
                                    } else {
                                        "Verifique o diagnóstico"
                                    }
                                ))
                                .truncate(),
                            ),
                    )
                    .child(
                        button(format!("connect-{}", device.device_id))
                            .w(px(136.))
                            .flex_shrink_0()
                            .label(tr("Abrir sessão"))
                            .disabled(busy || !ready)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let config = this.draft(cx);
                                let device = device.clone();
                                this.model
                                    .update(cx, |model, cx| model.connect(device, config, cx));
                            })),
                    ),
            );
        }
        content
    }
    fn render_elements(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let busy = model.busy;
        let connected = model.session.is_some();
        let snapshot = model.elements.clone();
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .mb_4()
            .child(
                primary_button("inspect-elements")
                    .icon(icons::icon(IconName::ScanLine))
                    .label(tr("Consultar elementos"))
                    .disabled(busy || !connected)
                    .loading(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.update(cx, |model, cx| model.inspect(cx))
                    })),
            )
            .child(
                div().flex_1().min_w_0().child(input_frame(
                    &self.filter,
                    Input::new(&self.filter)
                        .prefix(icons::icon(IconName::Search))
                        .aria_label(tr("Filtrar elementos")),
                    cx,
                )),
            );
        let mut providers = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .mb(px(12.))
            .child(hint("Provedor de elementos"));
        for (provider, label) in [
            (ElementProvider::Auto, "Automático"),
            (ElementProvider::Wda, "WDA"),
            (ElementProvider::IosAx, "iOS AX"),
        ] {
            providers = providers.child(
                button(format!("provider-{}", provider.as_str()))
                    .label(tr(label))
                    .selected(model.element_provider == provider)
                    .when(model.element_provider == provider, |button| {
                        button
                            .bg(theme::accent().opacity(0.15))
                            .text_color(theme::accent())
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.config.element_provider = provider;
                        this.discovery_config_changed(cx);
                        this.model
                            .update(cx, |model, cx| model.set_element_provider(provider, cx));
                    })),
            );
        }
        let mut page=div().flex().flex_col().size_full().min_h_0()
            .child(self.page_title("Elementos acessíveis.","Encontre controles por nome, identificador ou tipo. O servidor revalida a referência antes de ativar."))
            .child(providers)
            .child(hint("AX permite inspeção e seleção de referências. WDA permite ações por elemento com runner disponível.").mb(px(12.)))
            .child(toolbar);
        let Some(snapshot) = snapshot else {
            return page.child(empty(IconName::ScanLine,"Inspecione a tela atual","Abra uma sessão e consulte os elementos. AX pode oferecer somente leitura; a interface não simula uma ativação.")).into_any_element();
        };
        let filter = self.filter.read(cx).value().to_lowercase();
        self.element_rows = snapshot
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| {
                let matches = filter.is_empty()
                    || element.label.to_lowercase().contains(&filter)
                    || element
                        .identifier
                        .as_ref()
                        .is_some_and(|id| id.to_lowercase().contains(&filter))
                    || element
                        .role
                        .as_ref()
                        .is_some_and(|role| role.to_lowercase().contains(&filter));
                matches.then_some(index)
            })
            .collect();
        let count = self.element_rows.len();
        page = page.child(
            hint(format!(
                "{} · {} {} {} {} · {}",
                snapshot.provider,
                count,
                tr("de"),
                snapshot.elements.len(),
                tr("elementos"),
                tr(if snapshot.complete {
                    "consulta concluída"
                } else {
                    "consulta parcial"
                })
            ))
            .mb_3(),
        );
        for warning in &snapshot.warnings {
            page = page.child(
                hint_raw(presentation_notice(warning, model, &self.config))
                    .text_color(theme::warning())
                    .mb_2(),
            );
        }
        page = page.child(
            div()
                .id("elements-list-region")
                .role(Role::ListBox)
                .aria_label(tr("Elementos"))
                .flex_1()
                .min_h(px(160.))
                .border_1()
                .border_color(theme::border())
                .rounded(px(6.))
                .overflow_hidden()
                .child(
                    uniform_list(
                        "element-list",
                        count,
                        cx.processor(|this, range: Range<usize>, _, cx| {
                            range
                                .filter_map(|row| this.element_row(row, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full(),
                ),
        );
        if let Some(selected) = self
            .selected_element
            .as_ref()
            .and_then(|id| {
                snapshot
                    .elements
                    .iter()
                    .find(|element| &element.element_id == id)
            })
            .cloned()
        {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let fresh = now.saturating_sub(snapshot.created_at_unix_ms) < 30_000;
            let activatable = snapshot.activation_supported
                && selected.actions.iter().any(|action| action == "activate")
                && selected.enabled != Some(false)
                && selected.visible != Some(false)
                && fresh
                && !busy;
            let activate_id = selected.element_id.clone();
            let copy = selected.clone();
            page=page.child(div().mt_4().flex().flex_col().gap(px(8.))
                .child(div().font_weight(FontWeight::MEDIUM).truncate().child(if selected.label.is_empty(){selected.identifier.clone().unwrap_or_else(||tr("Elemento sem nome"))}else{selected.label.clone()}))
                .child(hint(format!("{} · {}: {:?} · {}: {:?}",selected.role.clone().unwrap_or_else(||tr("Tipo desconhecido")),tr("visível"),selected.visible,tr("habilitado"),selected.enabled)))
                .child(div().flex().gap(px(8.))
                    .child(button("copy-selector").icon(icons::icon(IconName::Copy)).label(tr("Copiar seletor")).on_click(cx.listener(move|this,_,_,cx|{
                        let selector=serde_json::json!({"identifier":copy.identifier,"label":copy.label,"role":copy.role});
                        match serde_json::to_string_pretty(&selector){
                            Ok(text)=>cx.write_to_clipboard(ClipboardItem::new_string(text)),
                            Err(error)=>this.model.update(cx,|model,cx|{model.error=Some(format!("Não foi possível copiar o seletor: {error}"));cx.notify();}),
                        }
                    })))
                    .child(button("activate-element").secondary().h(px(36.)).min_h(px(36.)).label(tr("Ativar elemento")).disabled(!activatable)
                        .on_click(cx.listener(move|this,_,_,cx|{let id=activate_id.clone();this.model.update(cx,|model,cx|model.activate(id,cx));})))));
        }
        page.into_any_element()
    }
    fn element_row(&self, row: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.model.read(cx).elements.clone()?;
        let index = *self.element_rows.get(row)?;
        let element = snapshot.elements.get(index)?;
        let id = element.element_id.clone();
        let label = if element.label.is_empty() {
            element
                .identifier
                .clone()
                .unwrap_or_else(|| tr("Sem nome acessível"))
        } else {
            element.label.clone()
        };
        let secondary = element.identifier.clone().unwrap_or_else(|| {
            element
                .role
                .clone()
                .unwrap_or_else(|| tr("Sem identificador"))
        });
        let selected = self.selected_element.as_ref() == Some(&id);
        let accessible_name = format!("{label} · {secondary}");
        Some(
            component::list::ListItem::new(format!("element-{id}"))
                // Virtual rows are laid out as roots: claim the whole available
                // width so the pointer target includes the row's empty area.
                .w_full()
                .role(Role::ListBoxOption)
                .accessibility_label(accessible_name)
                .aria_selected(selected)
                .cursor_pointer()
                .selected(selected)
                .h(px(64.))
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(div().text_size(px(13.)).truncate().child(label))
                        .child(hint_raw(secondary).truncate()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_element = Some(id.clone());
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
    fn render_diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let busy = model.busy;
        let report = model.report.clone();
        let runner = model
            .session
            .as_ref()
            .is_some_and(|session| session.runner_configured);
        let service_allowed = model
            .session
            .as_ref()
            .is_none_or(|session| session.allow_service_recovery);
        let mut page=div().flex().flex_col().gap(px(16.))
            .child(self.page_title("Saúde da conexão.","Compare hardware, serviço, identidade e automação antes de escolher uma recuperação."))
            .child(div().flex().flex_wrap().items_center().gap(px(8.))
                .child(primary_button("refresh-diagnostics").icon(icons::icon(IconName::RefreshCw)).label(tr("Atualizar diagnóstico")).disabled(busy).loading(busy)
                    .on_click(cx.listener(|this,_,_,cx|this.model.update(cx,|model,cx|model.diagnose(cx)))))
                .child(button("recover-service").label(tr("Reiniciar serviço Apple")).disabled(busy||!service_allowed)
                    .on_click(cx.listener(|this,_,_,cx|this.model.update(cx,|model,cx|model.recover_service(cx)))))
                .child(button("recover-runner").label(tr("Reiniciar automação")).disabled(busy||!runner)
                    .on_click(cx.listener(|this,_,_,cx|this.model.update(cx,|model,cx|model.recover_runner(cx))))));
        let Some(report) = report else {
            return page.child(empty(IconName::Activity,"Diagnóstico ainda não disponível","Atualize o relatório para consultar o estado real deste computador e do aparelho.")).into_any_element();
        };
        page = page.child(
            section("diagnostic-summary", IconName::Activity, "Resumo")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(match report.health.as_str() {
                                    "ready" => theme::success(),
                                    "degraded" => theme::warning(),
                                    _ => theme::error(),
                                })
                                .child(tr(match report.health.as_str() {
                                    "ready" => "Conexão pronta",
                                    "degraded" => "Requer atenção",
                                    _ => "Verificar conexão",
                                })),
                        )
                        .child(hint(format!(
                            "{} · {} · {} ms",
                            report.platform, report.architecture, report.elapsed_ms
                        ))),
                )
                .child(hint(format!(
                    "{} · {} · PID {}",
                    report.service.name,
                    report.service.state,
                    report
                        .service
                        .process_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "—".into())
                ))),
        );
        let mut checks = section("diagnostic-checks", IconName::Layers, "Camadas verificadas");
        for check in report.checks {
            let color = match check.status {
                CheckStatus::Pass => theme::success(),
                CheckStatus::Warn => theme::warning(),
                CheckStatus::Fail => theme::error(),
                _ => theme::muted(),
            };
            let mut check_row = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .pb_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .child(check.layer),
                        )
                        .child(hint(check.code)),
                )
                .child(div().text_size(px(13.)).child(check.message));
            for remediation in check.remediation {
                check_row = check_row.child(hint(remediation));
            }
            checks = checks.child(check_row);
        }
        page = page.child(checks);
        if let Some(history) = report.transport_history {
            let mut transport_history = section("transport-history", IconName::Activity, "Histórico do transporte")
                .child(hint(format!("{} {} · {} {}", history.samples,tr("consultas"),history.observations.len(),tr("registros recentes"))))
                .child(hint("Instantes observados por consulta; não comprovam o momento físico de uma desconexão nem sua causa."));
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            for observation in history.observations.into_iter().rev() {
                transport_history = transport_history.child(hint(format!(
                    "{} {} s · {} · {} {}",
                    tr("há"),
                    now.saturating_sub(observation.observed_at_unix_ms) / 1000,
                    observation.code,
                    observation.device_count,
                    tr("aparelho(s)")
                )));
            }
            page = page.child(transport_history);
        }
        let mut usb = section("usb-inventory", IconName::Usb, "USB e interfaces Apple");
        if report.usb_devices.is_empty() {
            usb = usb.child(hint(
                "Nenhum dispositivo USB retornado pelo inventário atual.",
            ));
        }
        for device in report.usb_devices {
            usb = usb.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_size(px(13.)).child(format!(
                        "{:04x}:{:04x} · {}",
                        device.vendor_id,
                        device.product_id,
                        device.name.unwrap_or_else(|| tr("Dispositivo USB"))
                    )))
                    .child(hint(format!(
                        "{} · {} {}",
                        device.location,
                        device.interfaces.len(),
                        tr("interface(s)")
                    ))),
            );
            for interface in device.interfaces {
                usb = usb.child(hint(format!(
                    "Interface {} · {} {:02x}/{:02x}/{:02x}",
                    interface.number,
                    tr("classe"),
                    interface.class,
                    interface.subclass,
                    interface.protocol
                )));
            }
        }
        for target in report.usb_recovery_targets {
            let instance = target.instance_id.clone();
            let allowed = self.model.read(cx).session.as_ref().is_none_or(|session| {
                session.allow_usb_recovery && session.device_id == target.device_id
            });
            usb = usb.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pt_3()
                    .border_t_1()
                    .border_color(theme::border())
                    .child(hint(format!(
                        "{} · {} · {}",
                        target.device_id,
                        target.driver_service,
                        target.driver_version.unwrap_or_default()
                    )))
                    .child(hint(target.instance_id))
                    .child(
                        button(format!("recover-usb-{}", target.device_id))
                            .label(tr("Reiniciar esta interface USB"))
                            .disabled(busy || !allowed)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let instance = instance.clone();
                                this.model
                                    .update(cx, |model, cx| model.recover_usb(instance, cx));
                            })),
                    ),
            );
        }
        page.child(usb).child(hint("Recuperações exigem privilégios e respeitam o intervalo do backend. O programa não reinicia o computador nem repete ações no aparelho.")).into_any_element()
    }
    fn render_integrations(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let busy = model.busy;
        let connected = model.session.is_some();
        let mcp=section("mcp-integration",IconName::Bot,"Automação com IA")
            .child(hint("Conecte seu cliente MCP à sessão local. Consulte o estado antes e depois de cada ação."))
            .child(div().text_size(px(12.)).font_family("Consolas").child("ui_snapshot → find_element → activate_element → ui_snapshot"))
            .child(primary_button("copy-mcp").max_w(px(280.)).icon(icons::icon(IconName::Copy)).label(tr("Copiar configuração MCP")).disabled(!connected||busy)
                .tooltip(tr("Inclui o token temporário desta sessão. Cole somente no cliente de IA em que você confia."))
                .on_click(cx.listener(|this,_,_,cx|{
                    match this.model.read(cx).mcp_config(){
                        Ok(config)=>{cx.write_to_clipboard(ClipboardItem::new_string(config));this.model.update(cx,|model,cx|{model.status="Configuração MCP copiada por solicitação. Ela contém o token da sessão.".into();cx.notify();});}
                        Err(error)=>this.model.update(cx,|model,cx|{model.error=Some(error);cx.notify();}),
                    }
                })));
        let server = section("existing-server", IconName::Server, "Servidor existente")
            .child(hint(
                "Endereço de loopback ou encaminhamento SSH. Token mantido somente em memória.",
            ))
            .child(field(
                &self.origin,
                "Endereço da API",
                busy || connected,
                cx,
            ))
            .child(field(
                &self.token,
                "Token de autenticação",
                busy || connected,
                cx,
            ))
            .child(
                button("connect-existing")
                    .max_w(px(240.))
                    .label(tr("Conectar ao servidor"))
                    .disabled(busy || connected)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let origin = this.origin.read(cx).value().trim().to_owned();
                        let token = this.token.read(cx).value().to_string();
                        this.model
                            .update(cx, |model, cx| model.connect_existing(origin, token, cx));
                    })),
            );
        div().flex().flex_col().gap(px(16.))
            .child(self.page_title("Conecte suas ferramentas.","MCP para agentes, servidor local para clientes e assinatura própria para elementos."))
            .child(mcp).child(server)
            .child(section("signing-guide",IconName::ShieldCheck,"Assinatura do iOS")
                .child(hint("O aplicativo Windows não precisa de certificado Apple. Cada usuário ou equipe prepara um runner WDA assinado e um perfil que autorize seus próprios aparelhos."))
                .child(hint("O pacote da comunidade não inclui chave privada nem runner pessoal. Informe o Bundle ID em Conectar → Opções da sessão."))
                .child(button("apple-signing-docs").max_w(px(300.)).ghost().icon(icons::icon(IconName::ExternalLink)).label(tr("Guia oficial de perfis Apple"))
                    .on_click(|_,_,cx|cx.open_url("https://developer.apple.com/help/account/provisioning-profiles/create-a-development-provisioning-profile"))))
            .into_any_element()
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let openness = gpui_kit::base::transition_with_status(
            "sidebar-openness",
            if self.collapsed { 0.0_f32 } else { 1.0_f32 },
            gpui_kit::base::Transition::new(Duration::from_millis(220))
                .ease(gpui_kit::base::animation::ease_in_out_cubic),
            window,
            cx,
        );
        let mut page_progress = 1.0;
        for (page, id) in [
            (Page::Connect, "page-connect"),
            (Page::Elements, "page-elements"),
            (Page::Diagnostics, "page-diagnostics"),
            (Page::Integrations, "page-integrations"),
            (Page::Settings, "page-settings"),
        ] {
            let presence = gpui_kit::base::Presence::new(id, self.page == page)
                .transition(gpui_kit::base::Transition::new(Duration::from_millis(180)))
                .sample(window, cx);
            if self.page == page {
                page_progress = presence.progress;
            }
        }
        let page = match self.page {
            Page::Connect => self.render_connect(window, cx),
            Page::Elements => self.render_elements(cx),
            Page::Diagnostics => self.render_diagnostics(cx),
            Page::Integrations => self.render_integrations(cx),
            Page::Settings => self.render_settings(cx),
        };
        let model = self.model.read(cx);
        let status = presentation_notice(&model.status, model, &self.config);
        let error = model
            .error
            .as_deref()
            .map(|error| presentation_notice(error, model, &self.config));
        let events: Vec<_> = model
            .events
            .iter()
            .map(|event| presentation_notice(event, model, &self.config))
            .collect();
        let sidebar = self.sidebar(openness.value, cx);
        let header = self.header(cx);
        let mut main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(header);
        if let Some(error) = error {
            main = main.child(
                component::alert::Alert::error("current-error", tr(&error)).on_close(cx.listener(
                    |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.error = None;
                            cx.notify();
                        })
                    },
                )),
            );
        }
        let content = div()
            .id("desktop-content")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p(px(24.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(px(960.))
                    .w_full()
                    .h_full()
                    .mx_auto()
                    .relative()
                    .top(px(8. * (1. - page_progress)))
                    .opacity(page_progress)
                    .child(page),
            );
        main = main.child(if self.page == Page::Elements {
            content.into_any_element()
        } else {
            content.overflow_y_scrollbar().into_any_element()
        });
        if self.activity_open {
            main = main.child(
                div()
                    .id("activity-log")
                    .h(px(140.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme::border())
                    .p(px(12.))
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .children(events.into_iter().map(hint)),
                    ),
            );
        }
        main = main.child(
            StatusBar::new()
                .h(px(28.))
                .border_t_1()
                .border_color(theme::border())
                .bg(theme::chrome())
                .left(
                    button("toggle-activity")
                        .h(px(24.))
                        .min_h(px(24.))
                        .ghost()
                        .xsmall()
                        .icon(icons::icon(IconName::Activity))
                        .label(tr("Atividade"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity_open = !this.activity_open;
                            cx.notify();
                        })),
                )
                .child(hint(status).truncate())
                .right(hint(if presentation_active() {
                    "Apresentação · dados do aparelho ocultos"
                } else {
                    "Open source · MIT"
                })),
        );
        let sidebar_width = px(64.) + (self.sidebar_expanded_width - px(64.)) * openness.value;
        let moving = matches!(
            openness.status,
            gpui_kit::base::MotionStatus::Running | gpui_kit::base::MotionStatus::Delayed
        );
        let sidebar_range = if self.collapsed || moving {
            sidebar_width..sidebar_width
        } else {
            px(200.)..px(320.)
        };
        let layout = h_resizable("workspace-panels")
            .with_state(&self.sidebar_resize)
            .on_resize(cx.listener(|this, state: &Entity<ResizableState>, _, cx| {
                if !this.collapsed {
                    if let Some(width) = state.read(cx).sizes().first().copied() {
                        this.sidebar_expanded_width = width.max(px(200.)).min(px(320.));
                    }
                }
            }))
            .child(
                resizable_panel()
                    .size(sidebar_width)
                    .size_range(sidebar_range)
                    .flex_none()
                    .child(sidebar),
            )
            .child(
                resizable_panel()
                    .size_range(px(320.)..Pixels::MAX)
                    .child(main),
            );
        div()
            .id("tetherdeck-workspace")
            .key_context("TetherDeck")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_sidebar(cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(14.))
            .child(self.caption(window, cx))
            .child(div().flex_1().min_h_0().w_full().child(layout))
    }
}
