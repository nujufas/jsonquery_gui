//! The MCP server in the app — built only with the cargo feature `mcp`: the 🔌 at the
//! right of the status bar, the panel it opens (start and stop the server, what the
//! assistant may do, the configuration to add it to an assistant with), what is kept
//! of it between runs (`settings.rs`), and what its tools do in the app (`host.rs`).
//!
//! Nothing of it runs until the user starts the server (or has asked for it to start
//! with the app): until then it is the button and nothing more.

mod ask;
mod host;
mod settings;
#[cfg(test)]
mod tests;

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use jsonquery_mcp::{url_of, Access, Config, Server, PAGE_SIZES, SERVER_NAME};

use self::host::AppHost;
use self::settings::McpSettings;
use super::{lock, App};
use crate::settings::{FileLimits, Store};

/// The plug: the status bar's button for the server.
pub(super) const ICON: &str = "🔌";

/// What the plug is drawn in while the server runs.
const ON_COLOR: egui::Color32 = egui::Color32::from_rgb(0x2e, 0xa0, 0x43);

/// How long "Copied" is said after a configuration was copied, in seconds.
const COPIED_SECS: f64 = 4.0;

/// How many of the last calls the panel lists.
const SHOWN_CALLS: usize = 5;

/// How wide the panel is, always.
const PANEL_WIDTH: f32 = 380.0;

/// An assistant the panel copies a configuration for.
struct Client {
    name: &'static str,
    tip: &'static str,
    /// It starts `jsonquery-mcp` itself, and talks to it over stdin and stdout (the
    /// program reads the token from `mcp.json`: what is copied holds none).
    starts_bridge: bool,
}

/// The panel's "Add to" buttons, a row of them to a line: what each copies is
/// [`Link::configuration`]'s.
const CLIENTS: [&[Client]; 2] = [
    &[
        Client {
            name: "Claude Code",
            tip: "Copy the command that adds it to Claude Code\nIt holds the token: keep it to yourself",
            starts_bridge: false,
        },
        Client {
            name: "Claude Desktop",
            tip: "Copy the entry for claude_desktop_config.json",
            starts_bridge: true,
        },
        Client {
            name: "VS Code",
            tip: "Copy the entry for VS Code's mcp.json\nIt holds the token: keep it to yourself",
            starts_bridge: false,
        },
    ],
    &[
        Client {
            name: "QuickAI",
            tip: "Copy what goes in QuickAI's Extra command-line arguments (llama.cpp)",
            starts_bridge: true,
        },
        Client {
            name: "JSON",
            tip: "Copy an mcpServers entry: .mcp.json, Cursor and others\nIt holds the token: keep it to yourself",
            starts_bridge: false,
        },
    ],
];

/// The program that takes the calls of an assistant that starts its servers itself to
/// the app (`jsonquery-mcp`, beside the app's own executable), if it is there.
fn bridge_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bridge = exe.with_file_name(format!("jsonquery-mcp{}", std::env::consts::EXE_SUFFIX));
    bridge.is_file().then_some(bridge)
}

/// The server, as the app holds it.
#[derive(Default)]
pub(super) struct Link {
    settings: McpSettings,
    /// Where the settings are kept (none in the tests).
    file: Option<PathBuf>,
    server: Option<Server>,
    /// The app, for the server's tools (set once the app is behind its lock).
    me: Weak<Mutex<App>>,
    /// What went wrong last: starting, or reading or writing the settings.
    error: Option<String>,
    /// The port box, as typed.
    port_text: String,
    /// What was copied last, and when (egui's clock).
    copied: Option<(&'static str, f64)>,
    /// What the assistant asks the user before it is done.
    asks: ask::Asks,
}

impl Link {
    pub(super) fn new(store: &Store) -> Self {
        let file = store.path().map(|p| p.with_file_name(settings::FILE));
        let (settings, error) = settings::load(file.as_deref());
        Link {
            port_text: settings.port.to_string(),
            settings,
            file,
            error,
            ..Link::default()
        }
    }

    /// The app is behind its lock now: its server can reach it, and starts if the user
    /// asked for it to start with the app.
    pub(super) fn attach(app: &Arc<Mutex<App>>, ctx: &egui::Context) {
        let mut guard = lock(app);
        let app_ref: &mut App = &mut guard;
        app_ref.mcp.me = Arc::downgrade(app);
        if app_ref.mcp.settings.start_with_app {
            let limits = app_ref.settings.limits;
            app_ref.mcp.start(ctx, limits);
        }
    }

    /// What the assistant may do.
    pub(super) fn access(&self) -> Access {
        self.settings.access
    }

    /// The user is not asked before what Edit allows is done.
    pub(super) fn always_allow(&self) -> bool {
        self.settings.always_allow
    }

    /// The most an answer may hold, in bytes.
    pub(super) fn page(&self) -> usize {
        self.settings.page
    }

    /// The question an assistant waits on, if one does, over the main window. "Always
    /// allow" there is kept, as the panel's box is.
    pub(super) fn questions(&mut self, ctx: &egui::Context) {
        if self.asks.show(ctx) && !self.settings.always_allow {
            self.settings.always_allow = true;
            self.save();
        }
    }

    fn start(&mut self, ctx: &egui::Context, limits: FileLimits) {
        let host = Arc::new(AppHost::new(self.me.clone(), ctx.clone(), limits));
        let notify = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let config = Config {
            port: self.settings.port,
            token: self.settings.token.clone(),
            access: self.settings.access,
            page: self.settings.page,
            notify,
        };
        match Server::start(config, host) {
            Ok(server) => {
                self.server = Some(server);
                self.error = None;
                self.save();
            }
            Err(e) => self.error = Some(start_error(&e, self.settings.port)),
        }
    }

    fn stop(&mut self) {
        self.server = None;
    }

    /// The port box's text, as the port if it is one (1 to 65535), kept; the box is
    /// given the port back either way.
    fn take_port(&mut self) {
        let typed = self.port_text.trim().parse::<u16>().ok().filter(|p| *p > 0);
        if let Some(port) = typed.filter(|p| *p != self.settings.port) {
            self.settings.port = port;
            self.save();
        }
        self.port_text = self.settings.port.to_string();
    }

    fn save(&mut self) {
        if let Some(file) = &self.file {
            if let Err(e) = settings::save(file, &self.settings) {
                self.error = Some(format!("{} could not be written: {e}", settings::FILE));
            }
        }
    }

    fn url(&self) -> String {
        match &self.server {
            Some(server) => server.url(),
            None => url_of(self.settings.port),
        }
    }

    /// The 🔌 at the right of the status bar, and the panel it opens.
    ///
    /// The status bar lays its right side out from the right: the spinner that shows
    /// while the assistant's call runs comes after the plug, at its left, and so moves
    /// nothing.
    pub(super) fn button(&mut self, ui: &mut egui::Ui, limits: FileLimits) {
        let busy = self
            .server
            .as_ref()
            .is_some_and(|server| server.status().running > 0);
        let mut icon = egui::RichText::new(ICON);
        if self.server.is_some() {
            icon = icon.color(ON_COLOR);
        }
        let tip = match &self.server {
            Some(server) => format!(
                "MCP server — on, at {}\nClick for its settings",
                server.url()
            ),
            None => {
                "MCP server — lets an AI assistant work with the app\nClick to set it up".to_owned()
            }
        };
        let response = ui.small_button(icon).on_hover_text(tip);
        egui::Popup::from_toggle_button_response(&response)
            .align(egui::RectAlign::TOP_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.panel(ui, limits));
        if busy {
            ui.spinner()
                .on_hover_text("MCP server — the assistant's call is running");
        }
    }

    /// The panel. It keeps one width and the same rows whatever happens, so that
    /// nothing moves under the pointer: it hangs above the 🔌 and grows upwards, and a
    /// row that came or went would move every button above it.
    fn panel(&mut self, ui: &mut egui::Ui, limits: FileLimits) {
        ui.set_width(PANEL_WIDTH);
        ui.horizontal(|ui| {
            ui.strong("MCP server");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.server.is_some() {
                    if ui
                        .button("Stop")
                        .on_hover_text("Stop it: assistants are disconnected")
                        .clicked()
                    {
                        self.stop();
                    }
                } else if ui
                    .button("Start")
                    .on_hover_text("Listen on this computer only (127.0.0.1)")
                    .clicked()
                {
                    // A port typed and not yet left is the one meant.
                    self.take_port();
                    self.start(ui.ctx(), limits);
                }
            });
        });
        let now = ui.input(|i| i.time);
        egui::Grid::new("mcp_panel")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Address");
                ui.horizontal(|ui| {
                    // As tall whether the port's box is in it or not, so that the panel
                    // keeps its height when the server starts or stops.
                    ui.set_min_height(ui.text_style_height(&egui::TextStyle::Monospace) + 4.0);
                    if self.server.is_some() {
                        ui.monospace(self.url());
                    } else {
                        ui.monospace("http://127.0.0.1:");
                        let edit = egui::TextEdit::singleline(&mut self.port_text)
                            .desired_width(48.0)
                            .font(egui::TextStyle::Monospace);
                        let response = ui.add(edit).on_hover_text("The port: 1 to 65535");
                        // Taken when the box is left (or Start clicked), whole: while it is
                        // typed, "70000" goes through 7000, which is a port too.
                        if response.lost_focus() {
                            self.take_port();
                        }
                        ui.monospace(jsonquery_mcp::PATH);
                    }
                });
                ui.end_row();

                ui.label("Allow");
                ui.horizontal(|ui| {
                    for access in Access::ALL {
                        let tip = match access {
                            Access::Read => "Read — look at documents, queries, results and the workspace, out of sight\nNothing in the window changes",
                            Access::Edit => "Edit — also open files, set the query, show values and Tools pages, save and delete\nYou are asked before a delete, a replace, a save to a file or a web address",
                        };
                        if ui
                            .radio(self.settings.access == access, access.label())
                            .on_hover_text(tip)
                            .clicked()
                            && self.settings.access != access
                        {
                            self.settings.access = access;
                            if let Some(server) = &self.server {
                                server.set_access(access);
                            }
                            self.save();
                        }
                    }
                    ui.add_space(8.0);
                    let mut always = self.settings.always_allow;
                    let edit = self.settings.access == Access::Edit;
                    let response = ui.add_enabled(edit, egui::Checkbox::new(&mut always, "Always allow"));
                    let response = if edit {
                        response.on_hover_text(ask::ALWAYS_TIP.split('\n').next().unwrap_or_default())
                    } else {
                        response.on_disabled_hover_text("Always allow — only with Edit: Read asks nothing")
                    };
                    if response.changed() {
                        self.settings.always_allow = always;
                        self.save();
                    }
                });
                ui.end_row();

                ui.label("Answers");
                ui.horizontal(|ui| {
                    // A row of sizes, not a drop-down: egui keeps one popup open at a
                    // time, and a drop-down's list would close the panel.
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for size in PAGE_SIZES {
                        let chosen = self.settings.page == size;
                        let response = ui
                            .selectable_label(chosen, (size / 1_000).to_string())
                            .on_hover_text(format!(
                                "Answers — at most {} each; the rest comes a page at a time\nSmaller for a model with a small context (a local one)",
                                kb(size)
                            ));
                        if response.clicked() && !chosen {
                            self.settings.page = size;
                            if let Some(server) = &self.server {
                                server.set_page(size);
                            }
                            self.save();
                        }
                    }
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label("KB");
                    ui.weak(format!("≈ {} tokens", tokens(self.settings.page)));
                });
                ui.end_row();

                let bridge = bridge_path();
                for (row, clients) in CLIENTS.iter().enumerate() {
                    ui.label(if row == 0 { "Add to" } else { "" });
                    ui.horizontal(|ui| {
                        for client in *clients {
                            let usable = !client.starts_bridge || bridge.is_some();
                            let button = ui.add_enabled(usable, egui::Button::new(client.name).small());
                            let button = if usable {
                                button.on_hover_text(client.tip)
                            } else {
                                button.on_disabled_hover_text(
                                    "jsonquery-mcp is not beside JsonQuery: it comes in the download of the MCP build",
                                )
                            };
                            if button.clicked() {
                                ui.ctx().copy_text(self.configuration(client.name, bridge.as_deref()));
                                self.copied = Some((client.name, now));
                            }
                        }
                        // Said where it moves nothing: at the end of the last row.
                        if row + 1 == CLIENTS.len() {
                            if let Some((_, at)) =
                                self.copied.filter(|(_, at)| now - at < COPIED_SECS)
                            {
                                ui.weak("✔ Copied");
                                ui.ctx().request_repaint_after(
                                    std::time::Duration::from_secs_f64(COPIED_SECS - (now - at)),
                                );
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        let mut start_with_app = self.settings.start_with_app;
        if ui
            .checkbox(&mut start_with_app, "Start it with JsonQuery")
            .changed()
        {
            self.settings.start_with_app = start_with_app;
            self.save();
        }
        ui.separator();
        // What it is doing: the same rows, filled or not.
        let status = self.server.as_ref().map(Server::status);
        let waiting = self.asks.waiting();
        let (first, first_tip, first_color) = match (&self.error, &status) {
            (None, Some(_)) if waiting > 0 => (
                if waiting == 1 {
                    "An assistant waits for your answer".to_owned()
                } else {
                    format!("{waiting} questions wait for your answer")
                },
                None,
                Some(ui.visuals().warn_fg_color),
            ),
            (Some(error), _) => (
                error.clone(),
                Some(error.clone()),
                Some(crate::tools::ERROR_COLOR),
            ),
            (None, None) => ("Not running".to_owned(), None, None),
            (None, Some(s)) if s.clients.is_empty() => {
                ("No assistant has connected yet".to_owned(), None, None)
            }
            (None, Some(s)) => {
                let names = s.clients.join(", ");
                (format!("Connected: {names}"), Some(names), None)
            }
        };
        status_line(ui, &first, first_tip.as_deref(), first_color);
        let calls = match &status {
            Some(s) if s.calls == 1 => "1 call".to_owned(),
            Some(s) if s.calls > 1 => format!("{} calls", s.calls),
            Some(_) => "No calls yet".to_owned(),
            None => String::new(),
        };
        status_line(ui, &calls, None, None);
        let recent = status.map(|s| s.recent).unwrap_or_default();
        for row in 0..SHOWN_CALLS {
            match recent.get(row) {
                Some(call) => {
                    let mark = if call.ok { "✔" } else { "✖" };
                    let line = format!(
                        "{mark} {}  {} ms  {}",
                        call.tool,
                        call.took.as_millis(),
                        ago(call.at)
                    );
                    let color = (!call.ok).then_some(crate::tools::ERROR_COLOR);
                    status_line(ui, &line, call.error.as_deref(), color);
                }
                None => status_line(ui, "", None, None),
            }
        }
        ui.separator();
        if ui
            .small_button("New token")
            .on_hover_text("Assistants added with the old token must be added again")
            .clicked()
        {
            self.settings.token = settings::new_token();
            self.save();
            if self.server.is_some() {
                self.stop();
                self.start(ui.ctx(), limits);
            }
        }
    }

    /// Listen on this port (0: any free one), for the layout tests.
    #[cfg(test)]
    pub(super) fn use_port(&mut self, port: u16) {
        self.settings.port = port;
        self.port_text = port.to_string();
    }

    #[cfg(test)]
    pub(super) fn is_running(&self) -> bool {
        self.server.is_some()
    }

    /// What adds the server to the assistant `name` (one of the panel's buttons);
    /// `bridge` is `jsonquery-mcp`, for those that start it themselves.
    fn configuration(&self, name: &str, bridge: Option<&std::path::Path>) -> String {
        let url = self.url();
        let bearer = format!("Bearer {}", self.settings.token);
        let command = bridge.map_or_else(
            || format!("jsonquery-mcp{}", std::env::consts::EXE_SUFFIX),
            |b| b.display().to_string(),
        );
        match name {
            "Claude Desktop" => pretty(&serde_json::json!({
                "mcpServers": {SERVER_NAME: {"command": command, "args": []}}
            })),
            // QuickAI splits the field at spaces and takes quotes away, but keeps what is
            // in single quotes as it is: the JSON, spaces and all.
            "QuickAI" => format!(
                "--mcp-servers-json '{}'",
                serde_json::json!({"mcpServers": {SERVER_NAME: {"command": command}}})
            ),
            "Claude Code" => format!(
                "claude mcp add --transport http {SERVER_NAME} {url} --header \"Authorization: {bearer}\""
            ),
            "VS Code" => pretty(&serde_json::json!({
                "servers": {SERVER_NAME: {"type": "http", "url": url, "headers": {"Authorization": bearer}}}
            })),
            _ => pretty(&serde_json::json!({
                "mcpServers": {SERVER_NAME: {"type": "http", "url": url, "headers": {"Authorization": bearer}}}
            })),
        }
    }
}

/// One line of what the server is doing, cut to the panel's width (the whole of it on
/// hover), so that a long one takes no second row. An empty one keeps its row.
fn status_line(ui: &mut egui::Ui, text: &str, tip: Option<&str>, color: Option<egui::Color32>) {
    let mut rich = egui::RichText::new(if text.is_empty() { " " } else { text });
    rich = match color {
        Some(color) => rich.color(color),
        None => rich.weak(),
    };
    let response = ui.add(egui::Label::new(rich).truncate());
    if let Some(tip) = tip {
        response.on_hover_text(tip);
    }
}

/// A page's size, as the panel says it: "24 KB".
fn kb(bytes: usize) -> String {
    format!("{} KB", bytes / 1_000)
}

/// About how many tokens of a model a page is (about four bytes make one): "6,000".
fn tokens(bytes: usize) -> String {
    let tokens = bytes / 4;
    if tokens >= 1_000 {
        format!("{},{:03}", tokens / 1_000, tokens % 1_000)
    } else {
        tokens.to_string()
    }
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// What the panel says when the server could not start.
fn start_error(e: &io::Error, port: u16) -> String {
    match e.kind() {
        io::ErrorKind::AddrInUse => format!(
            "Port {port} is taken (by another JsonQuery?): stop that, or choose another port"
        ),
        io::ErrorKind::PermissionDenied => {
            format!("Port {port} may not be listened on here: choose another port")
        }
        _ => format!("The server could not start: {e}"),
    }
}

/// How long ago `at` was, in a few characters.
fn ago(at: std::time::SystemTime) -> String {
    let secs = at.elapsed().map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}
