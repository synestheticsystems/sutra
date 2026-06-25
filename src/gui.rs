use iced::widget::{
    Column, column, container, mouse_area, row, scrollable, svg, text, text_editor, tooltip,
};
use iced::{Element, Font, Subscription, Theme, color};

use crate::model::{self, Environment, State};
use crate::notifications::Notifier;
use crate::watcher::{RegistryWatcher, WatchEvent};

/// Set the macOS dock icon from embedded PNG bytes.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/icon.png"));
    let image = NSImage::initWithData(NSImage::alloc(), &data);
    if let Some(image) = image {
        let app = NSApplication::sharedApplication(mtm);
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

const MONO: Font = Font::MONOSPACE;

// ---------------------------------------------------------------------------
// Lucide SVG icons (24x24 viewBox, stroke-based)
// ---------------------------------------------------------------------------

const ICON_VOLUME_2: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>"#;

const ICON_VOLUME_X: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" y1="9" x2="16" y2="15"/><line x1="16" y1="9" x2="22" y2="15"/></svg>"#;

const ICON_SUN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>"#;

const ICON_MOON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>"#;

const ICON_BELL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>"#;

const ICON_BELL_OFF: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13.73 21a2 2 0 0 1-3.46 0"/><path d="M18.63 13A17.89 17.89 0 0 1 18 8"/><path d="M6.26 6.26A5.86 5.86 0 0 0 6 8c0 7-3 9-3 9h14"/><path d="M18 8a6 6 0 0 0-9.33-5"/><line x1="1" y1="1" x2="23" y2="23"/></svg>"#;

const ICON_SQUARE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/></svg>"#;

const ICON_HELP_CIRCLE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>"#;

const ICON_COPY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>"#;

const ICON_CHECK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>"#;

/// Human-readable GitHub URL for the integration guide. Shown as a
/// clickable link below the agent prompt.
const HELP_DOC_BLOB_URL: &str =
    "https://github.com/synestheticsystems/sutra/blob/main/docs/INTEGRATION.md";

/// Drop-into-an-agent prompt. Available via the help panel's "Copy" button
/// — the user pastes it into Claude Code / Cursor / etc. and the agent
/// fetches the doc above and updates their dev script.
///
/// The URL pins to `main` rather than a release tag because the doc is a
/// living "best-practices" reference; we want shipped binaries to point
/// at the latest patterns, not a frozen snapshot. The doc structure is
/// stable (markdown, agent-targeted preamble, contract, recipe, checklist)
/// and changes are additive in practice — if that ever stops being true,
/// switch to a `vX.Y` tag and bump it on release.
const HELP_AGENT_PROMPT: &str = "Make my project's dev script sutra-compatible by following:
https://raw.githubusercontent.com/synestheticsystems/sutra/main/docs/INTEGRATION.md

If a dev script already exists, update it. Otherwise create one.
";

/// Build an iced Svg handle from an embedded byte slice, rendered at the given size and color.
fn icon_svg(data: &'static [u8], size: f32, color: iced::Color) -> Element<'static, Message> {
    let handle = svg::Handle::from_memory(data);
    svg(handle)
        .width(size)
        .height(size)
        .style(move |_theme, _status| svg::Style { color: Some(color) })
        .into()
}

/// Style for tooltip bubbles — background, rounded border, subtle shadow.
/// Wrap tooltip text in a styled container bubble.
fn tip_bubble(label: impl ToString, pal: &Palette) -> Element<'static, Message> {
    let fg = pal.fg;
    let bg = if pal.fg == color!(0x1e1e2e) {
        // light mode: slightly dark bubble
        color!(0x2c2c2c)
    } else {
        // dark mode: slightly lighter bubble
        color!(0x444860)
    };
    container(text(label.to_string()).size(11).color(color!(0xffffff)))
        .padding(iced::Padding::from([4.0, 8.0]))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: fg,
                width: 0.0,
                radius: 6.0.into(),
            },
            shadow: iced::Shadow {
                color: color!(0x000000, 0.25),
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            text_color: None,
        })
        .into()
}

/// Theme-aware color palette.
#[derive(Clone, Copy)]
struct Palette {
    fg: iced::Color,
    muted: iced::Color,
    card_bg: iced::Color,
    card_border: iced::Color,
    card_shadow: iced::Color,
    hover_bg: iced::Color,
    green: iced::Color,
    yellow: iced::Color,
    red: iced::Color,
    gray: iced::Color,
    cyan: iced::Color,
}

fn palette(dark_mode: bool) -> Palette {
    if dark_mode {
        Palette {
            fg: color!(0xf8f8f2),
            muted: color!(0x8890a0),
            card_bg: color!(0x282a36),
            card_border: color!(0x383a4a),
            card_shadow: color!(0x000000, 0.4),
            hover_bg: color!(0x343648),
            green: color!(0x50fa7b),
            yellow: color!(0xf1fa8c),
            red: color!(0xff5555),
            gray: color!(0x6272a4),
            cyan: color!(0x8be9fd),
        }
    } else {
        Palette {
            fg: color!(0x1e1e2e),
            muted: color!(0x9399a8),
            card_bg: color!(0xffffff),
            card_border: color!(0xeaeaea),
            card_shadow: color!(0x000000, 0.08),
            hover_bg: color!(0xf0f0f4),
            green: color!(0x2da44e),
            yellow: color!(0x9a6700),
            red: color!(0xd1242f),
            gray: color!(0xa0a8b8),
            cyan: color!(0x0969da),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    WatchEvent,
    ToggleGlobalMute,
    ToggleUnitMute { env_id: String, unit_name: String },
    ToggleGlobalNotifications,
    ToggleUnitNotifications { env_id: String, unit_name: String },
    ToggleTheme,
    ToggleHelp,
    OpenHelp,
    CloseHelp,
    CopyToClipboard(String),
    OpenBrowser { port: u16 },
    OpenUrl(String),
    PromptAction(text_editor::Action),
    TerminateEnv { pid: u32 },
    HoverUnit { env_id: String, unit_name: String },
    UnhoverUnit,
    Quit,
}

struct App {
    envs: Vec<Environment>,
    notifier: Notifier,
    dark_mode: bool,
    hovered_unit: Option<(String, String)>, // (env_id, unit_name)
    show_help: bool,
    /// Set true when the user clicks the help-panel Copy button. Cleared
    /// on the next periodic Tick (~2s) so the button visibly toggles to
    /// "Copied!" and back without needing an async timer.
    copied_flash: bool,
    /// Backing store for the help-panel prompt's `text_editor` widget.
    /// We accept all non-Edit actions so the user can select and scroll
    /// the prompt text, but block Edit actions to keep it read-only.
    prompt_content: text_editor::Content,
}

pub fn run() {
    #[cfg(target_os = "macos")]
    set_dock_icon();

    let icon = iced::window::icon::from_file_data(include_bytes!("../assets/icon.png"), None).ok();

    iced::application("Sutra", update, view)
        .theme(theme)
        .subscription(subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(480.0, 420.0),
            icon,
            ..Default::default()
        })
        .run_with(|| {
            let envs = model::load_all();
            let mut notifier = Notifier::new();
            notifier.process(&envs);
            (
                App {
                    envs,
                    notifier,
                    dark_mode: false,
                    hovered_unit: None,
                    show_help: false,
                    copied_flash: false,
                    prompt_content: text_editor::Content::with_text(HELP_AGENT_PROMPT.trim_end()),
                },
                iced::Task::none(),
            )
        })
        .expect("failed to launch GUI");
}

fn update(app: &mut App, message: Message) -> iced::Task<Message> {
    match message {
        Message::Tick => {
            app.envs = model::load_all();
            app.notifier.process(&app.envs);
            // Clear the copy-flash on the next periodic refresh. WatchEvent
            // doesn't clear it, so unrelated filesystem activity won't snap
            // the "Copied!" label away early.
            app.copied_flash = false;
        }
        Message::WatchEvent => {
            app.envs = model::load_all();
            app.notifier.process(&app.envs);
        }
        Message::ToggleGlobalMute => {
            app.notifier.toggle_global_mute();
        }
        Message::ToggleTheme => {
            app.dark_mode = !app.dark_mode;
        }
        Message::ToggleHelp => {
            app.show_help = !app.show_help;
        }
        Message::OpenHelp => {
            // Idempotent: '?' from the keyboard fires this. Open-only so
            // typing '?' while focused inside the help panel's editor
            // doesn't snap the panel closed.
            app.show_help = true;
        }
        Message::CloseHelp => {
            // Idempotent: Esc fires this. No-op when the panel isn't
            // open, so it doesn't trample any future Esc behavior.
            app.show_help = false;
        }
        Message::CopyToClipboard(content) => {
            app.copied_flash = true;
            return iced::clipboard::write::<Message>(content);
        }
        Message::PromptAction(action) => {
            // Read-only: drop Edit actions, perform everything else (cursor
            // movement, click/drag selection, scroll, SelectAll, etc.).
            if !matches!(action, text_editor::Action::Edit(_)) {
                app.prompt_content.perform(action);
            }
        }
        Message::ToggleUnitMute { env_id, unit_name } => {
            app.notifier.toggle_unit_mute(&env_id, &unit_name);
        }
        Message::ToggleGlobalNotifications => {
            app.notifier.toggle_global_notifications();
        }
        Message::ToggleUnitNotifications { env_id, unit_name } => {
            app.notifier.toggle_unit_notifications(&env_id, &unit_name);
        }
        Message::OpenBrowser { port } => {
            let _ = std::process::Command::new("open")
                .arg(format!("http://localhost:{port}"))
                .spawn();
        }
        Message::OpenUrl(url) => {
            // Cross-platform browser launcher. macOS: `open`, Linux:
            // `xdg-open`, Windows: `start` via cmd. We probe one and
            // ignore failure — there's no useful UI for "couldn't open
            // the link", and the user can read the URL on screen.
            #[cfg(target_os = "macos")]
            let opener = ("open", None::<&str>);
            #[cfg(target_os = "linux")]
            let opener = ("xdg-open", None::<&str>);
            #[cfg(target_os = "windows")]
            let opener = ("cmd", Some("/C start"));
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
            let opener = ("xdg-open", None::<&str>); // best-guess fallback

            let mut cmd = std::process::Command::new(opener.0);
            if let Some(arg) = opener.1 {
                cmd.arg(arg);
            }
            let _ = cmd.arg(url).spawn();
        }
        Message::TerminateEnv { pid } => {
            if let Ok(raw_pid) = i32::try_from(pid) {
                // Sutra is a situational-awareness dashboard with
                // intentionally limited control: one shutdown button
                // per env, which sends a single SIGTERM to the one
                // PID the dev runner published — its supervisor.
                // The supervisor's own trap reaps its children,
                // does any orderly cleanup, and removes the registry
                // entry. Signaling the process group ourselves would
                // override the supervisor's shutdown sequence and
                // overstep the dashboard boundary.
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(raw_pid),
                    nix::sys::signal::Signal::SIGTERM,
                );
            }
        }
        Message::HoverUnit { env_id, unit_name } => {
            app.hovered_unit = Some((env_id, unit_name));
        }
        Message::UnhoverUnit => {
            app.hovered_unit = None;
        }
        Message::Quit => {
            return iced::window::get_latest().and_then(iced::window::close);
        }
    }
    iced::Task::none()
}

fn view(app: &App) -> Element<'_, Message> {
    let pal = palette(app.dark_mode);

    // Toolbar -- minimal, right-aligned controls with SVG icons
    let toolbar = {
        let mute_icon = if app.notifier.global_mute {
            ICON_VOLUME_X
        } else {
            ICON_VOLUME_2
        };

        let notif_icon = if app.notifier.global_notifications_off {
            ICON_BELL_OFF
        } else {
            ICON_BELL
        };

        let theme_icon = if app.dark_mode { ICON_SUN } else { ICON_MOON };

        let icon_color = pal.fg;
        // Help icon adopts the cyan accent when the panel is open, so it
        // reads as the active selection in the toolbar.
        let help_color = if app.show_help { pal.cyan } else { icon_color };

        let mute_tip = if app.notifier.global_mute {
            "Unmute all sounds"
        } else {
            "Mute all sounds"
        };
        let notif_tip = if app.notifier.global_notifications_off {
            "Enable notifications"
        } else {
            "Disable notifications"
        };
        let theme_tip = if app.dark_mode {
            "Switch to light mode"
        } else {
            "Switch to dark mode"
        };
        let help_tip = if app.show_help {
            "Hide integration help"
        } else {
            "Integration help"
        };

        let toolbar_row = row![
            iced::widget::horizontal_space(),
            tooltip(
                mouse_area(icon_svg(ICON_HELP_CIRCLE, 16.0, help_color))
                    .on_press(Message::ToggleHelp),
                tip_bubble(help_tip, &pal),
                tooltip::Position::Bottom,
            )
            .gap(4),
            text("\u{00b7}").size(8).color(pal.muted),
            tooltip(
                mouse_area(icon_svg(mute_icon, 16.0, icon_color))
                    .on_press(Message::ToggleGlobalMute),
                tip_bubble(mute_tip, &pal),
                tooltip::Position::Bottom,
            )
            .gap(4),
            text("\u{00b7}").size(8).color(pal.muted), // middle dot separator
            tooltip(
                mouse_area(icon_svg(notif_icon, 16.0, icon_color))
                    .on_press(Message::ToggleGlobalNotifications),
                tip_bubble(notif_tip, &pal),
                tooltip::Position::Bottom,
            )
            .gap(4),
            text("\u{00b7}").size(8).color(pal.muted),
            tooltip(
                mouse_area(icon_svg(theme_icon, 16.0, icon_color)).on_press(Message::ToggleTheme),
                tip_bubble(theme_tip, &pal),
                tooltip::Position::Bottom,
            )
            .gap(4),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        container(toolbar_row)
            .padding(iced::Padding::from([6.0, 16.0]))
            .width(iced::Fill)
    };

    if app.show_help {
        return column![
            toolbar,
            help_panel(&pal, &app.prompt_content, app.copied_flash)
        ]
        .into();
    }

    if app.envs.is_empty() {
        let dir_label = model::state_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.dev-runner/".into());
        let empty_msg = column![
            text("No environments found.").size(14).color(pal.muted),
            text(format!("Watching {} for environments", dir_label))
                .size(12)
                .color(pal.muted),
        ]
        .spacing(4)
        .align_x(iced::Alignment::Center);
        return column![
            toolbar,
            container(empty_msg)
                .padding(40)
                .center_x(iced::Fill)
                .center_y(iced::Fill),
        ]
        .into();
    }

    let mut items: Vec<Element<Message>> = Vec::new();

    for env in app.envs.iter() {
        items.push(env_card(env, &app.notifier, &pal, &app.hovered_unit));
    }

    let content = Column::with_children(items).spacing(12).width(iced::Fill);

    column![
        toolbar,
        scrollable(
            container(content)
                .padding(iced::Padding {
                    top: 4.0,
                    right: 16.0,
                    bottom: 16.0,
                    left: 16.0,
                })
                .width(iced::Fill)
        ),
    ]
    .into()
}

/// Help panel — explains the `~/.dev-runner/` contract that any dev
/// script needs to satisfy to show up in sutra, and offers a copyable
/// agent prompt that points at the full integration guide on GitHub.
/// Toggled from the toolbar `?` icon (or pressing `?`).
///
/// Returns an `Element<'a>` (not `'static`) because the prompt block
/// embeds a `text_editor` borrowing from `prompt_content`.
fn help_panel<'a>(
    pal: &Palette,
    prompt_content: &'a text_editor::Content,
    copied_flash: bool,
) -> Element<'a, Message> {
    let muted = pal.muted;
    let fg = pal.fg;
    let bg = pal.hover_bg;
    let border = pal.card_border;
    let cyan = pal.cyan;

    let title = text("Integration").size(15).color(fg);

    let intro = text(
        "Sutra watches ~/.dev-runner/. Any dev script that writes the \
         right files gets a live status row — no daemon, no IPC.",
    )
    .size(12)
    .color(muted);

    // -- Meta file -------------------------------------------------------
    // Path in mono (it's a literal); description lines in default font
    // since they're prose. The path always comes first so the eye lands
    // on "where" before "what / when".
    let meta_header = text("Meta file").size(13).color(fg);
    let meta_body = column![
        text("~/.dev-runner/<id>").size(11).color(fg).font(MONO),
        text("KEY=VALUE per line. Required: DIR, PID. Optional: STARTED, *_PORT.")
            .size(11)
            .color(muted),
        text("Write once at startup. Delete on exit.")
            .size(11)
            .color(muted),
    ]
    .spacing(3);

    // -- Status file -----------------------------------------------------
    let status_header = text("Status file").size(13).color(fg);
    let status_body = column![
        text("~/.dev-runner/<id>.<unit>.status")
            .size(11)
            .color(fg)
            .font(MONO),
        text("Single line: <state>[: <detail>]")
            .size(11)
            .color(fg)
            .font(MONO),
        text("States: starting, building, running, ready, failed, stopped.")
            .size(11)
            .color(muted),
        text("Overwrite (don't append) on each transition. The detail is freeform.")
            .size(11)
            .color(muted),
    ]
    .spacing(3);

    // Thin horizontal divider — the contract section reads as facts,
    // the agent block below reads as the call to action.
    let divider_color = pal.card_border;
    let divider = container(text(""))
        .width(iced::Fill)
        .height(1.0)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(divider_color)),
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            text_color: None,
        });

    // -- Hand-off to an agent --------------------------------------------
    let agent_header = text("Hand it to an agent").size(13).color(fg);

    let agent_intro = text(
        "Paste this into Claude Code, Cursor, or any code agent — \
         it'll fetch the full guide and wire your dev script up.",
    )
    .size(12)
    .color(muted);

    // Copy button — flashes "Copied!" with a check icon for ~1–2s after
    // press (cleared on next Tick). The non-flash state stays cyan to
    // read as a clickable accent; the flash state goes green to match
    // the "ready" semantic elsewhere in the app.
    let (copy_icon, copy_label, copy_color) = if copied_flash {
        (ICON_CHECK, "Copied", pal.green)
    } else {
        (ICON_COPY, "Copy", cyan)
    };

    let copy_btn_inner = row![
        icon_svg(copy_icon, 11.0, copy_color),
        text(copy_label).size(11).color(copy_color),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center);

    let copy_btn = mouse_area(copy_btn_inner)
        .on_press(Message::CopyToClipboard(HELP_AGENT_PROMPT.to_string()));

    // Prompt is a read-only text_editor so the user can select text
    // (URL, individual lines) instead of being forced to use Copy.
    // Edit actions are dropped in update(); selection/scroll/etc work.
    // Style the editor with a transparent background so the wrapping
    // container's bg/border define the visual block.
    let editor = text_editor(prompt_content)
        .on_action(Message::PromptAction)
        .size(11)
        .height(iced::Length::Fixed(110.0))
        .style(move |_theme, _status| text_editor::Style {
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: muted,
            placeholder: muted,
            value: fg,
            selection: cyan,
        });

    let prompt_block = container(
        column![
            editor,
            row![iced::widget::horizontal_space(), copy_btn].align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(iced::Fill)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: iced::Shadow::default(),
        text_color: None,
    });

    // GitHub link — clickable, opens in browser. Useful for the human
    // who just wants to read it themselves rather than hand it off to
    // an agent. Trailing "↗" gives the link an external-link affordance
    // since iced text doesn't underline-on-hover out of the box.
    let github_row = row![
        text("Read on GitHub: ").size(11).color(muted),
        mouse_area(
            row![
                text(HELP_DOC_BLOB_URL.to_string()).size(11).color(cyan),
                text(" \u{2197}").size(11).color(cyan),
            ]
            .align_y(iced::Alignment::Center)
        )
        .on_press(Message::OpenUrl(HELP_DOC_BLOB_URL.to_string())),
    ]
    .align_y(iced::Alignment::Center);

    let content = column![
        title,
        intro,
        meta_header,
        meta_body,
        status_header,
        status_body,
        divider,
        agent_header,
        agent_intro,
        prompt_block,
        github_row,
    ]
    .spacing(10)
    .width(iced::Fill);

    scrollable(
        container(content)
            .padding(iced::Padding {
                top: 4.0,
                right: 16.0,
                bottom: 16.0,
                left: 16.0,
            })
            .width(iced::Fill),
    )
    .into()
}

fn env_card(
    env: &Environment,
    notifier: &Notifier,
    pal: &Palette,
    hovered_unit: &Option<(String, String)>,
) -> Element<'static, Message> {
    let alive_color = if env.alive { pal.green } else { pal.gray };

    // Header: alive dot + name + elapsed + terminate button
    let mut header = row![
        text("\u{25cf}").size(10).color(alive_color),
        text(env.display_name().to_string())
            .size(15)
            .color(pal.fg)
            .font(Font::DEFAULT),
        iced::widget::horizontal_space(),
        text(env.elapsed_string()).size(12).color(pal.muted),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    if env.alive {
        let stop_btn: Element<'static, Message> = tooltip(
            mouse_area(icon_svg(ICON_SQUARE, 10.0, pal.red))
                .on_press(Message::TerminateEnv { pid: env.pid }),
            tip_bubble("Terminate environment", pal),
            tooltip::Position::Top,
        )
        .gap(4)
        .into();
        header = header.push(stop_btn);
    }

    let mut card_col = column![header].spacing(8);

    if !env.units.is_empty() {
        // Compute fixed pixel widths for table-column alignment.
        // ~7.2px per char at size 12 monospace is a reasonable approximation.
        const CHAR_W: f32 = 7.2;

        let max_name_chars = env.units.iter().map(|u| u.name.len()).max().unwrap_or(0);
        let name_col_w = (max_name_chars as f32 * CHAR_W).ceil() + 4.0;

        let has_any_port = env.units.iter().any(|u| env.port_for(&u.name).is_some());
        let port_col_w: f32 = if has_any_port {
            6.0 * CHAR_W + 4.0
        } else {
            0.0
        };

        let max_state_chars = env
            .units
            .iter()
            .map(|u| u.state.to_string().len())
            .max()
            .unwrap_or(0);
        let state_col_w = (max_state_chars as f32 * CHAR_W).ceil() + 4.0;

        let muted_color = pal.muted;
        let cyan = pal.cyan;
        let hover_bg = pal.hover_bg;

        let mut unit_col = Column::new().spacing(3);

        for unit in &env.units {
            let is_muted = notifier.is_unit_muted(&env.id, &unit.name);
            let is_notif_off = notifier.is_unit_notifications_off(&env.id, &unit.name);
            let color = state_color(&unit.state, pal);
            let indicator = unit.state.display_indicator();

            let name_color = if is_muted { pal.muted } else { pal.fg };

            // Per-unit icons (left of indicator for alignment)
            let mute_icon_data = if is_muted {
                ICON_VOLUME_X
            } else {
                ICON_VOLUME_2
            };
            let notif_icon_data = if is_notif_off {
                ICON_BELL_OFF
            } else {
                ICON_BELL
            };

            // Fixed-width cells for true table-column alignment
            let name_cell = container(
                text(unit.name.clone())
                    .size(12)
                    .color(name_color)
                    .font(MONO),
            )
            .width(name_col_w);

            let port_cell: Element<'static, Message> = if has_any_port {
                let label = match env.port_for(&unit.name) {
                    Some(p) => format!(":{p}"),
                    None => String::new(),
                };
                container(text(label).size(11).color(cyan).font(MONO))
                    .width(port_col_w)
                    .into()
            } else {
                text("").into()
            };

            let state_cell =
                container(text(unit.state.to_string()).size(12).color(color)).width(state_col_w);

            let unit_mute_tip = if is_muted {
                format!("Unmute {}", unit.name)
            } else {
                format!("Mute {}", unit.name)
            };
            let unit_notif_tip = if is_notif_off {
                format!("Enable notifications for {}", unit.name)
            } else {
                format!("Disable notifications for {}", unit.name)
            };

            let mut unit_row = row![
                // icon pair: mute + bell (fixed 12px each)
                tooltip(
                    mouse_area(icon_svg(
                        mute_icon_data,
                        12.0,
                        if is_muted { pal.muted } else { pal.fg }
                    ))
                    .on_press(Message::ToggleUnitMute {
                        env_id: env.id.clone(),
                        unit_name: unit.name.clone(),
                    }),
                    tip_bubble(unit_mute_tip, pal),
                    tooltip::Position::Top,
                )
                .gap(4),
                tooltip(
                    mouse_area(icon_svg(
                        notif_icon_data,
                        12.0,
                        if is_notif_off { pal.muted } else { pal.fg }
                    ))
                    .on_press(Message::ToggleUnitNotifications {
                        env_id: env.id.clone(),
                        unit_name: unit.name.clone(),
                    }),
                    tip_bubble(unit_notif_tip, pal),
                    tooltip::Position::Top,
                )
                .gap(4),
                // indicator dot (fixed 14px container)
                container(text(indicator.to_string()).size(11).color(color)).width(14.0),
                name_cell,
                port_cell,
                state_cell,
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center);

            if let Some(ref detail) = unit.detail {
                unit_row = unit_row.push(text(detail.clone()).size(11).color(muted_color));
            }

            // "open" link for units with ports
            if let Some(port) = env.port_for(&unit.name) {
                unit_row = unit_row.push(iced::widget::horizontal_space());
                unit_row = unit_row.push(
                    tooltip(
                        mouse_area(text("\u{2197}").size(13).color(cyan))
                            .on_press(Message::OpenBrowser { port }),
                        tip_bubble("Open in browser", pal),
                        tooltip::Position::Top,
                    )
                    .gap(4),
                );
            }

            // Wrap the row in a container for hover highlighting
            let is_hovered = hovered_unit
                .as_ref()
                .map(|(eid, uname)| eid == &env.id && uname == &unit.name)
                .unwrap_or(false);

            let row_bg = if is_hovered {
                Some(iced::Background::Color(hover_bg))
            } else {
                None
            };

            let row_container: Element<'static, Message> = container(unit_row)
                .width(iced::Fill)
                .padding(iced::Padding::from([2.0, 4.0]))
                .style(move |_theme| container::Style {
                    background: row_bg,
                    border: iced::Border {
                        color: iced::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 4.0.into(),
                    },
                    shadow: iced::Shadow::default(),
                    text_color: None,
                })
                .into();

            let env_id = env.id.clone();
            let unit_name = unit.name.clone();

            let unit_element: Element<'static, Message> = mouse_area(row_container)
                .on_enter(Message::HoverUnit { env_id, unit_name })
                .on_exit(Message::UnhoverUnit)
                .into();

            unit_col = unit_col.push(unit_element);
        }

        card_col = card_col.push(unit_col);
    }

    let card_bg = pal.card_bg;
    let card_border = pal.card_border;
    let card_shadow = pal.card_shadow;

    container(card_col.width(iced::Fill))
        .padding(iced::Padding::from([14.0, 16.0]))
        .width(iced::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(card_bg)),
            border: iced::Border {
                color: card_border,
                width: 1.0,
                radius: 10.0.into(),
            },
            shadow: iced::Shadow {
                color: card_shadow,
                offset: iced::Vector::new(0.0, 1.0),
                blur_radius: 4.0,
            },
            text_color: None,
        })
        .into()
}

fn state_color(state: &State, pal: &Palette) -> iced::Color {
    match state {
        State::Running | State::Ready => pal.green,
        State::Building | State::Starting => pal.yellow,
        State::Failed => pal.red,
        State::Stopped | State::None | State::Other(_) => pal.gray,
    }
}

fn theme(app: &App) -> Theme {
    if app.dark_mode {
        Theme::Dark
    } else {
        Theme::Light
    }
}

fn subscription(_app: &App) -> Subscription<Message> {
    let tick = iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::Tick);

    let watcher = Subscription::run(watch_registry);

    let keyboard = iced::keyboard::on_key_press(|key, modifiers| {
        if modifiers.command() {
            if let iced::keyboard::Key::Character(c) = key.as_ref() {
                if c == "q" {
                    return Some(Message::Quit);
                }
            }
        }
        // '?' opens the help panel (open-only — won't close it if you
        // type '?' while focused inside the panel's read-only editor).
        if let iced::keyboard::Key::Character(c) = key.as_ref() {
            if c == "?" {
                return Some(Message::OpenHelp);
            }
        }
        // Esc closes the help panel (no-op when it's already closed,
        // so it doesn't conflict with any future Esc binding).
        if let iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) = key.as_ref() {
            return Some(Message::CloseHelp);
        }
        None
    });

    Subscription::batch([tick, watcher, keyboard])
}

fn watch_registry() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(32, |mut sender| async move {
        use iced::futures::SinkExt;
        use iced::futures::StreamExt;

        let Ok(watcher) = RegistryWatcher::new() else {
            std::future::pending::<()>().await;
            return;
        };
        let rx = watcher.rx;

        // Bridge the std::sync::mpsc channel to an async futures::channel::mpsc
        // so we don't block iced's event loop.
        let (mut async_tx, mut async_rx) = iced::futures::channel::mpsc::channel::<WatchEvent>(32);
        std::thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                // Use try_send to avoid needing async; drop events if the
                // channel is full (the next tick will catch up).
                if async_tx.try_send(event).is_err() {
                    // Channel closed or full — exit thread
                    if async_tx.is_closed() {
                        break;
                    }
                }
            }
        });

        while async_rx.next().await.is_some() {
            let _ = sender.send(Message::WatchEvent).await;
        }
    })
}
