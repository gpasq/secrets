//! JSON syntax highlighting for the value editors, colored by the active theme.
//!
//! The lexer is deliberately forgiving: it runs on every keystroke, so it must color
//! half-typed or invalid JSON sensibly instead of giving up.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

/// Colors for each JSON token kind (from the theme's `tokenColors`).
#[derive(Clone, Debug, PartialEq)]
pub struct Syntax {
    pub key: Color32,
    pub string: Color32,
    pub number: Color32,
    pub literal: Color32,
    pub punct: Color32,
    pub plain: Color32,
    // For sample code in other languages.
    pub keyword: Color32,
    pub comment: Color32,
    pub type_: Color32,
    pub function: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Key,
    String,
    Number,
    Literal,
    Punct,
    Space,
    Other,
}

/// Split `src` into `(kind, text)` tokens. Concatenating the texts gives back `src`.
pub fn tokenize(src: &str) -> Vec<(Kind, &str)> {
    let bytes = src.as_bytes();
    let mut out: Vec<(Kind, &str)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                if i < bytes.len() && bytes[i] == b'"' {
                    i += 1;
                }
                Kind::String
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                i += 1;
                Kind::Punct
            }
            b if b.is_ascii_whitespace() => {
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                Kind::Space
            }
            b'-' | b'0'..=b'9' => {
                i += 1;
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    i += 1;
                }
                Kind::Number
            }
            _ => {
                // A bare word: true/false/null or something invalid.
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'"' | b'{' | b'}' | b'[' | b']' | b':' | b',')
                {
                    i += 1;
                }
                match &src[start..i] {
                    "true" | "false" | "null" => Kind::Literal,
                    _ => Kind::Other,
                }
            }
        };
        let i_clamped = i.min(bytes.len());
        // Never split a UTF-8 character (the escape skip above can overshoot by one).
        let mut end = i_clamped;
        while !src.is_char_boundary(end) {
            end += 1;
        }
        i = end;
        out.push((kind, &src[start..end]));
    }
    // A string followed (after whitespace) by ':' is an object key.
    for idx in 0..out.len() {
        if out[idx].0 == Kind::String {
            let next = out[idx + 1..].iter().find(|(k, _)| *k != Kind::Space);
            if matches!(next, Some((Kind::Punct, ":"))) {
                out[idx].0 = Kind::Key;
            }
        }
    }
    out
}

/// Text that is (or is being typed as) a JSON object or array.
pub fn looks_like_json(text: &str) -> bool {
    matches!(text.trim_start().chars().next(), Some('{') | Some('['))
}

pub fn json_job(src: &str, syntax: &Syntax, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (kind, text) in tokenize(src) {
        let color = match kind {
            Kind::Key => syntax.key,
            Kind::String => syntax.string,
            Kind::Number => syntax.number,
            Kind::Literal => syntax.literal,
            Kind::Punct => syntax.punct,
            Kind::Space | Kind::Other => syntax.plain,
        };
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    }
    job
}

/// Lexical rules for highlighting sample code.
struct LangRules {
    line_comment: &'static [&'static str],
    block_comment: bool,
    backtick_strings: bool,
    keywords: &'static [&'static str],
}

fn rules(lang: crate::samples::Lang) -> LangRules {
    use crate::samples::Lang::*;
    const JS: &[&str] = &[
        "import",
        "from",
        "export",
        "const",
        "let",
        "var",
        "new",
        "await",
        "async",
        "function",
        "return",
        "interface",
        "type",
        "if",
        "else",
        "undefined",
        "null",
        "true",
        "false",
    ];
    match lang {
        JavaScript | TypeScript => LangRules {
            line_comment: &["//"],
            block_comment: true,
            backtick_strings: true,
            keywords: JS,
        },
        Java => LangRules {
            line_comment: &["//"],
            block_comment: true,
            backtick_strings: false,
            keywords: &[
                "import", "public", "class", "static", "void", "throws", "try", "new", "return",
                "byte", "null", "true", "false",
            ],
        },
        Rust => LangRules {
            line_comment: &["//"],
            block_comment: true,
            backtick_strings: false,
            keywords: &[
                "use", "fn", "let", "mut", "async", "await", "pub", "struct", "impl", "match",
                "return", "dyn", "Ok", "Err", "Some", "None", "true", "false",
            ],
        },
        Go => LangRules {
            line_comment: &["//"],
            block_comment: true,
            backtick_strings: true,
            keywords: &[
                "package", "import", "func", "var", "if", "err", "nil", "return", "any", "map",
                "string", "byte", "true", "false",
            ],
        },
        Python => LangRules {
            line_comment: &["#"],
            block_comment: false,
            backtick_strings: false,
            keywords: &[
                "import", "from", "as", "def", "return", "None", "True", "False", "print",
            ],
        },
        CSharp => LangRules {
            line_comment: &["//"],
            block_comment: true,
            backtick_strings: false,
            keywords: &[
                "using", "var", "new", "await", "async", "return", "null", "true", "false",
            ],
        },
        Ruby => LangRules {
            line_comment: &["#"],
            block_comment: false,
            backtick_strings: false,
            keywords: &["require", "puts", "def", "end", "nil", "true", "false"],
        },
        Php => LangRules {
            line_comment: &["//", "#"],
            block_comment: true,
            backtick_strings: false,
            keywords: &[
                "require", "use", "new", "echo", "true", "false", "null", "php", "PHP_EOL",
            ],
        },
    }
}

/// Highlight sample code: comments, strings, numbers, keywords, types, function calls.
pub fn code_job(src: &str, lang: crate::samples::Lang, syntax: &Syntax, font: FontId) -> LayoutJob {
    let r = rules(lang);
    let mut job = LayoutJob::default();
    let mut push = |text: &str, color: Color32| {
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    };
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &src[i..];
        let start = i;
        if r.line_comment.iter().any(|m| rest.starts_with(*m)) {
            let end = rest.find('\n').map_or(src.len(), |n| i + n);
            push(&src[start..end], syntax.comment);
            i = end;
            continue;
        }
        if r.block_comment && rest.starts_with("/*") {
            let end = rest.find("*/").map_or(src.len(), |n| i + n + 2);
            push(&src[start..end], syntax.comment);
            i = end;
            continue;
        }
        let c = bytes[i];
        if c == b'"' || c == b'\'' || (c == b'`' && r.backtick_strings) {
            i += 1;
            while i < bytes.len() && bytes[i] != c {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
            while !src.is_char_boundary(i) {
                i += 1;
            }
            push(&src[start..i], syntax.string);
            continue;
        }
        if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            push(&src[start..i], syntax.number);
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let next = src[i..].trim_start_matches(' ').chars().next();
            let color = if r.keywords.contains(&word) {
                syntax.keyword
            } else if next == Some('(') || (next == Some('!') && lang == crate::samples::Lang::Rust)
            {
                syntax.function
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                syntax.type_
            } else {
                syntax.plain
            };
            push(word, color);
            continue;
        }
        // Anything else: one character (keeping UTF-8 intact).
        let ch_len = rest.chars().next().map_or(1, char::len_utf8);
        i += ch_len;
        let color = if c.is_ascii_punctuation() {
            syntax.punct
        } else {
            syntax.plain
        };
        push(&src[start..i], color);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(Kind, String)> {
        tokenize(src)
            .into_iter()
            .filter(|(k, _)| *k != Kind::Space)
            .map(|(k, t)| (k, t.to_string()))
            .collect()
    }

    #[test]
    fn classifies_tokens() {
        use Kind::*;
        let k = kinds(r#"{"user": "a\"b", "n": -1.5e3, "ok": true, "x": null, "l": [1, false]}"#);
        let expected = vec![
            (Punct, "{"),
            (Key, r#""user""#),
            (Punct, ":"),
            (String, r#""a\"b""#),
            (Punct, ","),
            (Key, r#""n""#),
            (Punct, ":"),
            (Number, "-1.5e3"),
            (Punct, ","),
            (Key, r#""ok""#),
            (Punct, ":"),
            (Literal, "true"),
            (Punct, ","),
            (Key, r#""x""#),
            (Punct, ":"),
            (Literal, "null"),
            (Punct, ","),
            (Key, r#""l""#),
            (Punct, ":"),
            (Punct, "["),
            (Number, "1"),
            (Punct, ","),
            (Literal, "false"),
            (Punct, "]"),
            (Punct, "}"),
        ];
        let expected: Vec<(Kind, std::string::String)> = expected
            .into_iter()
            .map(|(k, t)| (k, t.to_string()))
            .collect();
        assert_eq!(k, expected);
    }

    #[test]
    fn round_trips_any_input() {
        for src in [
            "",
            "{\"unterminated",
            "{\"a\": tru",
            "  [1,\n 2 ]\n",
            "\"ends with backslash\\",
            "{\"é\": \"日本\\",
            "garbage here",
        ] {
            let joined: String = tokenize(src).into_iter().map(|(_, t)| t).collect();
            assert_eq!(joined, src);
        }
    }

    fn syntax() -> Syntax {
        let c = |v: u8| Color32::from_rgb(v, v, v);
        Syntax {
            key: c(1),
            string: c(2),
            number: c(3),
            literal: c(4),
            punct: c(5),
            plain: c(6),
            keyword: c(7),
            comment: c(8),
            type_: c(9),
            function: c(10),
        }
    }

    #[test]
    fn highlights_code_and_keeps_text() {
        use crate::samples::{Lang, Shape, sample};
        let font = FontId::monospace(14.0);
        for lang in Lang::ALL {
            let src = sample(lang, "prod/db", "us-east-1", &Shape::Text);
            let job = code_job(&src, lang, &syntax(), font.clone());
            assert_eq!(job.text, src, "{lang:?} text is unchanged");
            let colors: std::collections::HashSet<_> =
                job.sections.iter().map(|s| s.format.color).collect();
            for (what, c) in [("string", 2), ("comment", 8), ("keyword", 7)] {
                assert!(
                    colors.contains(&Color32::from_rgb(c, c, c)),
                    "{lang:?} has {what} highlighting"
                );
            }
        }
    }

    #[test]
    fn detects_json_like_text() {
        assert!(looks_like_json("  {\"a\":1}"));
        assert!(looks_like_json("[1]"));
        assert!(!looks_like_json("password123"));
        assert!(!looks_like_json(""));
    }
}
