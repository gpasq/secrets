//! Light/dark themes and VS Code color-theme support.
//!
//! A [`Palette`] is the small set of colors the UI uses. The built-in light and dark
//! palettes follow VS Code's "Modern" defaults; a VS Code color theme (`*.json`, JSONC
//! with comments, trailing commas and `include` chains) is mapped onto the same palette,
//! falling back to the built-in palette of its type for any color it doesn't define.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use eframe::egui::{self, Color32, Stroke, Visuals};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::highlight::Syntax;

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub fg: Color32,
    pub fg_weak: Color32,
    pub editor_bg: Color32,
    pub sidebar_bg: Color32,
    pub sidebar_fg: Color32,
    pub titlebar_bg: Color32,
    pub titlebar_fg: Color32,
    pub status_bg: Color32,
    pub status_fg: Color32,
    pub input_bg: Color32,
    pub input_fg: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_fg: Color32,
    pub selection_bg: Color32,
    pub selection_fg: Color32,
    pub hover_bg: Color32,
    pub widget_bg: Color32,
    pub widget_fg: Color32,
    pub link: Color32,
    pub error: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// JSON syntax colors.
    pub syntax: Syntax,
    /// The theme's terminal colors, used for colorful accents.
    pub ansi: Ansi,
    /// Whether to use colorful accents (Settings → Appearance).
    pub colorful: bool,
}

/// A theme's six terminal colors. Every VS Code theme defines them, so they give each
/// theme its own set of accent colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ansi {
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub blue: Color32,
    pub magenta: Color32,
    pub cyan: Color32,
}

impl Ansi {
    /// VS Code's default terminal colors.
    pub fn default_for(dark: bool) -> Self {
        if dark {
            Self {
                red: hex(0xf14c4c),
                green: hex(0x23d18b),
                yellow: hex(0xe5c07b),
                blue: hex(0x3b8eea),
                magenta: hex(0xd670d6),
                cyan: hex(0x29b8db),
            }
        } else {
            Self {
                red: hex(0xcd3131),
                green: hex(0x00a000),
                yellow: hex(0xb08800),
                blue: hex(0x0451a5),
                magenta: hex(0xbc05bc),
                cyan: hex(0x0598bc),
            }
        }
    }
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Palette {
    /// VS Code Dark Modern: the default dark theme, and the fallback for missing colors.
    pub fn dark() -> Self {
        Self {
            dark: true,
            fg: hex(0xcccccc),
            fg_weak: hex(0x9d9d9d),
            editor_bg: hex(0x1f1f1f),
            sidebar_bg: hex(0x181818),
            sidebar_fg: hex(0xcccccc),
            titlebar_bg: hex(0x181818),
            titlebar_fg: hex(0xcccccc),
            status_bg: hex(0x181818),
            status_fg: hex(0xcccccc),
            input_bg: hex(0x313131),
            input_fg: hex(0xcccccc),
            border: hex(0x2b2b2b),
            accent: hex(0x0078d4),
            accent_fg: hex(0xffffff),
            selection_bg: hex(0x04395e),
            selection_fg: hex(0xffffff),
            hover_bg: hex(0x2a2d2e),
            widget_bg: hex(0x313131),
            widget_fg: hex(0xcccccc),
            link: hex(0x4daafc),
            error: hex(0xf85149),
            success: hex(0x3fb950),
            warning: hex(0xcca700),
            // VS Code Dark+
            syntax: Syntax {
                key: hex(0x9cdcfe),
                string: hex(0xce9178),
                number: hex(0xb5cea8),
                literal: hex(0x569cd6),
                punct: hex(0xcccccc),
                plain: hex(0xcccccc),
                keyword: hex(0x569cd6),
                comment: hex(0x6a9955),
                type_: hex(0x4ec9b0),
                function: hex(0xdcdcaa),
            },
            ansi: Ansi::default_for(true),
            colorful: true,
        }
    }

    /// VS Code Light Modern: the default light theme, and the fallback for missing colors.
    pub fn light() -> Self {
        Self {
            dark: false,
            fg: hex(0x3b3b3b),
            fg_weak: hex(0x767676),
            editor_bg: hex(0xffffff),
            sidebar_bg: hex(0xf8f8f8),
            sidebar_fg: hex(0x3b3b3b),
            titlebar_bg: hex(0xf8f8f8),
            titlebar_fg: hex(0x3b3b3b),
            status_bg: hex(0xf8f8f8),
            status_fg: hex(0x3b3b3b),
            input_bg: hex(0xffffff),
            input_fg: hex(0x3b3b3b),
            border: hex(0xd8d8d8),
            accent: hex(0x005fb8),
            accent_fg: hex(0xffffff),
            selection_bg: hex(0xcce4f7),
            selection_fg: hex(0x1f1f1f),
            hover_bg: hex(0xf0f0f0),
            widget_bg: hex(0xe8e8e8),
            widget_fg: hex(0x3b3b3b),
            link: hex(0x005fb8),
            error: hex(0xc72e0f),
            success: hex(0x1a7f37),
            warning: hex(0xbf8803),
            // VS Code Light+
            syntax: Syntax {
                key: hex(0x0451a5),
                string: hex(0xa31515),
                number: hex(0x098658),
                literal: hex(0x0000ff),
                punct: hex(0x3b3b3b),
                plain: hex(0x3b3b3b),
                keyword: hex(0x0000ff),
                comment: hex(0x008000),
                type_: hex(0x267f99),
                function: hex(0x795e26),
            },
            ansi: Ansi::default_for(false),
            colorful: true,
        }
    }

    /// Map a VS Code theme's `colors` onto a palette. `dark` is the theme's type.
    pub fn from_vscode(
        colors: &HashMap<String, Color32>,
        tokens: &[TokenRule],
        dark: bool,
    ) -> Self {
        let base = if dark { Self::dark() } else { Self::light() };
        let editor_bg = colors
            .get("editor.background")
            .copied()
            .unwrap_or(base.editor_bg);
        // Resolve the first defined key, flattening any transparency onto `under`.
        let pick = |keys: &[&str], under: Color32, fallback: Color32| -> Color32 {
            keys.iter()
                .find_map(|k| colors.get(*k))
                .map(|c| flatten(*c, under))
                .unwrap_or(fallback)
        };
        let fg = pick(&["foreground", "editor.foreground"], editor_bg, base.fg);
        let sidebar_bg = pick(
            &["sideBar.background", "editor.background"],
            editor_bg,
            base.sidebar_bg,
        );
        let titlebar_bg = pick(
            &[
                "titleBar.activeBackground",
                "sideBar.background",
                "editor.background",
            ],
            editor_bg,
            base.titlebar_bg,
        );
        let status_bg = pick(
            &[
                "statusBar.background",
                "titleBar.activeBackground",
                "sideBar.background",
            ],
            editor_bg,
            base.status_bg,
        );
        let accent = pick(
            &["button.background", "focusBorder"],
            editor_bg,
            base.accent,
        );
        let input_bg = pick(
            &["input.background", "dropdown.background"],
            editor_bg,
            base.input_bg,
        );
        Self {
            dark,
            fg,
            fg_weak: pick(
                &["descriptionForeground", "disabledForeground"],
                editor_bg,
                mix(fg, editor_bg, 0.4),
            ),
            editor_bg,
            sidebar_bg,
            sidebar_fg: pick(&["sideBar.foreground", "foreground"], sidebar_bg, fg),
            titlebar_bg,
            titlebar_fg: pick(
                &["titleBar.activeForeground", "foreground"],
                titlebar_bg,
                fg,
            ),
            status_bg,
            status_fg: pick(&["statusBar.foreground", "foreground"], status_bg, fg),
            input_bg,
            input_fg: pick(&["input.foreground", "foreground"], input_bg, fg),
            border: pick(
                &[
                    "panel.border",
                    "sideBar.border",
                    "editorGroup.border",
                    "widget.border",
                    "contrastBorder",
                ],
                editor_bg,
                mix(fg, editor_bg, 0.85),
            ),
            accent,
            accent_fg: pick(&["button.foreground"], accent, contrast_on(accent)),
            selection_bg: pick(
                &[
                    "list.activeSelectionBackground",
                    "editor.selectionBackground",
                ],
                sidebar_bg,
                mix(accent, sidebar_bg, 0.55),
            ),
            selection_fg: pick(&["list.activeSelectionForeground"], sidebar_bg, fg),
            hover_bg: pick(
                &["list.hoverBackground"],
                sidebar_bg,
                mix(fg, sidebar_bg, 0.9),
            ),
            widget_bg: pick(
                &["button.secondaryBackground", "input.background"],
                editor_bg,
                mix(fg, editor_bg, 0.85),
            ),
            widget_fg: pick(&["button.secondaryForeground", "foreground"], editor_bg, fg),
            link: pick(
                &["textLink.foreground", "textLink.activeForeground"],
                editor_bg,
                base.link,
            ),
            error: pick(
                &["errorForeground", "editorError.foreground"],
                editor_bg,
                base.error,
            ),
            success: pick(
                &[
                    "gitDecoration.addedResourceForeground",
                    "terminal.ansiGreen",
                ],
                editor_bg,
                base.success,
            ),
            warning: pick(
                &["editorWarning.foreground", "terminal.ansiYellow"],
                editor_bg,
                base.warning,
            ),
            syntax: {
                let input_bg = pick(&["input.background"], editor_bg, base.input_bg);
                let code_fg = pick(&["editor.foreground", "foreground"], editor_bg, fg);
                // Scope stacks as VS Code's JSON grammar produces them.
                let tok = |inner: &str, fallback: Color32| {
                    resolve_token(
                        tokens,
                        &["source.json", "meta.structure.dictionary.json", inner],
                    )
                    .map(|c| flatten(c, input_bg))
                    .unwrap_or(fallback)
                };
                let no_tokens = tokens.is_empty();
                // Generic source-code scopes: the first one the theme colors wins.
                let code_tok = |scopes: &[&str], fallback: Color32| {
                    scopes
                        .iter()
                        .find_map(|sc| resolve_token(tokens, &["source", sc]))
                        .map(|c| flatten(c, input_bg))
                        .unwrap_or(fallback)
                };
                let def = |c: Color32| if no_tokens { c } else { code_fg };
                Syntax {
                    key: tok("support.type.property-name.json", def(base.syntax.key)),
                    string: tok("string.quoted.double.json", def(base.syntax.string)),
                    number: tok("constant.numeric.json", def(base.syntax.number)),
                    literal: tok("constant.language.json", def(base.syntax.literal)),
                    punct: tok("punctuation.separator.dictionary.key-value.json", code_fg),
                    plain: code_fg,
                    keyword: code_tok(
                        &["keyword.control", "storage.type", "keyword"],
                        def(base.syntax.keyword),
                    ),
                    comment: code_tok(
                        &["comment.line.double-slash", "comment"],
                        def(base.syntax.comment),
                    ),
                    type_: code_tok(
                        &[
                            "entity.name.type.class",
                            "support.class",
                            "entity.name.type",
                        ],
                        def(base.syntax.type_),
                    ),
                    function: code_tok(
                        &["entity.name.function", "support.function"],
                        def(base.syntax.function),
                    ),
                }
            },
            ansi: {
                let d = Ansi::default_for(dark);
                let a = |key: &str, fallback: Color32| {
                    colors
                        .get(key)
                        .map(|c| flatten(*c, editor_bg))
                        .unwrap_or(fallback)
                };
                Ansi {
                    red: a("terminal.ansiRed", d.red),
                    green: a("terminal.ansiGreen", d.green),
                    yellow: a("terminal.ansiYellow", d.yellow),
                    blue: a("terminal.ansiBlue", d.blue),
                    magenta: a("terminal.ansiMagenta", d.magenta),
                    cyan: a("terminal.ansiCyan", d.cyan),
                }
            },
            colorful: true,
        }
    }

    /// egui visuals for this palette.
    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let hovered_bg = mix(self.widget_fg, self.widget_bg, 0.86);
        // egui uses the "active" widget text color for strong text (headings, bold labels).
        let strong = if self.dark {
            mix(self.fg, Color32::WHITE, 0.6)
        } else {
            mix(self.fg, Color32::BLACK, 0.6)
        };
        v.override_text_color = None;
        v.weak_text_color = Some(self.fg_weak);
        v.hyperlink_color = self.link;
        v.panel_fill = self.editor_bg;
        v.window_fill = self.sidebar_bg;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.extreme_bg_color = self.input_bg;
        v.text_edit_bg_color = Some(self.input_bg);
        v.code_bg_color = self.input_bg;
        v.faint_bg_color = mix(self.fg, self.editor_bg, 0.96);
        v.error_fg_color = self.error;
        v.warn_fg_color = self.warning;
        v.selection.bg_fill = self.selection_bg;
        v.selection.stroke = Stroke::new(1.0, self.selection_fg);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.editor_bg;
        w.noninteractive.weak_bg_fill = self.editor_bg;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.fg);

        w.inactive.bg_fill = self.widget_bg;
        w.inactive.weak_bg_fill = self.widget_bg;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.widget_fg);

        for state in [&mut w.hovered, &mut w.open] {
            state.bg_fill = hovered_bg;
            state.weak_bg_fill = hovered_bg;
            state.bg_stroke = Stroke::new(1.0, mix(self.accent, hovered_bg, 0.4));
            state.fg_stroke = Stroke::new(1.5, strong);
        }

        let pressed = mix(self.widget_bg, self.accent, 0.45);
        w.active.bg_fill = pressed;
        w.active.weak_bg_fill = pressed;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(2.0, strong);
        v
    }
}

/// Linear blend: `t = 0` gives `a`, `t = 1` gives `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Composite a possibly-transparent color over an opaque background.
fn flatten(c: Color32, under: Color32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        return Color32::from_rgb(r, g, b);
    }
    mix(under, Color32::from_rgb(r, g, b), a as f32 / 255.0)
}

/// Text color that reads well on top of `fill`.
pub fn text_on(fill: Color32) -> Color32 {
    if luma(fill) > 160.0 {
        Color32::from_rgb(20, 20, 24)
    } else {
        Color32::WHITE
    }
}

/// Black or white, whichever reads better on `bg`.
fn luma(c: Color32) -> f32 {
    0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
}

fn contrast_on(bg: Color32) -> Color32 {
    if luma(bg) > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let digits: Vec<u8> = h
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let (r, g, b, a) = match digits.as_slice() {
        [r, g, b] => (r * 17, g * 17, b * 17, 255),
        [r, g, b, a] => (r * 17, g * 17, b * 17, a * 17),
        [r1, r2, g1, g2, b1, b2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255),
        [r1, r2, g1, g2, b1, b2, a1, a2] => {
            (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, a1 * 16 + a2)
        }
        _ => return None,
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

/// Strip `//` and `/* */` comments and trailing commas from JSONC, leaving strings intact.
pub fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    remove_trailing_commas(&out)
}

fn remove_trailing_commas(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut in_str = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

pub fn parse_jsonc(src: &str) -> Result<Value> {
    Ok(serde_json::from_str(&strip_jsonc(src))?)
}

/// One `tokenColors` rule: a TextMate scope selector and its foreground.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenRule {
    /// Descendant parts, e.g. `["meta.structure.dictionary.json", "string"]`.
    pub selector: Vec<String>,
    pub fg: Color32,
}

/// TextMate-style match of a descendant selector against a scope stack (outermost first).
/// The last selector part must match the innermost scope; earlier parts must match outer
/// scopes in order. Returns a specificity score.
fn selector_score(selector: &[String], stack: &[&str]) -> Option<(usize, usize)> {
    let matches = |sel: &str, scope: &str| {
        scope == sel || (scope.starts_with(sel) && scope.as_bytes().get(sel.len()) == Some(&b'.'))
    };
    let (last, outer) = selector.split_last()?;
    let (inner, rest) = stack.split_last()?;
    if !matches(last, inner) {
        return None;
    }
    let mut pos = 0;
    for part in outer {
        pos += rest[pos..].iter().position(|s| matches(part, s))? + 1;
    }
    Some((last.split('.').count(), selector.len()))
}

/// Foreground of the most specific matching rule (later rules win ties, like VS Code).
pub fn resolve_token(rules: &[TokenRule], stack: &[&str]) -> Option<Color32> {
    let mut best: Option<((usize, usize), Color32)> = None;
    for r in rules {
        if let Some(score) = selector_score(&r.selector, stack)
            && best.is_none_or(|(b, _)| score >= b)
        {
            best = Some((score, r.fg));
        }
    }
    best.map(|(_, c)| c)
}

fn parse_token_rules(v: &Value) -> Vec<TokenRule> {
    let mut out = Vec::new();
    for rule in v.as_array().into_iter().flatten() {
        let Some(fg) = rule
            .pointer("/settings/foreground")
            .and_then(Value::as_str)
            .and_then(parse_color)
        else {
            continue;
        };
        let selectors: Vec<String> = match rule.get("scope") {
            Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).collect(),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .map(|x| x.trim().to_string())
                .collect(),
            _ => continue,
        };
        for sel in selectors {
            // Exclusions ("a - b") aren't needed for JSON; skip them rather than misapply.
            if sel.is_empty() || sel.contains(" - ") {
                continue;
            }
            out.push(TokenRule {
                selector: sel.split_whitespace().map(str::to_string).collect(),
                fg,
            });
        }
    }
    out
}

/// A VS Code color theme loaded from disk, with `include`d themes merged in.
#[derive(Clone, Debug)]
pub struct VsCodeTheme {
    pub name: String,
    pub dark: bool,
    pub colors: HashMap<String, Color32>,
    pub tokens: Vec<TokenRule>,
}

impl VsCodeTheme {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_source(&Source::File(path.to_path_buf()))
    }

    /// One of the themes compiled into the app (see [`BUNDLED`]).
    pub fn bundled(file: &str) -> Result<Self> {
        Self::load_source(&Source::Bundled(file.to_string()))
    }

    fn load_source(src: &Source) -> Result<Self> {
        let (mut theme, declared_dark) = Self::load_depth(src, 0)?;
        // VS Code's built-in themes declare their type in the extension manifest rather than
        // the theme file; fall back to the brightness of the editor background.
        theme.dark = declared_dark.unwrap_or_else(|| {
            theme
                .colors
                .get("editor.background")
                .is_none_or(|bg| luma(*bg) < 128.0)
        });
        Ok(theme)
    }

    /// The merged theme plus its declared type (`Some(true)` = dark), if any file has one.
    fn load_depth(path: &Source, depth: usize) -> Result<(Self, Option<bool>)> {
        if depth > 8 {
            bail!("theme include chain is too deep");
        }
        let text = path.read()?;
        let json = parse_jsonc(&text)
            .with_context(|| format!("{} is not a valid VS Code theme file", path))?;
        let obj = json
            .as_object()
            .with_context(|| format!("{} is not a VS Code theme object", path))?;
        if obj.get("colors").is_none() && obj.get("tokenColors").is_none() && depth == 0 {
            bail!("{} has no \"colors\"; is it a VS Code color theme?", path);
        }

        let (mut theme, mut declared_dark) = match obj.get("include").and_then(Value::as_str) {
            Some(inc) => Self::load_depth(&path.include(inc), depth + 1)?,
            None => (
                VsCodeTheme {
                    name: String::new(),
                    dark: true,
                    colors: HashMap::new(),
                    tokens: Vec::new(),
                },
                None,
            ),
        };
        if let Some(name) = obj.get("name").and_then(Value::as_str) {
            theme.name = name.to_string();
        }
        if theme.name.is_empty() {
            theme.name = path.stem();
        }
        if let Some(t) = obj.get("type").and_then(Value::as_str) {
            declared_dark = Some(!t.to_lowercase().contains("light"));
        }
        if let Some(rules) = obj.get("tokenColors") {
            // Rules from this file come after (and so override) included ones.
            theme.tokens.extend(parse_token_rules(rules));
        }
        let colors = obj.get("colors").and_then(Value::as_object);
        for (k, v) in colors.into_iter().flatten() {
            if let Some(c) = v.as_str().and_then(parse_color) {
                theme.colors.insert(k.clone(), c);
            }
        }
        Ok((theme, declared_dark))
    }

    pub fn palette(&self) -> Palette {
        Palette::from_vscode(&self.colors, &self.tokens, self.dark)
    }
}

/// Where a theme file comes from: disk, or compiled into the app.
enum Source {
    File(PathBuf),
    Bundled(String),
}

impl Source {
    fn read(&self) -> Result<String> {
        match self {
            Source::File(p) => {
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
            }
            Source::Bundled(name) => BUNDLED_FILES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, text)| text.to_string())
                .with_context(|| format!("no bundled theme file {name}")),
        }
    }

    /// Resolve an `"include"` relative to this file.
    fn include(&self, rel: &str) -> Source {
        match self {
            Source::File(p) => Source::File(p.parent().unwrap_or(Path::new(".")).join(rel)),
            Source::Bundled(_) => Source::Bundled(rel.trim_start_matches("./").to_string()),
        }
    }

    fn stem(&self) -> String {
        match self {
            Source::File(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Theme".into()),
            Source::Bundled(n) => n.trim_end_matches(".json").to_string(),
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::File(p) => write!(f, "{}", p.display()),
            Source::Bundled(n) => write!(f, "bundled theme {n}"),
        }
    }
}

macro_rules! bundled_files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../assets/themes/", $name)))),*]
    };
}

/// Theme files compiled into the app (MIT licensed; see assets/themes/NOTICE.md).
const BUNDLED_FILES: &[(&str, &str)] = bundled_files![
    "dark_modern.json",
    "dark_plus.json",
    "dark_vs.json",
    "light_modern.json",
    "light_plus.json",
    "light_vs.json",
    "monokai-color-theme.json",
    "solarized-dark-color-theme.json",
    "solarized-light-color-theme.json",
    "night-owl-color-theme.json",
];

/// The preloaded themes: (label, file, dark).
pub const BUNDLED: &[(&str, &str, bool)] = &[
    ("Dark Modern", "dark_modern.json", true),
    ("Monokai", "monokai-color-theme.json", true),
    ("Night Owl", "night-owl-color-theme.json", true),
    ("Solarized Dark", "solarized-dark-color-theme.json", true),
    ("Light Modern", "light_modern.json", false),
    ("Solarized Light", "solarized-light-color-theme.json", false),
];

/// License notices for the bundled themes, shown in Settings.
pub const BUNDLED_LICENSES: &[(&str, &str)] = &[
    (
        "Dark Modern, Light Modern, Monokai, Solarized (from Visual Studio Code)",
        include_str!("../assets/themes/LICENSE-vscode.txt"),
    ),
    (
        "Night Owl by Sarah Drasner",
        include_str!("../assets/themes/LICENSE-night-owl.md"),
    ),
];

/// Light or dark mode. System follows the OS setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// A VS Code color theme: preloaded, or a file (imported or installed in an editor).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeRef {
    /// A preloaded theme, by file name in [`BUNDLED`].
    Bundled(String),
    File {
        label: String,
        path: PathBuf,
        dark: bool,
    },
}

impl ThemeRef {
    pub fn default_dark() -> Self {
        ThemeRef::Bundled("dark_modern.json".into())
    }

    pub fn default_light() -> Self {
        ThemeRef::Bundled("light_modern.json".into())
    }

    pub fn bundled() -> impl Iterator<Item = ThemeRef> {
        BUNDLED
            .iter()
            .map(|(_, f, _)| ThemeRef::Bundled(f.to_string()))
    }

    pub fn label(&self) -> &str {
        match self {
            ThemeRef::Bundled(file) => BUNDLED
                .iter()
                .find(|(_, f, _)| f == file)
                .map(|(l, _, _)| *l)
                .unwrap_or(file),
            ThemeRef::File { label, .. } => label,
        }
    }

    pub fn dark(&self) -> bool {
        match self {
            ThemeRef::Bundled(file) => BUNDLED
                .iter()
                .find(|(_, f, _)| f == file)
                .is_none_or(|(_, _, d)| *d),
            ThemeRef::File { dark, .. } => *dark,
        }
    }

    pub fn load(&self) -> Result<VsCodeTheme> {
        match self {
            ThemeRef::Bundled(file) => VsCodeTheme::bundled(file),
            ThemeRef::File { path, .. } => VsCodeTheme::load(path),
        }
    }
}

/// Folders where VS Code-family editors keep their extensions (each may contain themes).
fn extension_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(home) = &home {
        for editor in [
            ".vscode",
            ".vscode-insiders",
            ".cursor",
            ".windsurf",
            ".vscode-oss",
        ] {
            dirs.push(home.join(editor).join("extensions"));
        }
    }
    // Themes bundled with the editors themselves.
    for app in [
        "/Applications/Visual Studio Code.app/Contents/Resources/app/extensions",
        "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/extensions",
        "/Applications/Cursor.app/Contents/Resources/app/extensions",
        "/Applications/Windsurf.app/Contents/Resources/app/extensions",
        "/Applications/VSCodium.app/Contents/Resources/app/extensions",
    ] {
        dirs.push(PathBuf::from(app));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for app in [
            "Microsoft VS Code",
            "Microsoft VS Code Insiders",
            "cursor",
            "Windsurf",
        ] {
            dirs.push(
                local
                    .join("Programs")
                    .join(app)
                    .join("resources/app/extensions"),
            );
        }
    }
    dirs
}

/// Themes contributed by one extension folder (reads its package.json).
pub fn themes_in_extension(ext: &Path) -> Vec<ThemeRef> {
    let Ok(text) = std::fs::read_to_string(ext.join("package.json")) else {
        return Vec::new();
    };
    let Ok(pkg) = parse_jsonc(&text) else {
        return Vec::new();
    };
    let nls: HashMap<String, String> = std::fs::read_to_string(ext.join("package.nls.json"))
        .ok()
        .and_then(|t| parse_jsonc(&t).ok())
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();
    pkg.pointer("/contributes/themes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let path = ext.join(t.get("path")?.as_str()?);
            let raw = t
                .get("label")
                .or_else(|| t.get("id"))
                .and_then(Value::as_str)?;
            // Localized labels look like "%colorTheme.label%".
            let label = raw
                .strip_prefix('%')
                .and_then(|k| k.strip_suffix('%'))
                .and_then(|k| nls.get(k).cloned())
                .unwrap_or_else(|| raw.to_string());
            // uiTheme: "vs" (light), "vs-dark", "hc-black", "hc-light".
            let dark = !matches!(
                t.get("uiTheme").and_then(Value::as_str),
                Some("vs") | Some("hc-light")
            );
            path.is_file()
                .then_some(ThemeRef::File { label, path, dark })
        })
        .collect()
}

/// All color themes installed in VS Code-family editors on this machine.
pub fn discover_installed() -> Vec<ThemeRef> {
    let mut found = Vec::new();
    for dir in extension_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if e.path().is_dir() {
                found.extend(themes_in_extension(&e.path()));
            }
        }
    }
    found.sort_by_key(|t| t.label().to_lowercase());
    found.dedup_by(|a, b| a.label() == b.label());
    found
}

/// Make the palette available to widgets that need the success color (no egui slot).
pub fn store_palette(ctx: &egui::Context, p: &Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL.with("palette"), p.clone()));
}

pub fn palette(ui: &egui::Ui) -> Palette {
    ui.data(|d| d.get_temp::<Palette>(egui::Id::NULL.with("palette")))
        .unwrap_or_else(|| {
            if ui.visuals().dark_mode {
                Palette::dark()
            } else {
                Palette::light()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(
            parse_color("#102030"),
            Some(Color32::from_rgb(0x10, 0x20, 0x30))
        );
        assert_eq!(
            parse_color("#10203080"),
            Some(Color32::from_rgba_unmultiplied(0x10, 0x20, 0x30, 0x80))
        );
        assert_eq!(
            parse_color("#0000"),
            Some(Color32::from_rgba_unmultiplied(0, 0, 0, 0))
        );
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(
            flatten(
                Color32::from_rgba_unmultiplied(255, 255, 255, 0),
                Color32::BLACK
            ),
            Color32::BLACK
        );
    }

    #[test]
    fn jsonc() {
        let src = r#"{
            // line comment
            "a": "http://x.y/z", /* block
            comment */ "b": [1, 2,],
            "c": "quote \" and // not a comment",
        }"#;
        let v = parse_jsonc(src).unwrap();
        assert_eq!(v["a"], "http://x.y/z");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
        assert_eq!(v["c"], "quote \" and // not a comment");
    }

    #[test]
    fn loads_theme_with_include() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("base.json"),
            r##"{ "type": "light", "colors": { "editor.background": "#fafafa", "foreground": "#111111" } }"##,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("child.json"),
            r##"{
                // comments are fine
                "name": "Child",
                "include": "./base.json",
                "colors": { "editor.background": "#eeeeee", "button.background": "#ff0000", },
            }"##,
        )
        .unwrap();
        let t = VsCodeTheme::load(&dir.path().join("child.json")).unwrap();
        assert_eq!(t.name, "Child");
        assert!(!t.dark, "type comes from the included theme");
        let p = t.palette();
        assert_eq!(p.editor_bg, hex(0xeeeeee));
        assert_eq!(p.fg, hex(0x111111));
        assert_eq!(p.accent, hex(0xff0000));
        assert_eq!(p.accent_fg, Color32::WHITE);
        // Unspecified colors fall back to the built-in light palette.
        assert_eq!(p.error, Palette::light().error);
    }

    #[test]
    fn infers_type_from_background_when_undeclared() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.json");
        std::fs::write(&f, r##"{ "colors": { "editor.background": "#fdfdfd" } }"##).unwrap();
        assert!(!VsCodeTheme::load(&f).unwrap().dark);
        std::fs::write(&f, r##"{ "colors": { "editor.background": "#101010" } }"##).unwrap();
        assert!(VsCodeTheme::load(&f).unwrap().dark);
    }

    #[test]
    fn bundled_themes_load_with_expected_type() {
        for (label, file, dark) in BUNDLED {
            let t = VsCodeTheme::bundled(file).unwrap_or_else(|e| panic!("{label}: {e:#}"));
            assert!(t.colors.len() > 10, "{label} has colors");
            assert_eq!(t.dark, *dark, "{label} light/dark");
        }
    }

    #[test]
    fn token_selectors() {
        let rule = |sel: &str, c: u32| TokenRule {
            selector: sel.split_whitespace().map(str::to_string).collect(),
            fg: hex(c),
        };
        let rules = vec![
            rule("string", 0x111111),
            rule(
                "meta.structure.dictionary.json string.quoted.double.json",
                0x222222,
            ),
            rule("support.type.property-name", 0x333333),
            rule("support.type.property-name.json", 0x444444),
            rule("constant.numeric", 0x555555),
        ];
        let st = |inner| ["source.json", "meta.structure.dictionary.json", inner];
        assert_eq!(
            resolve_token(&rules, &st("string.quoted.double.json")),
            Some(hex(0x222222))
        );
        assert_eq!(
            resolve_token(&rules, &st("support.type.property-name.json")),
            Some(hex(0x444444))
        );
        assert_eq!(
            resolve_token(&rules, &st("constant.numeric.json")),
            Some(hex(0x555555))
        );
        assert_eq!(resolve_token(&rules, &st("constant.language.json")), None);
        // "string" must not match "strings.foo"
        assert_eq!(resolve_token(&rules, &["stringy"]), None);
    }

    #[test]
    fn bundled_themes_have_distinct_json_colors() {
        let dark = VsCodeTheme::bundled("dark_modern.json").unwrap().palette();
        assert_eq!(dark.syntax.key, hex(0x9cdcfe));
        assert_eq!(dark.syntax.string, hex(0xce9178));
        let owl = VsCodeTheme::bundled("night-owl-color-theme.json")
            .unwrap()
            .palette();
        assert_ne!(owl.syntax.string, owl.syntax.plain);
        assert_ne!(owl.syntax.number, owl.syntax.plain);
    }

    #[test]
    fn rejects_non_theme_json() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.json");
        std::fs::write(&f, r#"{ "hello": 1 }"#).unwrap();
        assert!(VsCodeTheme::load(&f).is_err());
        std::fs::write(&f, "not json").unwrap();
        assert!(VsCodeTheme::load(&f).is_err());
    }

    #[test]
    fn discovers_extension_themes_with_localized_labels() {
        let dir = tempfile::tempdir().unwrap();
        let ext = dir.path();
        std::fs::create_dir(ext.join("themes")).unwrap();
        std::fs::write(ext.join("themes/a.json"), r#"{"colors":{}}"#).unwrap();
        std::fs::write(
            ext.join("package.json"),
            r#"{ "contributes": { "themes": [
                { "label": "%theme.a%", "uiTheme": "vs-dark", "path": "./themes/a.json" },
                { "label": "Missing", "uiTheme": "vs", "path": "./themes/missing.json" }
            ] } }"#,
        )
        .unwrap();
        std::fs::write(ext.join("package.nls.json"), r#"{ "theme.a": "Theme A" }"#).unwrap();
        let found = themes_in_extension(ext);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label(), "Theme A");
    }

    /// Run against real theme files: SECRETS_VSCODE_THEMES=<dir of *.json> cargo test -- --ignored
    #[test]
    #[ignore]
    fn loads_real_themes() {
        let dir = std::env::var("SECRETS_VSCODE_THEMES").expect("set SECRETS_VSCODE_THEMES");
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let t = VsCodeTheme::load(&e.path()).unwrap_or_else(|e| panic!("{e:#}"));
            println!("{}: dark={} colors={}", t.name, t.dark, t.colors.len());
            assert!(!t.colors.is_empty());
        }
    }
}
