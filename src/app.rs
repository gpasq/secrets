//! Desktop UI (egui/eframe).
//!
//! AWS calls run on a background tokio runtime and report back over a channel, so the window
//! never blocks. A secret's value is loaded and shown as soon as it is selected, and is
//! discarded when the selection changes.

use std::future::Future;
use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Result;
use eframe::egui::{
    self, Align, Color32, ComboBox, CornerRadius, FontId, Frame, Grid, Id, Layout, Margin, Modal,
    RichText, ScrollArea, TextEdit, TextStyle, Ui,
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use crate::aws::{
    ConnOpts, Deletion, FetchedValue, ListFilter, RotationConfig, SecretDetail, SecretSummary,
    SecretValue, Sm, StaticKeys, VersionInfo,
};
use crate::connections::{Connection, ConnectionSecret, KeyringVault, Vault};
use crate::highlight::{Syntax, code_job, json_job, looks_like_json};
use crate::profiles::{REGIONS, list_profiles};
use crate::samples::{Lang, Shape};
use crate::theme::{self, Appearance, Palette, ThemeRef, VsCodeTheme};
use crate::util::{
    KvRow, b64, clean_tags, compact_if_json, detail_lines, display_value, fmt_time, format_json,
    json_object, kv_rows, kv_to_json, matches_query, same_value, tag_diff, validate_recovery_days,
};

/// Inner padding for every text field.
const FIELD_MARGIN: Margin = Margin::symmetric(10, 8);
/// Height of fixed-size text fields in tables.
const ROW_H: f32 = 34.0;

/// Key and value column widths for key/value and tag tables, leaving room for the row
/// buttons so nothing runs off the edge of narrower windows.
fn row_widths(available: f32) -> (f32, f32) {
    let key = (available * 0.3).clamp(160.0, 280.0);
    let value = (available - key - 210.0).clamp(180.0, 560.0);
    (key, value)
}

/// Roomier defaults than egui's compact look: larger text, padding and spacing.
fn apply_style(ctx: &egui::Context, palette: &Palette) {
    theme::store_palette(ctx, palette);
    ctx.all_styles_mut(|style| {
        style.visuals = palette.visuals();
        use egui::FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Small, FontId::new(12.5, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Heading, FontId::new(24.0, Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, Monospace)),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.item_spacing = egui::vec2(12.0, 12.0);
        sp.button_padding = egui::vec2(14.0, 7.0);
        sp.interact_size = egui::vec2(40.0, 32.0);
        sp.window_margin = Margin::same(24);
        sp.menu_margin = Margin::same(10);
        sp.indent = 24.0;
        sp.icon_width = 18.0;
        sp.icon_width_inner = 10.0;
        sp.icon_spacing = 8.0;
        sp.combo_width = 160.0;
        let v = &mut style.visuals;
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(7);
        }
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
    });
}

fn dialog_frame(ctx: &egui::Context) -> Frame {
    Frame::popup(&ctx.global_style()).inner_margin(Margin::same(28))
}

fn connection_opts(
    conn: &Connection,
    secret: &ConnectionSecret,
    region: Option<String>,
) -> ConnOpts {
    let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    ConnOpts {
        profile: None,
        region: region.or_else(|| opt(&conn.region)),
        endpoint_url: std::env::var("SECRETS_ENDPOINT_URL").ok(),
        keys: Some(StaticKeys {
            access_key_id: conn.access_key_id.trim().to_string(),
            secret_access_key: secret.secret_access_key.trim().to_string(),
            session_token: opt(&secret.session_token),
        }),
        role_arn: opt(&conn.role_arn),
        external_id: opt(&conn.external_id),
    }
}

fn new_editor() -> ConnEditor {
    ConnEditor {
        conn: Connection {
            id: Connection::new_id(),
            region: "us-east-1".into(),
            ..Default::default()
        },
        secret: ConnectionSecret::default(),
        is_new: true,
    }
}

/// Multiline code editor that colors JSON with the active theme's syntax colors.
fn code_editor(
    ui: &mut Ui,
    text: &mut String,
    syntax: &Syntax,
    rows: usize,
    interactive: bool,
) -> egui::Response {
    code_view(ui, text, syntax, rows, interactive, None)
}

/// Code box highlighted as `lang` source, or as JSON when `lang` is `None` and the text
/// looks like JSON.
fn code_view(
    ui: &mut Ui,
    text: &mut String,
    syntax: &Syntax,
    rows: usize,
    interactive: bool,
    lang: Option<Lang>,
) -> egui::Response {
    let font = TextStyle::Monospace.resolve(ui.style());
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let src = buf.as_str();
        let mut job = if let Some(lang) = lang {
            code_job(src, lang, syntax, font.clone())
        } else if looks_like_json(src) {
            json_job(src, syntax, font.clone())
        } else {
            egui::text::LayoutJob::simple(src.to_string(), font.clone(), syntax.plain, wrap_width)
        };
        job.wrap.max_width = wrap_width;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    ui.add(
        TextEdit::multiline(text)
            .margin(Margin::symmetric(14, 12))
            .code_editor()
            .desired_rows(rows)
            .desired_width(f32::INFINITY)
            .interactive(interactive)
            .layouter(&mut layouter),
    )
}

/// What a button does, which decides its color when colorful accents are on.
#[derive(Clone, Copy)]
enum Tone {
    /// Create or add something (green).
    Create,
    /// Save changes (blue).
    Save,
    /// Destructive (red).
    Danger,
}

/// A button filled with its tone's color from the theme's terminal palette
/// (a plain button when colorful accents are off).
fn tone_button(ui: &Ui, text: &str, tone: Tone) -> egui::Button<'static> {
    let p = theme::palette(ui);
    if !p.colorful {
        return egui::Button::new(text.to_string());
    }
    let c = match tone {
        Tone::Create => p.ansi.green,
        Tone::Save => p.ansi.blue,
        Tone::Danger => p.ansi.red,
    };
    // Slightly deeper in dark themes so white text stays readable.
    let fill = if p.dark {
        theme::mix(c, Color32::BLACK, 0.15)
    } else {
        c
    };
    egui::Button::new(RichText::new(text).color(theme::text_on(fill)).strong()).fill(fill)
}

/// A small rounded label tinted with `color` (muted when colorful accents are off).
fn chip(ui: &mut Ui, text: &str, color: Color32) {
    let p = theme::palette(ui);
    let (fg, bg, stroke) = if p.colorful {
        (
            color,
            theme::mix(p.editor_bg, color, if p.dark { 0.20 } else { 0.12 }),
            egui::Stroke::new(1.0, theme::mix(p.editor_bg, color, 0.5)),
        )
    } else {
        (p.fg_weak, p.input_bg, egui::Stroke::new(1.0, p.border))
    };
    Frame::new()
        .fill(bg)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(fg).small().strong());
        });
}

/// A short colored bar drawn before a heading.
fn marker(ui: &mut Ui, color: Color32) {
    let h = ui.text_style_height(&TextStyle::Heading);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(5.0, h * 0.8), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), color);
}

/// A stable accent per secret from the theme's terminal colors: red for production,
/// yellow for staging, green for development, otherwise picked by name.
fn name_color(name: &str, p: &Palette) -> Color32 {
    let first = name
        .split(['/', '-', '_', '.'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    match first.as_str() {
        "prod" | "production" | "prd" | "live" => p.ansi.red,
        "stag" | "staging" | "stage" | "stg" | "uat" | "qa" => p.ansi.yellow,
        "dev" | "development" | "test" | "testing" | "sandbox" | "local" => p.ansi.green,
        _ => {
            let h = first
                .bytes()
                .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            [p.ansi.blue, p.ansi.magenta, p.ansi.cyan][(h % 3) as usize]
        }
    }
}

fn red(ui: &Ui) -> Color32 {
    theme::palette(ui).error
}

enum Msg {
    Connected(Result<Sm>),
    Listed(Result<Vec<SecretSummary>>),
    Detail(String, Result<Box<(SecretDetail, Vec<VersionInfo>)>>),
    /// `None` means the secret exists but has no value yet.
    Value(String, Result<Option<FetchedValue>>),
    Peek(String, String, Result<FetchedValue>),
    Done(String, Change, Result<String>),
    Password(Result<String>),
    Tested(Result<usize>),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overview,
    Value,
    Versions,
    Tags,
    Rotation,
    Code,
}

enum Editor {
    Text { text: String, original: String },
    Kv { rows: Vec<KvRow>, original: String },
    Binary(Vec<u8>),
}

enum Action {
    Put(String, SecretValue),
    Metadata {
        id: String,
        desc: Option<String>,
        kms: Option<String>,
    },
    Tags {
        id: String,
        set: Vec<(String, String)>,
        remove: Vec<String>,
    },
    Delete(String, Deletion),
    Restore(String),
    Rotate(String, RotationConfig),
    CancelRotation(String),
    Promote(String, String),
    Create(NewSecret),
}

struct Confirm {
    title: String,
    body: String,
    button: String,
    action: Action,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum NewKind {
    #[default]
    KeyValue,
    Text,
    Empty,
}

#[derive(Default)]
struct NewSecret {
    name: String,
    description: String,
    kms: String,
    tags: Vec<(String, String)>,
    kind: NewKind,
    text: String,
    rows: Vec<KvRow>,
}

struct DeleteDialog {
    id: String,
    days: i64,
    force: bool,
    typed: String,
}

#[derive(Default)]
struct RotationEdit {
    lambda: String,
    use_schedule: bool,
    days: String,
    schedule: String,
    window: String,
    rotate_now: bool,
}

/// Where credentials come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Source {
    /// A profile in ~/.aws/config or ~/.aws/credentials.
    Profile(String),
    /// A connection saved in Settings, by id.
    Connection(String),
}

#[derive(Clone, Copy, PartialEq)]
enum SettingsPage {
    Connections,
    Appearance,
}

struct ConnEditor {
    conn: Connection,
    secret: ConnectionSecret,
    is_new: bool,
}

struct SettingsUi {
    page: SettingsPage,
    editor: Option<ConnEditor>,
    /// Id of the connection whose delete is awaiting confirmation.
    confirm_delete: Option<String>,
    testing: bool,
    test_result: Option<(bool, String)>,
}

impl SettingsUi {
    fn new(page: SettingsPage) -> Self {
        Self {
            page,
            editor: None,
            confirm_delete: None,
            testing: false,
            test_result: None,
        }
    }
}

/// What a completed change did to the secret list (ListSecrets lags behind writes).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    Created,
    Deleted,
    Restored,
    Other,
}

/// A recent create/delete the list may not reflect yet.
struct PendingChange {
    name: String,
    change: Change,
    since: std::time::Instant,
}

/// How long to keep correcting the list for a recent change.
const PENDING_FOR: std::time::Duration = std::time::Duration::from_secs(30);
/// How often to re-check while a change is pending.
const RECHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

pub struct SecretsApp {
    rt: Runtime,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    busy: usize,

    profiles: Vec<String>,
    source: Option<Source>,
    connections: Vec<Connection>,
    /// List profiles from the AWS config/credentials files.
    use_aws_config: bool,
    vault: Box<dyn Vault>,
    settings: Option<SettingsUi>,
    pending: Vec<PendingChange>,
    code_lang: Lang,
    logo: Option<egui::TextureHandle>,
    recheck_at: Option<std::time::Instant>,
    /// Empty = use the profile's configured region.
    region: String,
    sm: Option<Sm>,

    secrets: Vec<SecretSummary>,
    search: String,
    show_deleted: bool,
    selected: Option<String>,
    detail: Option<SecretDetail>,
    versions: Vec<VersionInfo>,
    tab: Tab,

    editor: Option<Editor>,
    /// Version id and creation time of the value shown in the editor.
    loaded_version: Option<(String, Option<chrono::DateTime<chrono::Utc>>)>,
    meta_desc: String,
    meta_kms: String,
    tags_edit: Vec<(String, String)>,
    rotation: RotationEdit,

    peek: Option<(String, Option<FetchedValue>)>,
    new_secret: Option<NewSecret>,
    delete: Option<DeleteDialog>,
    confirm: Option<Confirm>,
    status: Option<(String, bool)>,

    appearance: Appearance,
    /// Theme used in dark mode.
    dark_theme: ThemeRef,
    /// Theme used in light mode.
    light_theme: ThemeRef,
    /// Colored buttons, chips, dots and markers from the theme's terminal colors.
    colorful: bool,
    /// Themes found in installed VS Code-family editors.
    installed_themes: Vec<ThemeRef>,
    /// Theme files the user imported.
    imported_themes: Vec<ThemeRef>,
    palette: Palette,
    /// The settings (and OS dark mode) the current palette was built for.
    applied_theme: Option<(Appearance, ThemeRef, ThemeRef, bool, bool)>,
}

impl SecretsApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::offline(cc.egui_ctx.clone());
        if let Some(storage) = cc.storage {
            if let Some(a) = eframe::get_value(storage, "appearance") {
                app.appearance = a;
            }
            if let Some(t) = eframe::get_value(storage, "dark_theme") {
                app.dark_theme = t;
            }
            if let Some(t) = eframe::get_value(storage, "light_theme") {
                app.light_theme = t;
            }
            if let Some(c) = eframe::get_value(storage, "colorful") {
                app.colorful = c;
            }
            if let Some(imported) = eframe::get_value(storage, "imported_themes") {
                app.imported_themes = imported;
            }
            if let Some(lang) = eframe::get_value(storage, "code_lang") {
                app.code_lang = lang;
            }
            if let Some(conns) = eframe::get_value(storage, "connections") {
                app.connections = conns;
            }
            if let Some(use_cfg) = eframe::get_value(storage, "use_aws_config") {
                app.use_aws_config = use_cfg;
            }
            if let Some(source) = eframe::get_value::<Source>(storage, "source")
                && app.source_available(&source)
            {
                app.source = Some(source);
            }
        }
        if app.source.is_none() {
            app.source = app.default_source();
        }
        app.installed_themes = theme::discover_installed();
        if app.source.is_some() {
            app.connect();
        }
        app
    }

    /// App state without connecting to AWS (used directly by tests).
    fn offline(ctx: egui::Context) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, rx) = channel();
        let profiles = list_profiles();
        let region = std::env::var("AWS_REGION").unwrap_or_default();
        Self {
            rt,
            tx,
            rx,
            ctx,
            busy: 0,
            profiles,
            source: None,
            connections: Vec::new(),
            use_aws_config: true,
            vault: Box::new(KeyringVault),
            settings: None,
            pending: Vec::new(),
            code_lang: Lang::JavaScript,
            logo: None,
            recheck_at: None,
            region,
            sm: None,
            secrets: Vec::new(),
            search: String::new(),
            show_deleted: false,
            selected: None,
            detail: None,
            versions: Vec::new(),
            tab: Tab::Value,
            editor: None,
            loaded_version: None,
            meta_desc: String::new(),
            meta_kms: String::new(),
            tags_edit: Vec::new(),
            rotation: RotationEdit::default(),
            peek: None,
            new_secret: None,
            delete: None,
            confirm: None,
            status: None,
            appearance: Appearance::System,
            dark_theme: ThemeRef::default_dark(),
            light_theme: ThemeRef::default_light(),
            colorful: true,
            installed_themes: Vec::new(),
            imported_themes: Vec::new(),
            palette: Palette::dark(),
            applied_theme: None,
        }
    }

    // ----- background work ---------------------------------------------------------------

    fn spawn(&mut self, fut: impl Future<Output = Msg> + Send + 'static) {
        self.busy += 1;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn(async move {
            let msg = fut.await;
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn connect(&mut self) {
        self.sm = None;
        self.secrets.clear();
        self.select(None);
        let Some(source) = self.source.clone() else {
            return;
        };
        let region = Some(self.region.clone()).filter(|r| !r.is_empty());
        let opts = match source {
            Source::Profile(p) => ConnOpts {
                profile: Some(p),
                region,
                endpoint_url: std::env::var("SECRETS_ENDPOINT_URL").ok(),
                ..Default::default()
            },
            Source::Connection(id) => {
                let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() else {
                    self.error("That saved connection no longer exists");
                    return;
                };
                let secret = match self.vault.get(&conn.id) {
                    Ok(Some(s)) => s,
                    Ok(None) => {
                        self.error(format!(
                            "The secret key for {} is missing from the credential store; edit the connection in Settings",
                            conn.name
                        ));
                        return;
                    }
                    Err(e) => return self.error(format!("{e:#}")),
                };
                connection_opts(&conn, &secret, region)
            }
        };
        self.spawn(async move { Msg::Connected(Sm::connect(&opts).await) });
    }

    fn source_available(&self, source: &Source) -> bool {
        match source {
            Source::Profile(p) => self.use_aws_config && self.profiles.contains(p),
            Source::Connection(id) => self.connections.iter().any(|c| &c.id == id),
        }
    }

    /// $AWS_PROFILE, then "default", then the first profile, then the first saved connection.
    fn default_source(&self) -> Option<Source> {
        let profiles = if self.use_aws_config {
            &self.profiles[..]
        } else {
            &[]
        };
        std::env::var("AWS_PROFILE")
            .ok()
            .filter(|p| profiles.contains(p))
            .or_else(|| profiles.iter().find(|p| *p == "default").cloned())
            .or_else(|| profiles.first().cloned())
            .map(Source::Profile)
            .or_else(|| {
                self.connections
                    .first()
                    .map(|c| Source::Connection(c.id.clone()))
            })
    }

    fn source_label(&self) -> String {
        match &self.source {
            Some(Source::Profile(p)) => p.clone(),
            Some(Source::Connection(id)) => self
                .connections
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "(missing connection)".into()),
            None => "Not connected".into(),
        }
    }

    fn use_source(&mut self, source: Source) {
        self.region = match &source {
            Source::Connection(id) => self
                .connections
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.region.clone())
                .unwrap_or_default(),
            // Let the profile's own region apply.
            Source::Profile(_) => String::new(),
        };
        self.source = Some(source);
        self.connect();
    }

    /// Make the list reflect recent creates/deletes that ListSecrets doesn't show yet, and
    /// schedule a re-check until it does (or [`PENDING_FOR`] passes).
    fn apply_pending(&mut self) {
        let now = std::time::Instant::now();
        self.pending
            .retain(|p| now.duration_since(p.since) < PENDING_FOR);
        let mut lagging = false;
        for p in &self.pending {
            let pos = self.secrets.iter().position(|s| s.name == p.name);
            match (p.change, pos) {
                (Change::Created | Change::Restored, None) => {
                    lagging = true;
                    self.secrets.push(SecretSummary {
                        name: p.name.clone(),
                        description: None,
                        last_changed: Some(chrono::Utc::now()),
                        deleted: None,
                        rotation_enabled: false,
                        tags: Vec::new(),
                    });
                }
                (Change::Restored, Some(i)) if self.secrets[i].deleted.is_some() => {
                    lagging = true;
                    self.secrets[i].deleted = None;
                }
                (Change::Deleted, Some(i)) if !self.show_deleted => {
                    lagging = true;
                    self.secrets.remove(i);
                }
                (Change::Deleted, Some(i)) if self.secrets[i].deleted.is_none() => {
                    lagging = true;
                    self.secrets[i].deleted = Some(chrono::Utc::now());
                }
                _ => {}
            }
        }
        // Once AWS has caught up with a change, stop tracking it.
        if !lagging {
            self.pending.clear();
        }
        self.secrets.sort_by_key(|s| s.name.to_lowercase());
        self.recheck_at = lagging.then(|| now + RECHECK_EVERY);
    }

    /// Manual refresh: the list, the selected secret's details, and its value (unless it has
    /// unsaved edits).
    fn refresh(&mut self) {
        if self.sm.is_none() {
            self.connect();
            return;
        }
        if !self.editor_dirty() {
            self.editor = None;
        }
        self.reload_list();
        self.status = Some(("Refreshed".into(), false));
    }

    fn editor_dirty(&self) -> bool {
        match &self.editor {
            Some(Editor::Text { text, original }) => !same_value(text, original),
            Some(Editor::Kv { rows, original }) => {
                kv_to_json(rows).map_or(true, |j| !same_value(&j, original))
            }
            Some(Editor::Binary(_)) | None => false,
        }
    }

    fn reload_list(&mut self) {
        let Some(sm) = self.sm.clone() else { return };
        let filter = ListFilter {
            include_deleted: self.show_deleted,
            ..Default::default()
        };
        self.spawn(async move { Msg::Listed(sm.list(&filter).await) });
    }

    fn load_detail(&mut self) {
        let (Some(sm), Some(id)) = (self.sm.clone(), self.selected.clone()) else {
            return;
        };
        self.spawn(async move {
            let res = async {
                let d = sm.describe(&id).await?;
                let v = sm.versions(&id, true).await?;
                Ok(Box::new((d, v)))
            }
            .await;
            Msg::Detail(id, res)
        });
        // Keep any unsaved edits on refresh; otherwise load the current value too.
        if self.editor.is_none() {
            self.fetch_value();
        }
    }

    fn fetch_value(&mut self) {
        let (Some(sm), Some(id)) = (self.sm.clone(), self.selected.clone()) else {
            return;
        };
        self.spawn(async move {
            let res = match sm.get_value(&id, None, None).await {
                Ok(v) => Ok(Some(v)),
                // A secret created without a value has no AWSCURRENT version yet.
                Err(e) if e.to_string().contains("ResourceNotFoundException") => Ok(None),
                Err(e) => Err(e),
            };
            Msg::Value(id, res)
        });
    }

    fn peek_version(&mut self, version: String) {
        let (Some(sm), Some(id)) = (self.sm.clone(), self.selected.clone()) else {
            return;
        };
        self.peek = Some((version.clone(), None));
        self.spawn(async move {
            let res = sm.get_value(&id, Some(&version), None).await;
            Msg::Peek(id, version, res)
        });
    }

    fn run(&mut self, action: Action) {
        let Some(sm) = self.sm.clone() else { return };
        let change = match &action {
            Action::Create(_) => Change::Created,
            Action::Delete(..) => Change::Deleted,
            Action::Restore(_) => Change::Restored,
            _ => Change::Other,
        };
        let (id, fut): (
            String,
            std::pin::Pin<Box<dyn Future<Output = Result<String>> + Send>>,
        ) = match action {
            Action::Put(id, value) => (
                id.clone(),
                Box::pin(async move {
                    let v = sm.put_value(&id, &value, &[]).await?;
                    Ok(format!("Saved new version {}", v.unwrap_or_default()))
                }),
            ),
            Action::Metadata { id, desc, kms } => (
                id.clone(),
                Box::pin(async move {
                    sm.update_metadata(&id, desc.as_deref(), kms.as_deref())
                        .await?;
                    Ok(format!("Updated {id}"))
                }),
            ),
            Action::Tags { id, set, remove } => (
                id.clone(),
                Box::pin(async move {
                    sm.untag(&id, &remove).await?;
                    sm.tag(&id, &set).await?;
                    Ok(format!("Updated tags on {id}"))
                }),
            ),
            Action::Delete(id, how) => (
                id.clone(),
                Box::pin(async move {
                    Ok(match sm.delete(&id, how).await? {
                        Some(t) => format!(
                            "{id} is scheduled for deletion on {}. Tick \"Show deleted\" to restore it.",
                            fmt_time(Some(t))
                        ),
                        None => format!("Deleted {id}"),
                    })
                }),
            ),
            Action::Restore(id) => (
                id.clone(),
                Box::pin(async move {
                    sm.restore(&id).await?;
                    Ok(format!("Restored {id}"))
                }),
            ),
            Action::Rotate(id, cfg) => (
                id.clone(),
                Box::pin(async move {
                    sm.rotate(&id, &cfg).await?;
                    Ok(if cfg.rotate_immediately {
                        format!("Rotation started for {id}")
                    } else {
                        format!("Rotation settings saved for {id}")
                    })
                }),
            ),
            Action::CancelRotation(id) => (
                id.clone(),
                Box::pin(async move {
                    sm.cancel_rotation(&id).await?;
                    Ok(format!("Rotation disabled for {id}"))
                }),
            ),
            Action::Promote(id, version) => (
                id.clone(),
                Box::pin(async move {
                    sm.promote(&id, &version).await?;
                    Ok(format!("Version {version} is now current"))
                }),
            ),
            Action::Create(n) => {
                let id = n.name.trim().to_string();
                (
                    id.clone(),
                    Box::pin(async move {
                        let value = match n.kind {
                            NewKind::Empty => None,
                            NewKind::Text => Some(SecretValue::Text(compact_if_json(&n.text))),
                            NewKind::KeyValue => Some(SecretValue::Text(kv_to_json(&n.rows)?)),
                        };
                        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
                        sm.create(
                            &id,
                            opt(&n.description).as_deref(),
                            opt(&n.kms).as_deref(),
                            &n.tags,
                            value.as_ref(),
                        )
                        .await?;
                        Ok(format!("Created {id}"))
                    }),
                )
            }
        };
        self.spawn(async move { Msg::Done(id, change, fut.await) });
    }

    fn select(&mut self, id: Option<String>) {
        if self.selected == id {
            return;
        }
        self.selected = id;
        self.detail = None;
        self.versions.clear();
        self.editor = None;
        self.loaded_version = None;
        self.peek = None;
        if self.selected.is_some() {
            self.load_detail();
        }
    }

    fn error(&mut self, e: impl std::fmt::Display) {
        self.status = Some((e.to_string(), true));
    }

    fn handle(&mut self, msg: Msg) {
        self.busy = self.busy.saturating_sub(1);
        match msg {
            Msg::Connected(Ok(sm)) => {
                self.region = sm.region.clone();
                self.sm = Some(sm);
                self.status = None;
                self.reload_list();
            }
            Msg::Connected(Err(e)) => self.error(format!("{e:#}")),
            Msg::Listed(Ok(list)) => {
                self.secrets = list;
                self.apply_pending();
                if let Some(sel) = &self.selected {
                    if self.secrets.iter().any(|s| &s.name == sel) {
                        self.load_detail();
                    } else {
                        self.select(None);
                    }
                }
            }
            Msg::Listed(Err(e)) => self.error(format!("{e:#}")),
            Msg::Detail(id, res) if Some(&id) == self.selected.as_ref() => match res.map(|b| *b) {
                Ok((d, v)) => {
                    self.meta_desc = d.description.clone().unwrap_or_default();
                    self.meta_kms = d.kms_key_id.clone().unwrap_or_default();
                    self.tags_edit = d.tags.clone();
                    self.rotation = RotationEdit {
                        lambda: d.rotation_lambda.clone().unwrap_or_default(),
                        use_schedule: d.rotation_schedule.is_some(),
                        days: d.rotation_days.map(|n| n.to_string()).unwrap_or_default(),
                        schedule: d.rotation_schedule.clone().unwrap_or_default(),
                        window: d.rotation_window.clone().unwrap_or_default(),
                        rotate_now: !d.rotation_enabled,
                    };
                    self.detail = Some(d);
                    self.versions = v;
                }
                Err(e) => self.error(format!("{e:#}")),
            },
            Msg::Value(id, res) if Some(&id) == self.selected.as_ref() => match res {
                Ok(v) => {
                    self.loaded_version = v
                        .as_ref()
                        .and_then(|v| v.version_id.clone().map(|id| (id, v.created)));
                    self.editor = Some(match v.map(|v| v.value) {
                        Some(SecretValue::Binary(b)) => Editor::Binary(b),
                        Some(SecretValue::Text(s)) => match json_object(&s) {
                            Some(map) => Editor::Kv {
                                rows: kv_rows(&map),
                                original: s,
                            },
                            None => Editor::Text {
                                text: format_json(&s).unwrap_or_else(|| s.clone()),
                                original: s,
                            },
                        },
                        None => Editor::Text {
                            text: String::new(),
                            original: String::new(),
                        },
                    });
                }
                Err(e) => self.error(format!("{e:#}")),
            },
            Msg::Peek(id, version, res) if Some(&id) == self.selected.as_ref() => match res {
                Ok(v) => self.peek = Some((version, Some(v))),
                Err(e) => {
                    self.peek = None;
                    self.error(format!("{e:#}"));
                }
            },
            Msg::Done(id, change, res) => match res {
                Ok(msg) => {
                    self.status = Some((msg, false));
                    self.editor = None;
                    self.peek = None;
                    if change != Change::Other {
                        self.pending.retain(|p| p.name != id);
                        self.pending.push(PendingChange {
                            name: id.clone(),
                            change,
                            since: std::time::Instant::now(),
                        });
                        self.apply_pending();
                    }
                    let hidden = change == Change::Deleted && !self.show_deleted;
                    if hidden {
                        if self.selected.as_ref() == Some(&id) {
                            self.select(None);
                        }
                    } else if self.selected.as_ref() != Some(&id) {
                        self.select(Some(id));
                    }
                    self.reload_list();
                }
                Err(e) => self.error(format!("{e:#}")),
            },
            Msg::Password(Ok(p)) => {
                if let Some(n) = &mut self.new_secret {
                    match n.kind {
                        NewKind::Text => n.text = p,
                        _ => {
                            n.kind = NewKind::KeyValue;
                            n.rows.push(KvRow {
                                key: "password".into(),
                                text: p,
                                original: None,
                            });
                        }
                    }
                } else if let Some(Editor::Text { text, .. }) = &mut self.editor {
                    *text = p;
                }
            }
            Msg::Password(Err(e)) => self.error(format!("{e:#}")),
            Msg::Tested(res) => {
                if let Some(st) = &mut self.settings {
                    st.testing = false;
                    st.test_result = Some(match res {
                        Ok(n) => (
                            true,
                            format!("Connected. {n} secret(s) visible on the first page."),
                        ),
                        Err(e) => (false, format!("{e:#}").replace('\n', " ")),
                    });
                }
            }
            // Responses for a secret that is no longer selected are dropped.
            Msg::Detail(..) | Msg::Value(..) | Msg::Peek(..) => {}
        }
    }

    fn generate_password(&mut self) {
        let Some(sm) = self.sm.clone() else { return };
        self.spawn(async move { Msg::Password(sm.random_password(32, false).await) });
    }

    // ----- layout ------------------------------------------------------------------------

    /// Whether dark mode is in effect.
    fn is_dark(&self, ctx: &egui::Context) -> bool {
        match self.appearance {
            Appearance::Dark => true,
            Appearance::Light => false,
            Appearance::System => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
        }
    }

    /// Rebuild the style when the theme settings (or, for System, the OS appearance) change.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let dark = self.is_dark(ctx);
        let key = (
            self.appearance,
            self.dark_theme.clone(),
            self.light_theme.clone(),
            self.colorful,
            dark,
        );
        if self.applied_theme.as_ref() == Some(&key) {
            return;
        }
        let chosen = if dark {
            &self.dark_theme
        } else {
            &self.light_theme
        };
        let mut palette = match chosen.load() {
            Ok(t) => t.palette(),
            Err(e) => {
                self.error(format!("{e:#}"));
                if dark {
                    Palette::dark()
                } else {
                    Palette::light()
                }
            }
        };
        palette.colorful = self.colorful;
        self.palette = palette;
        apply_style(ctx, &self.palette);
        self.applied_theme = Some(key);
    }

    fn import_theme(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import a VS Code color theme")
            .add_filter("VS Code color theme", &["json", "jsonc"])
            .pick_file()
        else {
            return;
        };
        match VsCodeTheme::load(&path) {
            Ok(t) => {
                let r = ThemeRef::File {
                    label: t.name.clone(),
                    path,
                    dark: t.dark,
                };
                self.imported_themes.retain(|c| c != &r);
                self.imported_themes.push(r.clone());
                let mode = if t.dark { "dark" } else { "light" };
                if t.dark {
                    self.dark_theme = r;
                } else {
                    self.light_theme = r;
                }
                self.status = Some((format!("Using {} as your {mode} theme", t.name), false));
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// All themes of one type: preloaded, then imported, then installed in VS Code.
    fn themes_for(&self, dark: bool) -> Vec<(ThemeRef, &'static str)> {
        let mut out: Vec<(ThemeRef, &'static str)> = ThemeRef::bundled()
            .filter(|t| t.dark() == dark)
            .map(|t| (t, "Preloaded"))
            .collect();
        out.extend(
            self.imported_themes
                .iter()
                .filter(|t| t.dark() == dark)
                .map(|t| (t.clone(), "Imported")),
        );
        out.extend(
            self.installed_themes
                .iter()
                .filter(|t| t.dark() == dark)
                .map(|t| (t.clone(), "Installed in VS Code")),
        );
        out
    }

    /// A dropdown of themes of one type, grouped by where they come from.
    fn theme_combo(&mut self, ui: &mut Ui, dark: bool, id: &str) -> egui::Response {
        let themes = self.themes_for(dark);
        let current = if dark {
            self.dark_theme.clone()
        } else {
            self.light_theme.clone()
        };
        let mut picked = None;
        let resp = ComboBox::from_id_salt(id)
            .selected_text(current.label().to_string())
            .width(240.0)
            .height(420.0)
            .show_ui(ui, |ui| {
                let mut group = "";
                for (t, g) in &themes {
                    if *g != group {
                        if !group.is_empty() {
                            ui.separator();
                        }
                        ui.weak(*g);
                        group = g;
                    }
                    if ui.selectable_label(*t == current, t.label()).clicked() {
                        picked = Some(t.clone());
                    }
                }
            })
            .response;
        if let Some(t) = picked {
            if dark {
                self.dark_theme = t;
            } else {
                self.light_theme = t;
            }
        }
        resp
    }

    /// Status bar control: System / Light / Dark.
    fn theme_picker(&mut self, ui: &mut Ui) {
        // Laid out right-to-left, so the last button is added first.
        let fg = self.palette.status_fg;
        for (a, label) in [
            (Appearance::Dark, "Dark"),
            (Appearance::Light, "Light"),
            (Appearance::System, "System"),
        ] {
            let selected = self.appearance == a;
            let text = RichText::new(label).color(if selected {
                self.palette.selection_fg
            } else {
                fg
            });
            if ui
                .add(egui::Button::selectable(selected, text))
                .on_hover_text("Appearance (choose themes in Settings → Appearance)")
                .clicked()
            {
                self.appearance = a;
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let title_fg = self.palette.titlebar_fg;
            let logo = self
                .logo
                .get_or_insert_with(|| {
                    let icon =
                        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                            .expect("bundled icon is a valid PNG");
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [icon.width as usize, icon.height as usize],
                        &icon.rgba,
                    );
                    ui.ctx()
                        .load_texture("logo", image, egui::TextureOptions::LINEAR)
                })
                .clone();
            ui.add(egui::Image::new((logo.id(), egui::vec2(30.0, 30.0))).alt_text("Secrets"));
            ui.add_space(-6.0);
            ui.label(RichText::new("Secrets").strong().size(17.0).color(title_fg));
            ui.separator();

            let mut pick: Option<Source> = None;
            let mut manage = false;
            let label = ui.label(RichText::new("Account").color(title_fg));
            ComboBox::from_id_salt("source")
                .selected_text(self.source_label())
                .width(200.0)
                .show_ui(ui, |ui| {
                    if !self.connections.is_empty() {
                        ui.weak("Saved connections");
                        for c in &self.connections {
                            let src = Source::Connection(c.id.clone());
                            let selected = self.source.as_ref() == Some(&src);
                            if ui.selectable_label(selected, &c.name).clicked() {
                                pick = Some(src);
                            }
                        }
                    }
                    if self.use_aws_config && !self.profiles.is_empty() {
                        if !self.connections.is_empty() {
                            ui.separator();
                        }
                        ui.weak("AWS config profiles");
                        for p in &self.profiles {
                            let src = Source::Profile(p.clone());
                            let selected = self.source.as_ref() == Some(&src);
                            if ui.selectable_label(selected, p).clicked() {
                                pick = Some(src);
                            }
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(false, "Manage connections…").clicked() {
                        manage = true;
                    }
                })
                .response
                .labelled_by(label.id);
            if manage {
                self.settings = Some(SettingsUi::new(SettingsPage::Connections));
            }
            let mut region = self.region.clone();
            let label = ui.label(RichText::new("Region").color(title_fg));
            ComboBox::from_id_salt("region")
                .selected_text(if region.is_empty() {
                    "(default)"
                } else {
                    &region
                })
                .show_ui(ui, |ui| {
                    if !REGIONS.contains(&region.as_str()) && !region.is_empty() {
                        let cur = region.clone();
                        ui.selectable_value(&mut region, cur.clone(), cur);
                    }
                    for r in REGIONS {
                        ui.selectable_value(&mut region, r.to_string(), *r);
                    }
                })
                .response
                .labelled_by(label.id);
            if let Some(src) = pick {
                if self.source.as_ref() != Some(&src) {
                    self.use_source(src);
                }
            } else if region != self.region && self.source.is_some() {
                self.region = region;
                self.connect();
            }

            ui.separator();
            if ui
                .checkbox(&mut self.show_deleted, "Show deleted")
                .changed()
            {
                self.reload_list();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.settings = Some(SettingsUi::new(SettingsPage::Connections));
                }
                let enabled = self.sm.is_some();
                if ui
                    .add_enabled(enabled, tone_button(ui, "+ New secret", Tone::Create))
                    .clicked()
                {
                    self.new_secret = Some(NewSecret {
                        rows: vec![KvRow {
                            key: String::new(),
                            text: String::new(),
                            original: None,
                        }],
                        ..Default::default()
                    });
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let fg = self.palette.status_fg;
        ui.horizontal(|ui| {
            if self.busy > 0 {
                ui.add(egui::Spinner::new().color(fg));
            }
            // The status bar may be brightly colored (many VS Code themes), so messages use
            // its own foreground with a colored marker instead of colored text.
            let dot = |ui: &mut Ui, color: Color32| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, color);
            };
            match &self.status {
                Some((msg, true)) => {
                    let msg = msg.clone();
                    dot(ui, self.palette.error);
                    ui.label(RichText::new(msg).color(fg));
                    if ui.button("Dismiss").clicked() {
                        self.status = None;
                    }
                }
                Some((msg, false)) => {
                    dot(ui, self.palette.success);
                    ui.label(RichText::new(msg).color(fg));
                }
                None if self.sm.is_none() && self.busy > 0 => {
                    ui.label(
                        RichText::new(format!("Connecting to {}…", self.source_label())).color(fg),
                    );
                }
                None => {
                    ui.label(
                        RichText::new(format!(
                            "{} secrets · {} · {}",
                            self.secrets.len(),
                            self.source_label(),
                            self.region
                        ))
                        .color(fg),
                    );
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.theme_picker(ui);
            });
        });
    }

    fn secret_list(&mut self, ui: &mut Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let refresh_w = 86.0;
            // Exact sizes: anything wider makes the resizable panel grow every frame.
            let search_w = ui.available_width() - refresh_w - ui.spacing().item_spacing.x;
            ui.add_sized(
                [search_w, ROW_H],
                TextEdit::singleline(&mut self.search)
                    .margin(FIELD_MARGIN)
                    .hint_text("Search secrets"),
            );
            let shortcut = if cfg!(target_os = "macos") {
                "Cmd+R"
            } else {
                "Ctrl+R"
            };
            if ui
                .add_sized([refresh_w, ROW_H], egui::Button::new("Refresh"))
                .on_hover_text(format!(
                    "Reload the list and the selected secret ({shortcut} or F5)"
                ))
                .clicked()
            {
                self.refresh();
            }
        });
        ui.add_space(8.0);
        let mut clicked = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                for s in self
                    .secrets
                    .iter()
                    .filter(|s| matches_query(s, &self.search))
                {
                    let selected = self.selected.as_deref() == Some(s.name.as_str());
                    let colorful = self.palette.colorful;
                    let mut text = RichText::new(&s.name);
                    if s.deleted.is_some() {
                        text = text.color(red(ui)).strikethrough();
                    }
                    // With colorful accents, a fixed slot holds the colored dot so long
                    // names wrap beside it rather than underneath it.
                    let dot_id = egui::IdSalt::new("secret_dot");
                    let mut atoms = egui::Atoms::default();
                    if colorful {
                        atoms.push_right(egui::Atom::custom(dot_id, egui::vec2(10.0, 10.0)));
                    }
                    atoms.push_right(text);
                    atoms.push_right(egui::Atom::grow());
                    let resp = egui::Button::selectable(selected, atoms)
                        .min_size(egui::vec2(0.0, 34.0))
                        .atom_ui(ui);
                    let dot_rect = resp.rect(dot_id);
                    let resp = resp
                        .response
                        .on_hover_text(format!(
                            "{}\nLast changed {}{}",
                            s.description.as_deref().unwrap_or(&s.name),
                            fmt_time(s.last_changed),
                            if s.rotation_enabled {
                                "\nRotation enabled"
                            } else {
                                ""
                            }
                        ));
                    if let Some(r) = dot_rect {
                        let dot = if s.deleted.is_some() {
                            red(ui)
                        } else {
                            name_color(&s.name, &self.palette)
                        };
                        ui.painter().circle_filled(r.center(), 5.0, dot);
                    }
                    if resp.clicked() {
                        clicked = Some(s.name.clone());
                    }
                }
                if self.secrets.is_empty() && self.sm.is_some() && self.busy == 0 {
                    ui.weak("No secrets in this region.");
                }
            });
        });
        if clicked.is_some() {
            self.select(clicked);
        }
    }

    fn details(&mut self, ui: &mut Ui) {
        if self.source.is_none() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.heading(RichText::new("Welcome to Secrets").strong());
                ui.add_space(12.0);
                ui.label("Add your AWS access keys to get started.");
                ui.weak(
                    "Already use the AWS CLI? Profiles from ~/.aws/config appear automatically.",
                );
                ui.add_space(20.0);
                if ui
                    .add(tone_button(ui, "Add a connection", Tone::Create))
                    .clicked()
                {
                    let mut st = SettingsUi::new(SettingsPage::Connections);
                    st.editor = Some(new_editor());
                    self.settings = Some(st);
                }
            });
            return;
        }
        let Some(id) = self.selected.clone() else {
            ui.centered_and_justified(|ui| {
                ui.weak("Select a secret on the left, or create a new one.");
            });
            return;
        };
        let summary = self.secrets.iter().find(|s| s.name == id).cloned();

        ui.horizontal(|ui| {
            if self.palette.colorful {
                marker(ui, name_color(&id, &self.palette));
            }
            ui.heading(RichText::new(&id).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let deleted = summary.as_ref().is_some_and(|s| s.deleted.is_some());
                if deleted {
                    if ui.add(tone_button(ui, "Restore", Tone::Create)).clicked() {
                        self.run(Action::Restore(id.clone()));
                    }
                } else if ui.button(RichText::new("Delete…").color(red(ui))).clicked() {
                    self.delete = Some(DeleteDialog {
                        id: id.clone(),
                        days: 30,
                        force: false,
                        typed: String::new(),
                    });
                }
            });
        });
        if let Some(when) = summary.as_ref().and_then(|s| s.deleted) {
            ui.label(
                RichText::new(format!(
                    "Scheduled for deletion on {}. Restore it to use it again.",
                    fmt_time(Some(when))
                ))
                .color(red(ui)),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Value, "Value"),
                (Tab::Overview, "Overview"),
                (Tab::Versions, "Versions"),
                (Tab::Tags, "Tags"),
                (Tab::Rotation, "Rotation"),
                (Tab::Code, "Code"),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
        ui.separator();

        let Some(detail) = self.detail.clone() else {
            ui.spinner();
            return;
        };
        ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::Overview => self.overview_tab(ui, &detail),
                Tab::Value => self.value_tab(ui, &id),
                Tab::Versions => self.versions_tab(ui),
                Tab::Tags => self.tags_tab(ui, &detail),
                Tab::Rotation => self.rotation_tab(ui, &detail),
                Tab::Code => self.code_tab(ui, &detail),
            });
    }

    fn overview_tab(&mut self, ui: &mut Ui, d: &SecretDetail) {
        Grid::new("overview")
            .num_columns(2)
            .spacing([32.0, 14.0])
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in detail_lines(d) {
                    if k == "Description" || k == "KMS key" {
                        continue;
                    }
                    ui.strong(k.as_str());
                    if k == "ARN" {
                        ui.horizontal(|ui| {
                            ui.monospace(&v);
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(v.clone());
                            }
                        });
                    } else {
                        ui.label(v);
                    }
                    ui.end_row();
                }
            });
        ui.add_space(24.0);
        ui.strong("Settings");
        Grid::new("meta")
            .num_columns(2)
            .spacing([32.0, 14.0])
            .show(ui, |ui| {
                ui.label("Description");
                ui.add(
                    TextEdit::singleline(&mut self.meta_desc)
                        .margin(FIELD_MARGIN)
                        .hint_text("Description")
                        .desired_width(420.0),
                );
                ui.end_row();
                ui.label("KMS key");
                ui.add(
                    TextEdit::singleline(&mut self.meta_kms)
                        .margin(FIELD_MARGIN)
                        .hint_text("aws/secretsmanager (default)")
                        .desired_width(420.0),
                );
                ui.end_row();
            });
        let old_desc = d.description.clone().unwrap_or_default();
        let old_kms = d.kms_key_id.clone().unwrap_or_default();
        let changed = self.meta_desc != old_desc || self.meta_kms.trim() != old_kms;
        ui.horizontal(|ui| {
            if ui.add_enabled(changed, tone_button(ui, "Save settings", Tone::Save)).clicked() {
                if self.meta_kms.trim().is_empty() && !old_kms.is_empty() {
                    self.error("The KMS key can't be cleared; enter aws/secretsmanager to use the default key");
                } else {
                    self.run(Action::Metadata {
                        id: d.name.clone(),
                        desc: (self.meta_desc != old_desc).then(|| self.meta_desc.clone()),
                        kms: (self.meta_kms.trim() != old_kms).then(|| self.meta_kms.trim().to_string()),
                    });
                }
            }
            if changed && ui.button("Revert").clicked() {
                self.meta_desc = old_desc;
                self.meta_kms = old_kms;
            }
        });
    }

    fn value_tab(&mut self, ui: &mut Ui, id: &str) {
        let syntax = self.palette.syntax.clone();
        let Some(editor) = &mut self.editor else {
            ui.add_space(16.0);
            if self.busy > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading value…");
                });
            } else {
                ui.label("The value couldn't be loaded (see the status bar).");
                if ui.button("Retry").clicked() {
                    self.fetch_value();
                }
            }
            return;
        };

        let version_note = self
            .loaded_version
            .as_ref()
            .map(|(v, t)| format!(" (version {v}, created {})", fmt_time(*t)))
            .unwrap_or_default();
        let mut save: Option<SecretValue> = None;
        let mut to_text: Option<String> = None;
        let mut gen_password = false;
        match editor {
            Editor::Binary(bytes) => {
                ui.label(format!(
                    "Binary secret, {} bytes (shown as base64)",
                    bytes.len()
                ));
                let mut b = b64(bytes);
                ui.add(
                    TextEdit::multiline(&mut b)
                        .margin(FIELD_MARGIN)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
                ui.horizontal(|ui| {
                    if ui.button("Copy base64").clicked() {
                        ui.ctx().copy_text(b64(bytes));
                    }
                });
            }
            Editor::Text { text, original } => {
                let is_json = looks_like_json(text);
                let kind = if is_json {
                    "JSON value"
                } else {
                    "Plain text value"
                };
                ui.weak(format!(
                    "{kind}{version_note}. Saving creates a new version, which becomes current."
                ));
                code_editor(ui, text, &syntax, 12, true);
                let parsed = is_json.then(|| serde_json::from_str::<serde_json::Value>(text));
                if let Some(Err(e)) = &parsed {
                    ui.label(RichText::new(format!("Not valid JSON: {e}")).color(red(ui)));
                }
                let changed = !same_value(text, original);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(changed, tone_button(ui, "Save new version", Tone::Save))
                        .on_hover_text("JSON is saved compactly, the way the AWS console stores it")
                        .clicked()
                    {
                        save = Some(SecretValue::Text(compact_if_json(text)));
                    }
                    if changed && ui.button("Discard changes").clicked() {
                        *text = format_json(original).unwrap_or_else(|| original.clone());
                    }
                    if let Some(pretty) = format_json(text).filter(|p| p != text)
                        && ui.button("Format JSON").clicked()
                    {
                        *text = pretty;
                    }
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(compact_if_json(text));
                    }
                    if !is_json && ui.button("Generate password").clicked() {
                        gen_password = true;
                    }
                });
                if !changed && text.trim().is_empty() {
                    ui.weak("This secret has no value yet. Enter one and save.");
                }
                if json_object(text).is_some() && ui.link("Edit as key/value pairs").clicked() {
                    to_text = Some(text.clone());
                }
            }
            Editor::Kv { rows, original } => {
                ui.weak(format!("Key/value secret{version_note}. Saving creates a new version, which becomes current."));
                let mut remove = None;
                let (kw, vw) = row_widths(ui.available_width());
                Grid::new("kv")
                    .num_columns(4)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        ui.strong("Key");
                        ui.strong("Value");
                        ui.end_row();
                        for (i, row) in rows.iter_mut().enumerate() {
                            ui.add_sized(
                                [kw, ROW_H],
                                TextEdit::singleline(&mut row.key).margin(FIELD_MARGIN),
                            );
                            ui.add_sized(
                                [vw, ROW_H],
                                TextEdit::singleline(&mut row.text).margin(FIELD_MARGIN),
                            );
                            if ui.button("Copy").on_hover_text("Copy this value").clicked() {
                                ui.ctx().copy_text(row.text.clone());
                            }
                            if ui.button("Remove").on_hover_text("Remove key").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(i) = remove {
                    rows.remove(i);
                }
                let json = kv_to_json(rows);
                let changed = match &json {
                    Ok(j) => !same_value(j, original),
                    Err(_) => true,
                };
                if let Err(e) = &json {
                    ui.label(RichText::new(e.to_string()).color(red(ui)));
                }
                ui.horizontal(|ui| {
                    if ui.button("+ Add key").clicked() {
                        rows.push(KvRow {
                            key: String::new(),
                            text: String::new(),
                            original: None,
                        });
                    }
                    if ui
                        .add_enabled(
                            changed && json.is_ok(),
                            tone_button(ui, "Save new version", Tone::Save),
                        )
                        .clicked()
                        && let Ok(j) = &json
                    {
                        save = Some(SecretValue::Text(j.clone()));
                    }
                    if changed
                        && ui.button("Discard changes").clicked()
                        && let Some(map) = json_object(original)
                    {
                        *rows = kv_rows(&map);
                    }
                    if ui.button("Copy JSON").clicked() {
                        ui.ctx().copy_text(original.clone());
                    }
                });
                if ui.link("Edit as raw JSON").clicked() {
                    to_text = Some(match &json {
                        Ok(j) => crate::util::pretty_value(j),
                        Err(_) => crate::util::pretty_value(original),
                    });
                }
            }
        }
        if let Some(text) = to_text {
            let original = match &self.editor {
                Some(Editor::Kv { original, .. } | Editor::Text { original, .. }) => {
                    original.clone()
                }
                _ => String::new(),
            };
            self.editor = Some(match json_object(&text) {
                Some(map) if matches!(self.editor, Some(Editor::Text { .. })) => Editor::Kv {
                    rows: kv_rows(&map),
                    original,
                },
                _ => Editor::Text { text, original },
            });
        }
        if gen_password {
            self.generate_password();
        }
        if let Some(v) = save {
            self.run(Action::Put(id.to_string(), v));
        }
    }

    /// What the selected secret holds, from the loaded value (keys for JSON secrets).
    fn value_shape(&self) -> Shape {
        match &self.editor {
            Some(Editor::Kv { rows, .. }) => Shape::Json(
                rows.iter()
                    .filter(|r| !r.key.trim().is_empty())
                    .map(|r| {
                        let v = r
                            .original
                            .clone()
                            .unwrap_or_else(|| serde_json::Value::String(r.text.clone()));
                        (r.key.trim().to_string(), v)
                    })
                    .collect(),
            ),
            Some(Editor::Text { text, .. }) => match json_object(text) {
                Some(map) => Shape::Json(map.into_iter().collect()),
                None => Shape::Text,
            },
            Some(Editor::Binary(_)) => Shape::Binary,
            None => Shape::Text,
        }
    }

    fn code_tab(&mut self, ui: &mut Ui, d: &SecretDetail) {
        let syntax = self.palette.syntax.clone();
        let region = if self.region.is_empty() {
            self.sm
                .as_ref()
                .map(|s| s.region.clone())
                .unwrap_or_default()
        } else {
            self.region.clone()
        };
        let shape = self.value_shape();
        ui.weak(
            "Read this secret with the official AWS SDK. Credentials come from the default \
             AWS credential chain: environment variables, ~/.aws, or an IAM role.",
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for lang in Lang::ALL {
                let b = egui::Button::selectable(self.code_lang == lang, lang.label())
                    .min_size(egui::vec2(0.0, 32.0));
                if ui.add(b).clicked() {
                    self.code_lang = lang;
                }
            }
        });
        ui.add_space(8.0);

        let lang = self.code_lang;
        let section = |ui: &mut Ui, title: &str, text: &str, hl: Option<Lang>, id: &str| {
            ui.horizontal(|ui| {
                ui.strong(title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(text.to_string());
                    }
                });
            });
            let mut t = text.to_string();
            ui.push_id(id, |ui| {
                code_view(ui, &mut t, &syntax, 1, false, hl);
            });
        };
        section(ui, "Install", lang.install(), None, "install");
        ui.add_space(8.0);
        let code = crate::samples::sample(lang, &d.name, &region, &shape);
        section(ui, lang.label(), &code, Some(lang), "code");
        if matches!(&shape, Shape::Json(k) if !k.is_empty()) {
            ui.weak("The sample reads one key of this JSON secret; the others work the same way.");
        }
    }

    fn versions_tab(&mut self, ui: &mut Ui) {
        let mut peek = None;
        let mut promote = None;
        Grid::new("versions")
            .num_columns(5)
            .spacing([32.0, 14.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Version");
                ui.strong("Labels");
                ui.strong("Created");
                ui.strong("Last accessed");
                ui.end_row();
                for v in &self.versions {
                    let current = v.stages.iter().any(|s| s == "AWSCURRENT");
                    ui.monospace(&v.id);
                    if v.stages.is_empty() {
                        ui.weak("(deprecated)");
                    } else {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for stage in &v.stages {
                                let a = theme::palette(ui).ansi;
                                let color = match stage.as_str() {
                                    "AWSCURRENT" => a.green,
                                    "AWSPREVIOUS" => a.yellow,
                                    "AWSPENDING" => a.magenta,
                                    _ => a.blue,
                                };
                                chip(ui, stage, color);
                            }
                        });
                    }
                    ui.label(fmt_time(v.created));
                    ui.label(fmt_time(v.last_accessed));
                    ui.horizontal(|ui| {
                        if ui.button("View value").clicked() {
                            peek = Some(v.id.clone());
                        }
                        if !current && ui.button("Make current").clicked() {
                            promote = Some(v.id.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        ui.add_space(16.0);
        ui.weak("\"Make current\" moves the AWSCURRENT label to that version (a rollback). The previous current version becomes AWSPREVIOUS.");
        if let Some(v) = peek {
            self.peek_version(v);
        }
        if let (Some(v), Some(id)) = (promote, self.selected.clone()) {
            self.confirm = Some(Confirm {
                title: "Make version current".into(),
                body: format!("Make version {v} the current (AWSCURRENT) value of {id}?"),
                button: "Make current".into(),
                action: Action::Promote(id, v),
            });
        }
    }

    fn tags_tab(&mut self, ui: &mut Ui, d: &SecretDetail) {
        let mut remove = None;
        let (kw, vw) = row_widths(ui.available_width());
        Grid::new("tags")
            .num_columns(3)
            .spacing([12.0, 10.0])
            .show(ui, |ui| {
                ui.strong("Key");
                ui.strong("Value");
                ui.end_row();
                for (i, (k, v)) in self.tags_edit.iter_mut().enumerate() {
                    ui.add_sized([kw, ROW_H], TextEdit::singleline(k).margin(FIELD_MARGIN));
                    ui.add_sized([vw, ROW_H], TextEdit::singleline(v).margin(FIELD_MARGIN));
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.tags_edit.remove(i);
        }
        if self.tags_edit.is_empty() {
            ui.weak("No tags.");
        }
        let cleaned = clean_tags(&self.tags_edit);
        let diff = cleaned.as_ref().ok().map(|t| tag_diff(&d.tags, t));
        let changed = diff
            .as_ref()
            .is_none_or(|(s, r)| !s.is_empty() || !r.is_empty());
        if let Err(e) = &cleaned {
            ui.label(RichText::new(e.to_string()).color(red(ui)));
        }
        ui.horizontal(|ui| {
            if ui.button("+ Add tag").clicked() {
                self.tags_edit.push(Default::default());
            }
            if ui
                .add_enabled(
                    changed && diff.is_some(),
                    tone_button(ui, "Save tags", Tone::Save),
                )
                .clicked()
                && let Some((set, remove)) = diff.clone()
            {
                self.run(Action::Tags {
                    id: d.name.clone(),
                    set,
                    remove,
                });
            }
            if changed && ui.button("Revert").clicked() {
                self.tags_edit = d.tags.clone();
            }
        });
    }

    fn rotation_tab(&mut self, ui: &mut Ui, d: &SecretDetail) {
        if d.rotation_enabled {
            chip(ui, "Rotation enabled", theme::palette(ui).ansi.green);
        } else {
            chip(ui, "Rotation disabled", theme::palette(ui).fg_weak);
        }
        if d.rotation_enabled {
            ui.label(format!(
                "Last rotated {} · next rotation {}",
                fmt_time(d.last_rotated),
                fmt_time(d.next_rotation)
            ));
        }
        ui.add_space(16.0);
        let r = &mut self.rotation;
        Grid::new("rotation")
            .num_columns(2)
            .spacing([32.0, 14.0])
            .show(ui, |ui| {
                ui.label("Rotation Lambda ARN");
                ui.add(
                    TextEdit::singleline(&mut r.lambda)
                        .margin(FIELD_MARGIN)
                        .hint_text("blank for managed rotation or to keep the current function")
                        .desired_width(460.0),
                );
                ui.end_row();
                ui.label("Schedule");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut r.use_schedule, false, "Every");
                    ui.add_enabled(
                        !r.use_schedule,
                        TextEdit::singleline(&mut r.days)
                            .margin(FIELD_MARGIN)
                            .desired_width(50.0),
                    );
                    ui.label("days");
                    ui.radio_value(&mut r.use_schedule, true, "Expression");
                    ui.add_enabled(
                        r.use_schedule,
                        TextEdit::singleline(&mut r.schedule)
                            .margin(FIELD_MARGIN)
                            .hint_text("rate(10 days) or cron(0 4 ? * SUN *)")
                            .desired_width(220.0),
                    );
                });
                ui.end_row();
                ui.label("Window");
                ui.add(
                    TextEdit::singleline(&mut r.window)
                        .margin(FIELD_MARGIN)
                        .hint_text("e.g. 3h")
                        .desired_width(80.0),
                );
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut r.rotate_now, "Rotate immediately after saving");
                ui.end_row();
            });
        ui.add_space(12.0);
        let mut action = None;
        ui.horizontal(|ui| {
            let label = if d.rotation_enabled {
                "Save rotation settings"
            } else {
                "Enable rotation"
            };
            let tone = if d.rotation_enabled {
                Tone::Save
            } else {
                Tone::Create
            };
            if ui.add(tone_button(ui, label, tone)).clicked() {
                let r = &self.rotation;
                let days = if r.use_schedule || r.days.trim().is_empty() {
                    Ok(None)
                } else {
                    r.days
                        .trim()
                        .parse::<i64>()
                        .map(Some)
                        .map_err(|_| "Days must be a whole number")
                };
                match days {
                    Err(e) => action = Some(Err(e.to_string())),
                    Ok(days) => {
                        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
                        action = Some(Ok(Action::Rotate(
                            d.name.clone(),
                            RotationConfig {
                                lambda_arn: opt(&r.lambda),
                                days,
                                schedule: if r.use_schedule {
                                    opt(&r.schedule)
                                } else {
                                    None
                                },
                                window: opt(&r.window),
                                rotate_immediately: r.rotate_now,
                            },
                        )));
                    }
                }
            }
            if d.rotation_enabled {
                if ui.button("Rotate now").clicked() {
                    self.confirm = Some(Confirm {
                        title: "Rotate now".into(),
                        body: format!("Rotate {} now using its current rotation settings?", d.name),
                        button: "Rotate".into(),
                        action: Action::Rotate(
                            d.name.clone(),
                            RotationConfig {
                                rotate_immediately: true,
                                ..Default::default()
                            },
                        ),
                    });
                }
                if ui
                    .button(RichText::new("Disable rotation").color(red(ui)))
                    .clicked()
                {
                    self.confirm = Some(Confirm {
                        title: "Disable rotation".into(),
                        body: format!("Turn off automatic rotation for {}?", d.name),
                        button: "Disable".into(),
                        action: Action::CancelRotation(d.name.clone()),
                    });
                }
            }
        });
        match action {
            Some(Ok(a)) => self.run(a),
            Some(Err(e)) => self.error(e),
            None => {}
        }
    }

    // ----- dialogs -----------------------------------------------------------------------

    fn settings_window(&mut self, ctx: &egui::Context) {
        if self.settings.is_none() {
            return;
        }
        let mut close = false;
        let size = egui::vec2(
            (ctx.content_rect().width() - 80.0).clamp(640.0, 960.0),
            (ctx.content_rect().height() - 80.0).clamp(420.0, 660.0),
        );
        const NAV_W: f32 = 210.0;
        let resp = Modal::new(Id::unique("settings"))
            .frame(Frame::popup(&ctx.global_style()).inner_margin(Margin::same(0)))
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                let page = self.settings.as_ref().map(|s| s.page);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                    // Navigation column
                    ui.allocate_ui_with_layout(
                        egui::vec2(NAV_W, size.y),
                        Layout::top_down_justified(Align::LEFT),
                        |ui| {
                            let rect = ui.max_rect();
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius {
                                    nw: 12,
                                    sw: 12,
                                    ne: 0,
                                    se: 0,
                                },
                                self.palette.sidebar_bg,
                            );
                            ui.set_min_size(egui::vec2(NAV_W, size.y));
                            Frame::new()
                                .inner_margin(Margin::symmetric(16, 24))
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                                    ui.label(RichText::new("Settings").size(20.0).strong());
                                    ui.add_space(12.0);
                                    for (p, label) in [
                                        (SettingsPage::Connections, "Connections"),
                                        (SettingsPage::Appearance, "Appearance"),
                                    ] {
                                        let b = egui::Button::selectable(page == Some(p), label)
                                            .min_size(egui::vec2(0.0, 36.0));
                                        if ui.add(b).clicked()
                                            && let Some(st) = &mut self.settings
                                        {
                                            st.page = p;
                                        }
                                    }
                                });
                        },
                    );

                    // Content column
                    let content_w = size.x - NAV_W;
                    ui.allocate_ui_with_layout(
                        egui::vec2(content_w, size.y),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.set_min_size(egui::vec2(content_w, size.y));
                            Frame::new()
                                .inner_margin(Margin::symmetric(36, 28))
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                                    let inner_w = content_w - 72.0;
                                    ui.set_width(inner_w);
                                    ScrollArea::vertical()
                                        .auto_shrink(false)
                                        .max_height(size.y - 56.0 - 52.0)
                                        .show(ui, |ui| {
                                            ui.set_width(inner_w - 16.0);
                                            match page {
                                                Some(SettingsPage::Appearance) => {
                                                    self.appearance_page(ui)
                                                }
                                                _ => self.connections_page(ui),
                                            }
                                        });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.button(RichText::new("Done").strong()).clicked() {
                                            close = true;
                                        }
                                    });
                                });
                        },
                    );
                });
            });
        let editing = self.settings.as_ref().is_some_and(|s| s.editor.is_some());
        if close || (resp.should_close() && !editing) {
            self.settings = None;
        }
    }

    fn connections_page(&mut self, ui: &mut Ui) {
        if self.settings.as_ref().is_some_and(|s| s.editor.is_some()) {
            return self.connection_editor(ui);
        }
        ui.heading(RichText::new("Connections").strong());
        ui.label(
            "Save AWS access keys here to use the app without an AWS config file. \
             Saved connections appear in the Account menu.",
        );
        ui.weak(format!(
            "Secret keys and session tokens are kept in {}, not in a file.",
            if cfg!(windows) {
                "Windows Credential Manager"
            } else if cfg!(target_os = "macos") {
                "your macOS Keychain"
            } else {
                "the system keyring"
            }
        ));
        ui.add_space(8.0);

        let mut action: Option<(&'static str, String)> = None;
        let confirm_delete = self
            .settings
            .as_ref()
            .and_then(|s| s.confirm_delete.clone());
        for c in &self.connections {
            let in_use = self.source == Some(Source::Connection(c.id.clone()));
            Frame::new()
                .fill(self.palette.input_bg)
                .stroke(egui::Stroke::new(1.0, self.palette.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let text_w = (ui.available_width() - 250.0).max(120.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(text_w, 0.0),
                            Layout::top_down(Align::LEFT),
                            |ui| {
                                ui.set_width(text_w);
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&c.name).strong().size(16.0));
                                    if in_use {
                                        chip(ui, "in use", theme::palette(ui).ansi.green);
                                    }
                                });
                                let mut detail = format!("{} · {}", c.key_hint(), c.region);
                                if !c.role_arn.trim().is_empty() {
                                    detail.push_str(&format!(" · assumes {}", c.role_arn.trim()));
                                }
                                ui.add(egui::Label::new(RichText::new(detail).weak()).truncate());
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if confirm_delete.as_deref() == Some(c.id.as_str()) {
                                if ui.button("Cancel").clicked() {
                                    action = Some(("cancel-delete", c.id.clone()));
                                }
                                if ui
                                    .button(RichText::new("Delete").color(red(ui)).strong())
                                    .clicked()
                                {
                                    action = Some(("delete", c.id.clone()));
                                }
                                ui.label("Delete this connection?");
                            } else {
                                if ui.button(RichText::new("Delete").color(red(ui))).clicked() {
                                    action = Some(("ask-delete", c.id.clone()));
                                }
                                if ui.button("Edit").clicked() {
                                    action = Some(("edit", c.id.clone()));
                                }
                                if !in_use && ui.button("Use").clicked() {
                                    action = Some(("use", c.id.clone()));
                                }
                            }
                        });
                    });
                });
        }
        if self.connections.is_empty() {
            ui.weak("No saved connections yet.");
        }
        ui.add_space(4.0);
        if ui
            .button(RichText::new("+ Add connection").strong())
            .clicked()
        {
            action = Some(("add", String::new()));
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.strong("AWS config files");
        let changed = ui
            .checkbox(
                &mut self.use_aws_config,
                format!(
                    "Also list profiles from ~/.aws/config and ~/.aws/credentials ({} found)",
                    self.profiles.len()
                ),
            )
            .changed();
        ui.weak("Includes SSO and assume-role profiles set up with the AWS CLI.");
        if changed
            && (self.source.is_none()
                || (!self.use_aws_config && matches!(self.source, Some(Source::Profile(_)))))
        {
            self.source = self.default_source();
            self.connect();
        }

        let Some((what, id)) = action else { return };
        let st = self.settings.as_mut().expect("settings open");
        st.test_result = None;
        match what {
            "add" => st.editor = Some(new_editor()),
            "edit" => {
                if let Some(conn) = self.connections.iter().find(|c| c.id == id).cloned() {
                    let secret = match self.vault.get(&id) {
                        Ok(s) => s.unwrap_or_default(),
                        Err(e) => {
                            self.error(format!("{e:#}"));
                            ConnectionSecret::default()
                        }
                    };
                    self.settings.as_mut().unwrap().editor = Some(ConnEditor {
                        conn,
                        secret,
                        is_new: false,
                    });
                }
            }
            "ask-delete" => st.confirm_delete = Some(id),
            "cancel-delete" => st.confirm_delete = None,
            "delete" => {
                st.confirm_delete = None;
                if let Err(e) = self.vault.delete(&id) {
                    self.error(format!("{e:#}"));
                }
                self.connections.retain(|c| c.id != id);
                if self.source == Some(Source::Connection(id)) {
                    self.source = self.default_source();
                    self.connect();
                }
            }
            "use" => {
                self.use_source(Source::Connection(id));
            }
            _ => {}
        }
    }

    fn connection_editor(&mut self, ui: &mut Ui) {
        let palette = self.palette.clone();
        let others = self.connections.clone();
        let Some(st) = &mut self.settings else { return };
        let Some(ed) = &mut st.editor else { return };
        ui.heading(
            RichText::new(if ed.is_new {
                "New connection"
            } else {
                "Edit connection"
            })
            .strong(),
        );
        ui.add_space(4.0);
        let field_w = (ui.available_width() - 200.0).clamp(280.0, 520.0);
        Grid::new("conn-form")
            .num_columns(2)
            .spacing([24.0, 14.0])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, label: &str, hint: &str, value: &mut String| {
                    let l = ui.label(label);
                    ui.add_sized(
                        [field_w, ROW_H],
                        TextEdit::singleline(value)
                            .margin(FIELD_MARGIN)
                            .hint_text(hint),
                    )
                    .labelled_by(l.id);
                    ui.end_row();
                };
                row(ui, "Name", "e.g. Production", &mut ed.conn.name);
                row(ui, "Access key ID", "AKIA…", &mut ed.conn.access_key_id);
                row(
                    ui,
                    "Secret access key",
                    "",
                    &mut ed.secret.secret_access_key,
                );
                row(
                    ui,
                    "Session token",
                    "Only for temporary (ASIA…) keys",
                    &mut ed.secret.session_token,
                );
                let l = ui.label("Region");
                ComboBox::from_id_salt("conn-region")
                    .selected_text(if ed.conn.region.is_empty() {
                        "Choose…"
                    } else {
                        &ed.conn.region
                    })
                    .width(field_w)
                    .height(360.0)
                    .show_ui(ui, |ui| {
                        for r in REGIONS {
                            ui.selectable_value(&mut ed.conn.region, r.to_string(), *r);
                        }
                    })
                    .response
                    .labelled_by(l.id);
                ui.end_row();
                row(
                    ui,
                    "Assume role ARN",
                    "Optional, e.g. arn:aws:iam::123456789012:role/Admin",
                    &mut ed.conn.role_arn,
                );
                if !ed.conn.role_arn.trim().is_empty() {
                    row(ui, "External ID", "Optional", &mut ed.conn.external_id);
                }
            });

        let valid = ed.conn.validate(&ed.secret, &others);
        ui.add_space(8.0);
        if let Some((ok, msg)) = &st.test_result {
            ui.horizontal_wrapped(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    rect.center(),
                    5.0,
                    if *ok { palette.success } else { palette.error },
                );
                ui.label(msg);
            });
        } else if let Err(e) = &valid
            && !ed.conn.name.is_empty()
        {
            ui.weak(e.to_string());
        }

        let mut test = false;
        let mut save = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(valid.is_ok(), tone_button(ui, "Save", Tone::Save))
                .clicked()
            {
                save = true;
            }
            if ui
                .add_enabled(
                    valid.is_ok() && !st.testing,
                    egui::Button::new("Test connection"),
                )
                .clicked()
            {
                test = true;
            }
            if st.testing {
                ui.spinner();
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });

        if test {
            st.testing = true;
            st.test_result = None;
            let opts = connection_opts(&ed.conn, &ed.secret, None);
            self.spawn(async move {
                Msg::Tested(async { Sm::connect(&opts).await?.check().await }.await)
            });
        } else if cancel {
            st.editor = None;
            st.test_result = None;
        } else if save {
            let ed = st.editor.take().expect("editor open");
            st.test_result = None;
            let mut conn = ed.conn;
            conn.name = conn.name.trim().to_string();
            conn.access_key_id = conn.access_key_id.trim().to_string();
            if let Err(e) = self.vault.set(&conn.id, &ed.secret) {
                self.error(format!("Couldn't save the secret key: {e:#}"));
                return;
            }
            let id = conn.id.clone();
            match self.connections.iter_mut().find(|c| c.id == id) {
                Some(existing) => *existing = conn,
                None => self.connections.push(conn),
            }
            self.status = Some(("Connection saved".into(), false));
            // Use it right away if nothing is connected, or reconnect if it's in use.
            let src = Source::Connection(id);
            if self.source.is_none() || self.source.as_ref() == Some(&src) {
                self.use_source(src);
            }
        }
    }

    fn appearance_page(&mut self, ui: &mut Ui) {
        ui.heading(RichText::new("Appearance").strong());
        ui.label(
            "Pick a theme for dark mode and one for light mode. VS Code color themes are \
             supported, including any installed in VS Code.",
        );
        ui.add_space(8.0);
        let mut import = false;
        Grid::new("appearance")
            .num_columns(2)
            .spacing([28.0, 16.0])
            .show(ui, |ui| {
                ui.label("Appearance");
                ui.horizontal(|ui| {
                    for (a, label) in [
                        (Appearance::System, "System"),
                        (Appearance::Light, "Light"),
                        (Appearance::Dark, "Dark"),
                    ] {
                        let b = egui::Button::selectable(self.appearance == a, label)
                            .min_size(egui::vec2(72.0, 32.0));
                        if ui.add(b).clicked() {
                            self.appearance = a;
                        }
                    }
                });
                ui.end_row();

                let l = ui.label("Dark theme");
                self.theme_combo(ui, true, "dark-theme").labelled_by(l.id);
                ui.end_row();

                let l = ui.label("Light theme");
                self.theme_combo(ui, false, "light-theme").labelled_by(l.id);
                ui.end_row();

                ui.label("Colors");
                ui.checkbox(&mut self.colorful, "Colorful accents")
                    .on_hover_text(
                        "Colored buttons, version labels, secret dots and headings, using the \
                     theme's own terminal colors",
                    );
                ui.end_row();

                ui.label("");
                if ui.button("Import VS Code theme…").clicked() {
                    import = true;
                }
                ui.end_row();
            });
        if import {
            self.import_theme();
        }
        ui.add_space(16.0);
        ui.collapsing("Theme licenses", |ui| {
            for (who, text) in theme::BUNDLED_LICENSES {
                ui.strong(*who);
                ui.label(RichText::new(*text).small());
                ui.add_space(8.0);
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let syntax = self.palette.syntax.clone();
        self.settings_window(ctx);
        if let Some(c) = &self.confirm {
            let mut result = None;
            let resp = Modal::new(Id::unique("confirm"))
                .frame(dialog_frame(ctx))
                .show(ctx, |ui| {
                    ui.set_max_width(480.0);
                    ui.heading(&c.title);
                    ui.add_space(12.0);
                    ui.label(&c.body);
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(&c.button).strong()).clicked() {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            if resp.should_close() && result.is_none() {
                result = Some(false);
            }
            if let Some(ok) = result
                && let Some(c) = self.confirm.take()
                && ok
            {
                self.run(c.action);
            }
        }

        if let Some(dlg) = &mut self.delete {
            let mut result = None;
            let resp = Modal::new(Id::unique("delete"))
                .frame(dialog_frame(ctx))
                .show(ctx, |ui| {
                    ui.set_max_width(520.0);
                    ui.heading(format!("Delete {}", dlg.id));
                    ui.add_space(12.0);
                    ui.radio_value(&mut dlg.force, false, "Schedule deletion (recoverable)");
                    ui.add_enabled_ui(!dlg.force, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.label("Recovery window");
                            ui.add(egui::Slider::new(&mut dlg.days, 7..=30).suffix(" days"));
                        });
                    });
                    ui.radio_value(
                        &mut dlg.force,
                        true,
                        RichText::new("Delete immediately. This can't be undone.").color(red(ui)),
                    );
                    ui.add_space(16.0);
                    ui.label(format!("Type {} to confirm:", dlg.id));
                    ui.add(
                        TextEdit::singleline(&mut dlg.typed)
                            .margin(FIELD_MARGIN)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let ok = dlg.typed == dlg.id;
                        if ui
                            .add_enabled(ok, tone_button(ui, "Delete", Tone::Danger))
                            .clicked()
                        {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            if resp.should_close() && result.is_none() {
                result = Some(false);
            }
            if let Some(ok) = result
                && let Some(dlg) = self.delete.take()
                && ok
            {
                let how = if dlg.force {
                    Ok(Deletion::Force)
                } else {
                    validate_recovery_days(dlg.days).map(Deletion::RecoveryWindow)
                };
                match how {
                    Ok(how) => self.run(Action::Delete(dlg.id, how)),
                    Err(e) => self.error(e),
                }
            }
        }

        if self.new_secret.is_some() {
            self.new_secret_dialog(ctx);
        }

        if let Some((version, value)) = &self.peek {
            let mut open = true;
            egui::Window::new(format!("Version {version}"))
                .open(&mut open)
                .collapsible(false)
                .default_width(520.0)
                .show(ctx, |ui| match value {
                    None => {
                        ui.spinner();
                    }
                    Some(v) => {
                        ui.weak(format!(
                            "Labels: {}",
                            if v.stages.is_empty() {
                                "(none)".into()
                            } else {
                                v.stages.join(", ")
                            }
                        ));
                        let mut text = display_value(&v.value);
                        ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                            code_editor(ui, &mut text, &syntax, 6, false);
                        });
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(match &v.value {
                                SecretValue::Text(s) => s.clone(),
                                SecretValue::Binary(b) => b64(b),
                            });
                        }
                    }
                });
            if !open {
                self.peek = None;
            }
        }
    }

    fn new_secret_dialog(&mut self, ctx: &egui::Context) {
        let mut result = None;
        let mut gen_password = false;
        let syntax = self.palette.syntax.clone();
        let Some(n) = &mut self.new_secret else {
            return;
        };
        let resp = Modal::new(Id::unique("new-secret"))
            .frame(dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(680.0);
                ui.heading("New secret");
                ui.add_space(12.0);
                Grid::new("new-meta")
                    .num_columns(2)
                    .spacing([24.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add(
                            TextEdit::singleline(&mut n.name)
                                .margin(FIELD_MARGIN)
                                .hint_text("e.g. prod/myapp/db")
                                .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label("Description");
                        ui.add(
                            TextEdit::singleline(&mut n.description)
                                .margin(FIELD_MARGIN)
                                .hint_text("Optional")
                                .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label("KMS key");
                        ui.add(
                            TextEdit::singleline(&mut n.kms)
                                .margin(FIELD_MARGIN)
                                .hint_text("aws/secretsmanager (default)")
                                .desired_width(380.0),
                        );
                        ui.end_row();
                    });
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.strong("Value");
                    ui.radio_value(&mut n.kind, NewKind::KeyValue, "Key/value pairs");
                    ui.radio_value(&mut n.kind, NewKind::Text, "Plain text");
                    ui.radio_value(&mut n.kind, NewKind::Empty, "No value yet");
                });
                match n.kind {
                    NewKind::KeyValue => {
                        let mut remove = None;
                        Grid::new("new-kv")
                            .num_columns(3)
                            .spacing([12.0, 10.0])
                            .show(ui, |ui| {
                                for (i, row) in n.rows.iter_mut().enumerate() {
                                    ui.add_sized(
                                        [220.0, ROW_H],
                                        TextEdit::singleline(&mut row.key)
                                            .margin(FIELD_MARGIN)
                                            .hint_text("key"),
                                    );
                                    ui.add_sized(
                                        [340.0, ROW_H],
                                        TextEdit::singleline(&mut row.text)
                                            .margin(FIELD_MARGIN)
                                            .hint_text("value"),
                                    );
                                    if ui.button("Remove").clicked() {
                                        remove = Some(i);
                                    }
                                    ui.end_row();
                                }
                            });
                        if let Some(i) = remove {
                            n.rows.remove(i);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("+ Add key").clicked() {
                                n.rows.push(KvRow {
                                    key: String::new(),
                                    text: String::new(),
                                    original: None,
                                });
                            }
                            if ui.button("Add generated password").clicked() {
                                gen_password = true;
                            }
                        });
                    }
                    NewKind::Text => {
                        code_editor(ui, &mut n.text, &syntax, 6, true);
                        if let Some(pretty) = format_json(&n.text).filter(|p| p != &n.text)
                            && ui.button("Format JSON").clicked()
                        {
                            n.text = pretty;
                        }
                        if ui.button("Generate password").clicked() {
                            gen_password = true;
                        }
                    }
                    NewKind::Empty => {
                        ui.weak(
                        "The secret is created without a value. Add one later from the Value tab.",
                    );
                    }
                }
                ui.add_space(16.0);
                ui.collapsing(format!("Tags ({})", n.tags.len()), |ui| {
                    let mut remove = None;
                    for (i, (k, v)) in n.tags.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [220.0, ROW_H],
                                TextEdit::singleline(k)
                                    .margin(FIELD_MARGIN)
                                    .hint_text("key"),
                            );
                            ui.add_sized(
                                [320.0, ROW_H],
                                TextEdit::singleline(v)
                                    .margin(FIELD_MARGIN)
                                    .hint_text("value"),
                            );
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        n.tags.remove(i);
                    }
                    if ui.button("+ Add tag").clicked() {
                        n.tags.push(Default::default());
                    }
                });

                let problem = if n.name.trim().is_empty() {
                    Some("Enter a name".to_string())
                } else if let Err(e) = clean_tags(&n.tags) {
                    Some(e.to_string())
                } else if n.kind == NewKind::KeyValue {
                    kv_to_json(&n.rows).err().map(|e| e.to_string())
                } else {
                    None
                };
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            problem.is_none(),
                            tone_button(ui, "Create secret", Tone::Create),
                        )
                        .clicked()
                    {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                    if let Some(p) = &problem
                        && !n.name.trim().is_empty()
                    {
                        ui.label(RichText::new(p).color(red(ui)));
                    }
                });
            });
        if gen_password {
            self.generate_password();
        }
        if resp.should_close() && result.is_none() {
            result = Some(false);
        }
        if let Some(ok) = result
            && let Some(mut n) = self.new_secret.take()
            && ok
        {
            n.tags = clean_tags(&n.tags).unwrap_or_default();
            self.run(Action::Create(n));
        }
    }
}

impl eframe::App for SecretsApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "appearance", &self.appearance);
        eframe::set_value(storage, "dark_theme", &self.dark_theme);
        eframe::set_value(storage, "light_theme", &self.light_theme);
        eframe::set_value(storage, "colorful", &self.colorful);
        eframe::set_value(storage, "imported_themes", &self.imported_themes);
        eframe::set_value(storage, "connections", &self.connections);
        eframe::set_value(storage, "code_lang", &self.code_lang);
        eframe::set_value(storage, "use_aws_config", &self.use_aws_config);
        if let Some(source) = &self.source {
            eframe::set_value(storage, "source", source);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
        let ctx = ui.ctx().clone();
        self.sync_theme(&ctx);

        // Manual refresh shortcut: Cmd/Ctrl+R or F5.
        let refresh_key = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::F5)
        });
        if refresh_key && self.settings.is_none() {
            self.refresh();
        }
        // Re-check the list while a recent create/delete hasn't shown up in it yet.
        if let Some(at) = self.recheck_at {
            let now = std::time::Instant::now();
            if now >= at {
                self.recheck_at = None;
                if self.busy == 0 {
                    self.reload_list();
                } else {
                    self.recheck_at = Some(now + RECHECK_EVERY);
                }
            } else {
                ctx.request_repaint_after(at - now);
            }
        }

        let p = self.palette.clone();
        let panel = |fill: Color32, x: i8, y: i8| {
            Frame::new()
                .fill(fill)
                .inner_margin(Margin::symmetric(x, y))
                .stroke(egui::Stroke::new(1.0, p.border))
        };
        egui::Panel::top("top")
            .frame(panel(p.titlebar_bg, 20, 14))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .frame(panel(p.status_bg, 20, 10))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("list")
            .frame(panel(p.sidebar_bg, 14, 14))
            .resizable(true)
            .default_size(320.0)
            .size_range(220.0..=640.0)
            .show(ui, |ui| self.secret_list(ui));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.editor_bg)
                    .inner_margin(Margin::symmetric(32, 24)),
            )
            .show(ui, |ui| self.details(ui));

        self.dialogs(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn sample(ctx: egui::Context) -> SecretsApp {
        let mut app = SecretsApp::offline(ctx);
        app.vault = Box::new(crate::connections::MemVault(Default::default()));
        app.profiles = vec!["dev".into()];
        app.source = Some(Source::Profile("dev".into()));
        app.region = "us-east-1".into();
        let now = chrono::Utc::now();
        app.secrets = [
            "dev/api-key",
            "prod/db",
            "prod/old-token",
            "rds-db-credentials/cluster-ZPV5ADGCPW7EDA6RA7B7NPGVHI/postgres/1768923618636",
        ]
            .iter()
            .map(|n| SecretSummary {
                name: n.to_string(),
                description: Some(format!("{n} description")),
                last_changed: Some(now),
                deleted: (*n == "prod/old-token").then_some(now),
                rotation_enabled: *n == "prod/db",
                tags: vec![("team".into(), "core".into())],
            })
            .collect();
        app.selected = Some("prod/db".into());
        let mut versions = std::collections::BTreeMap::new();
        versions.insert("v-2".to_string(), vec!["AWSCURRENT".to_string()]);
        versions.insert("v-1".to_string(), vec!["AWSPREVIOUS".to_string()]);
        app.detail = Some(SecretDetail {
            name: "prod/db".into(),
            arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:prod/db-AbCdEf".into(),
            description: Some("Primary database".into()),
            created: Some(now),
            last_changed: Some(now),
            rotation_enabled: true,
            rotation_lambda: Some("arn:aws:lambda:us-east-1:123456789012:function:rotate".into()),
            rotation_days: Some(30),
            tags: vec![
                ("team".into(), "core".into()),
                ("env".into(), "prod".into()),
            ],
            versions,
            ..Default::default()
        });
        app.versions = vec![
            VersionInfo {
                id: "v-2".into(),
                stages: vec!["AWSCURRENT".into()],
                created: Some(now),
                last_accessed: Some(now),
            },
            VersionInfo {
                id: "v-1".into(),
                stages: vec!["AWSPREVIOUS".into()],
                created: Some(now),
                last_accessed: None,
            },
            VersionInfo {
                id: "v-0".into(),
                stages: vec![],
                created: Some(now),
                last_accessed: None,
            },
        ];
        let d = app.detail.clone().unwrap();
        app.meta_desc = d.description.clone().unwrap_or_default();
        app.tags_edit = d.tags.clone();
        app.rotation.days = "30".into();
        app.rotation.lambda = d.rotation_lambda.clone().unwrap();
        app
    }

    /// Switch to `t`, setting the appearance to match its type.
    fn use_theme(h: &mut Harness<'static, SecretsApp>, t: ThemeRef) {
        let app = h.state_mut();
        if t.dark() {
            app.appearance = Appearance::Dark;
            app.dark_theme = t;
        } else {
            app.appearance = Appearance::Light;
            app.light_theme = t;
        }
    }

    fn reset_theme(h: &mut Harness<'static, SecretsApp>) {
        let app = h.state_mut();
        app.appearance = Appearance::System;
        app.dark_theme = ThemeRef::default_dark();
        app.light_theme = ThemeRef::default_light();
        app.colorful = true;
    }

    fn harness() -> Harness<'static, SecretsApp> {
        Harness::builder()
            .with_size([1200.0, 700.0])
            .build_eframe(|cc| sample(cc.egui_ctx.clone()))
    }

    /// Render the current frame. Set SECRETS_UI_SHOTS=<dir> to save PNGs for manual review.
    fn shot(h: &mut Harness<'static, SecretsApp>, name: &str) {
        h.run_steps(3);
        // GPU-less CI machines may not be able to render; the frame logic above still ran.
        match h.render() {
            Ok(img) => {
                if let Ok(dir) = std::env::var("SECRETS_UI_SHOTS") {
                    img.save(std::path::Path::new(&dir).join(format!("{name}.png")))
                        .unwrap();
                }
            }
            Err(e) => eprintln!("skipping render of {name}: {e}"),
        }
    }

    #[test]
    fn renders_every_view() {
        let mut h = harness();
        for (tab, name) in [
            (Tab::Overview, "overview"),
            (Tab::Value, "value-loading"),
            (Tab::Versions, "versions"),
            (Tab::Tags, "tags"),
            (Tab::Rotation, "rotation"),
        ] {
            h.state_mut().tab = tab;
            shot(&mut h, name);
        }

        h.state_mut().tab = Tab::Value;
        let json = r#"{"username":"app","password":"hunter2","port":5432}"#.to_string();
        h.state_mut().editor = Some(Editor::Kv {
            rows: kv_rows(&json_object(&json).unwrap()),
            original: json,
        });
        shot(&mut h, "value-kv");
        h.state_mut().colorful = false;
        shot(&mut h, "value-kv-plain");
        h.state_mut().colorful = true;
        // The resizable list panel must not creep wider frame after frame.
        let list_width =
            |h: &mut Harness<'static, SecretsApp>| h.get_by_label("Refresh").rect().right();
        let before = list_width(&mut h);
        h.run_steps(40);
        assert_eq!(list_width(&mut h), before, "list panel width is stable");

        h.state_mut().new_secret = Some(NewSecret {
            name: "prod/new".into(),
            rows: vec![KvRow {
                key: "user".into(),
                text: "x".into(),
                original: None,
            }],
            ..Default::default()
        });
        shot(&mut h, "new-secret");
        h.state_mut().new_secret = None;

        h.state_mut().delete = Some(DeleteDialog {
            id: "prod/db".into(),
            days: 30,
            force: false,
            typed: String::new(),
        });
        shot(&mut h, "delete");
        h.state_mut().delete = None;

        h.state_mut().appearance = Appearance::Light;
        shot(&mut h, "theme-light");
        assert!(!h.state_mut().palette.dark);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ocean.json");
        std::fs::write(
            &path,
            r##"{
                // A VS Code theme fixture (JSONC)
                "name": "Ocean Test",
                "type": "dark",
                "colors": {
                    "editor.background": "#0b1d2e",
                    "sideBar.background": "#08172a",
                    "titleBar.activeBackground": "#061322",
                    "statusBar.background": "#0e639c",
                    "statusBar.foreground": "#ffffff",
                    "foreground": "#d6deeb",
                    "button.background": "#2a9d8f",
                    "list.activeSelectionBackground": "#1d3b53",
                    "input.background": "#0f2a40",
                },
            }"##,
        )
        .unwrap();
        use_theme(
            &mut h,
            ThemeRef::File {
                label: "Ocean Test".into(),
                path,
                dark: true,
            },
        );
        shot(&mut h, "theme-vscode");
        assert_eq!(
            h.state_mut().palette.editor_bg,
            Color32::from_rgb(0x0b, 0x1d, 0x2e)
        );

        use_theme(
            &mut h,
            ThemeRef::Bundled("night-owl-color-theme.json".into()),
        );
        shot(&mut h, "theme-night-owl");

        // JSON editor, highlighted per theme.
        let raw = r#"{"db":{"host":"db.internal","port":5432,"tls":true},"replicas":["r1","r2"],"note":null}"#;
        h.state_mut().tab = Tab::Value;
        for (choice, name) in [
            (
                ThemeRef::Bundled("night-owl-color-theme.json".into()),
                "json-night-owl",
            ),
            (
                ThemeRef::Bundled("monokai-color-theme.json".into()),
                "json-monokai",
            ),
            (ThemeRef::default_light(), "json-light-modern"),
            (ThemeRef::default_dark(), "json-dark"),
        ] {
            use_theme(&mut h, choice);
            h.state_mut().editor = Some(Editor::Text {
                text: format_json(raw).unwrap(),
                original: raw.into(),
            });
            shot(&mut h, name);
        }
        reset_theme(&mut h);

        // Code samples.
        reset_theme(&mut h);
        let raw = r#"{"username":"app","password":"hunter2","port":5432}"#.to_string();
        h.state_mut().editor = Some(Editor::Kv {
            rows: kv_rows(&json_object(&raw).unwrap()),
            original: raw,
        });
        h.state_mut().tab = Tab::Code;
        for (lang, name) in [
            (Lang::TypeScript, "code-typescript"),
            (Lang::Go, "code-go"),
            (Lang::Python, "code-python"),
        ] {
            h.state_mut().code_lang = lang;
            shot(&mut h, name);
        }
        h.state_mut().code_lang = Lang::Java;
        use_theme(
            &mut h,
            ThemeRef::Bundled("night-owl-color-theme.json".into()),
        );
        shot(&mut h, "code-java-night-owl");
        reset_theme(&mut h);
        h.state_mut().tab = Tab::Value;

        // Settings: connections list, editor, appearance.
        h.state_mut().connections = vec![
            Connection {
                id: "c1".into(),
                name: "Production".into(),
                access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
                region: "us-east-1".into(),
                role_arn: "arn:aws:iam::123456789012:role/SecretsAdmin".into(),
                ..Default::default()
            },
            Connection {
                id: "c2".into(),
                name: "Staging".into(),
                access_key_id: "AKIAI44QH8DHBEXAMPLE".into(),
                region: "eu-west-1".into(),
                ..Default::default()
            },
        ];
        h.state_mut().settings = Some(SettingsUi::new(SettingsPage::Connections));
        shot(&mut h, "settings-connections");
        let mut ed = new_editor();
        ed.conn.name = "Dev".into();
        ed.conn.access_key_id = "AKIAIOSFODNN7EXAMPLE".into();
        ed.secret.secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into();
        h.state_mut().settings.as_mut().unwrap().editor = Some(ed);
        shot(&mut h, "settings-editor");
        h.state_mut().settings = Some(SettingsUi::new(SettingsPage::Appearance));
        shot(&mut h, "settings-appearance");
        h.state_mut().settings = None;

        // First run with no credentials anywhere.
        h.state_mut().source = None;
        h.state_mut().selected = None;
        shot(&mut h, "welcome");

        // Preview any theme file: SECRETS_THEME_FILE=path SECRETS_UI_SHOTS=dir cargo test
        if let Ok(f) = std::env::var("SECRETS_THEME_FILE") {
            let t = VsCodeTheme::load(std::path::Path::new(&f)).unwrap();
            use_theme(
                &mut h,
                ThemeRef::File {
                    label: "file".into(),
                    path: f.into(),
                    dark: t.dark,
                },
            );
            shot(&mut h, "theme-file");
        }
    }

    #[test]
    fn formatted_json_counts_as_unchanged() {
        let raw = r#"{"a":1,"b":[2]}"#;
        assert!(same_value(&format_json(raw).unwrap(), raw));
        assert_eq!(compact_if_json(&format_json(raw).unwrap()), raw);
    }

    #[test]
    fn new_secret_stays_listed_until_aws_catches_up() {
        let mut app = sample(egui::Context::default());
        app.busy = 2;
        app.handle(Msg::Done(
            "prod/new".into(),
            Change::Created,
            Ok("Created".into()),
        ));
        assert!(app.secrets.iter().any(|s| s.name == "prod/new"));
        assert_eq!(app.selected.as_deref(), Some("prod/new"));

        // ListSecrets hasn't caught up yet: the new secret must not vanish.
        let stale: Vec<SecretSummary> = app
            .secrets
            .iter()
            .filter(|s| s.name != "prod/new")
            .cloned()
            .collect();
        app.handle(Msg::Listed(Ok(stale.clone())));
        assert!(app.secrets.iter().any(|s| s.name == "prod/new"));
        assert_eq!(app.selected.as_deref(), Some("prod/new"));
        assert!(app.recheck_at.is_some(), "a re-check is scheduled");

        // Once it appears, tracking stops.
        let mut fresh = stale;
        fresh.push(SecretSummary {
            name: "prod/new".into(),
            description: Some("from AWS".into()),
            last_changed: None,
            deleted: None,
            rotation_enabled: false,
            tags: vec![],
        });
        app.busy = 1;
        app.handle(Msg::Listed(Ok(fresh)));
        assert!(app.pending.is_empty());
        assert!(app.recheck_at.is_none());
    }

    #[test]
    fn deleted_secret_disappears_immediately() {
        let mut app = sample(egui::Context::default());
        app.busy = 2;
        app.handle(Msg::Done(
            "prod/db".into(),
            Change::Deleted,
            Ok("Deleted".into()),
        ));
        assert!(!app.secrets.iter().any(|s| s.name == "prod/db"));
        assert_eq!(app.selected, None);
        // A stale list that still has it doesn't bring it back.
        let mut stale = app.secrets.clone();
        stale.push(SecretSummary {
            name: "prod/db".into(),
            description: None,
            last_changed: None,
            deleted: None,
            rotation_enabled: false,
            tags: vec![],
        });
        app.handle(Msg::Listed(Ok(stale)));
        assert!(!app.secrets.iter().any(|s| s.name == "prod/db"));
    }

    #[test]
    fn saved_connections_become_sources() {
        let mut app = sample(egui::Context::default());
        app.use_aws_config = false;
        assert!(!app.source_available(&Source::Profile("dev".into())));
        assert_eq!(app.default_source(), None);
        app.connections.push(Connection {
            id: "c9".into(),
            name: "Mine".into(),
            ..Default::default()
        });
        assert_eq!(app.default_source(), Some(Source::Connection("c9".into())));
        app.source = app.default_source();
        assert_eq!(app.source_label(), "Mine");
    }

    #[test]
    fn selecting_another_secret_drops_the_value() {
        let mut app = sample(egui::Context::default());
        app.editor = Some(Editor::Text {
            text: "s3cret".into(),
            original: "s3cret".into(),
        });
        app.select(Some("dev/api-key".into()));
        assert!(app.editor.is_none());
        assert!(app.detail.is_none());
    }

    #[test]
    fn stale_responses_are_ignored() {
        let mut app = sample(egui::Context::default());
        app.busy = 1;
        app.handle(Msg::Value(
            "dev/api-key".into(),
            Ok(Some(FetchedValue {
                value: SecretValue::Text("other".into()),
                version_id: None,
                stages: vec![],
                created: None,
            })),
        ));
        assert!(
            app.editor.is_none(),
            "value for a different secret must not be shown"
        );
        assert_eq!(app.busy, 0);
    }
}
