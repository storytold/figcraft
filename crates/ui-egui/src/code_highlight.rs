//! The colours the code in the Inspect panel is shown with.
//!
//! A small tokenizer that knows the three shapes the panel generates — the curly-brace ones
//! (TypeScript, Swift, Kotlin, Dart, Rust), the markup ones (HTML, XML, JSX, the single-file
//! components) and the stylesheet ones (CSS, SCSS) — so every language is coloured the same
//! way, with the same palette.

use egui::Color32;
use figcraft_engine::codegen::CodeLang;

/// The colours a piece of code is shown with.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub plain: Color32,
    pub comment: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub function: Color32,
    pub type_name: Color32,
    pub tag: Color32,
    pub attribute: Color32,
    pub property: Color32,
    pub punctuation: Color32,
}

impl Palette {
    /// The palette of the dark theme.
    pub fn dark() -> Palette {
        Palette {
            dark: true,
            plain: Color32::from_rgb(208, 200, 198),
            comment: Color32::from_rgb(126, 112, 90),
            keyword: Color32::from_rgb(168, 155, 185),
            string: Color32::from_rgb(190, 181, 91),
            number: Color32::from_rgb(210, 139, 113),
            function: Color32::from_rgb(138, 179, 181),
            type_name: Color32::from_rgb(244, 188, 135),
            tag: Color32::from_rgb(203, 96, 119),
            attribute: Color32::from_rgb(210, 139, 113),
            property: Color32::from_rgb(123, 189, 164),
            punctuation: Color32::from_rgb(208, 200, 198),
        }
    }

    /// The palette of the light theme.
    pub fn light() -> Palette {
        Palette {
            dark: false,
            plain: Color32::from_rgb(88, 110, 117),
            comment: Color32::from_rgb(147, 161, 161),
            keyword: Color32::from_rgb(108, 113, 196),
            string: Color32::from_rgb(42, 161, 152),
            number: Color32::from_rgb(203, 75, 22),
            function: Color32::from_rgb(38, 139, 210),
            type_name: Color32::from_rgb(181, 137, 0),
            tag: Color32::from_rgb(220, 50, 47),
            attribute: Color32::from_rgb(203, 75, 22),
            property: Color32::from_rgb(133, 153, 0),
            punctuation: Color32::from_rgb(88, 110, 117),
        }
    }

    /// The palette of the theme the panel is shown in: the plain text, the comments, the
    /// punctuation and the property names take their colour from the theme tokens, the rest keep
    /// their tuned hues.
    pub fn from_tokens(t: &crate::theme::Tokens) -> Palette {
        let dark = t.dark;
        let base = if dark { Palette::dark() } else { Palette::light() };
        Palette { dark, plain: t.text, comment: t.text_tertiary, punctuation: t.text_secondary, property: t.success, ..base }
    }
}

/// The kinds of piece the code is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Tag,
    Attribute,
    Property,
    Variable,
    Punctuation,
}

impl Kind {
    /// The colour a piece of this kind is shown with.
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Kind::Plain => p.plain,
            Kind::Keyword => p.keyword,
            Kind::String => p.string,
            Kind::Number => p.number,
            Kind::Comment => p.comment,
            Kind::Function => p.function,
            Kind::Type => p.type_name,
            Kind::Tag => p.tag,
            Kind::Attribute => p.attribute,
            Kind::Property => p.property,
            Kind::Variable => p.function,
            Kind::Punctuation => p.punctuation,
        }
    }
}

/// The shape of a language's code.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// The curly-brace ones.
    Code,
    /// The markup ones (HTML, XML): the text between the tags is text.
    Markup,
    /// The JSX ones: the text between the tags is code, the text inside them is text.
    Jsx,
    /// The stylesheet ones.
    Styles,
    /// A single-file component: markup with `<script>` and `<style>` sections.
    Component,
}

/// The keywords of a language, for the tokenizer.
struct Keywords {
    words: &'static [&'static str],
    /// Whether a name that starts with a capital is a type.
    types: bool,
    /// Whether a `'` opens a string (Rust and Swift use it for lifetimes).
    single_quote: bool,
}

const TS: Keywords = Keywords {
    words: &[
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "yield",
    ],
    types: true,
    single_quote: true,
};

const SWIFT: Keywords = Keywords {
    words: &[
        "as",
        "associatedtype",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "convenience",
        "default",
        "defer",
        "deinit",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "false",
        "fileprivate",
        "final",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "nil",
        "open",
        "operator",
        "override",
        "private",
        "protocol",
        "public",
        "repeat",
        "required",
        "return",
        "self",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "typealias",
        "var",
        "weak",
        "where",
        "while",
    ],
    types: true,
    single_quote: false,
};

const KOTLIN: Keywords = Keywords {
    words: &[
        "abstract",
        "as",
        "break",
        "by",
        "catch",
        "class",
        "companion",
        "const",
        "constructor",
        "continue",
        "data",
        "do",
        "else",
        "enum",
        "false",
        "final",
        "finally",
        "for",
        "fun",
        "get",
        "if",
        "import",
        "in",
        "init",
        "interface",
        "internal",
        "is",
        "lateinit",
        "null",
        "object",
        "open",
        "operator",
        "out",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "sealed",
        "set",
        "super",
        "this",
        "throw",
        "true",
        "try",
        "typealias",
        "val",
        "var",
        "vararg",
        "when",
        "while",
    ],
    types: true,
    single_quote: true,
};

const DART: Keywords = Keywords {
    words: &[
        "abstract",
        "as",
        "assert",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "covariant",
        "default",
        "deferred",
        "do",
        "dynamic",
        "else",
        "enum",
        "export",
        "extends",
        "extension",
        "external",
        "factory",
        "false",
        "final",
        "finally",
        "for",
        "get",
        "if",
        "implements",
        "import",
        "in",
        "is",
        "late",
        "library",
        "mixin",
        "new",
        "null",
        "on",
        "operator",
        "part",
        "required",
        "rethrow",
        "return",
        "sealed",
        "set",
        "show",
        "static",
        "super",
        "switch",
        "sync",
        "this",
        "throw",
        "true",
        "try",
        "typedef",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    types: true,
    single_quote: true,
};

const RUST: Keywords = Keywords {
    words: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
        "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
        "unsafe", "use", "where", "while",
    ],
    types: true,
    single_quote: false,
};

/// The shape and the keywords of a language.
fn language(lang: CodeLang) -> (Shape, &'static Keywords) {
    match lang {
        CodeLang::Css | CodeLang::Scss => (Shape::Styles, &TS),
        CodeLang::Html | CodeLang::AndroidXml => (Shape::Markup, &TS),
        CodeLang::React | CodeLang::StyledComponents | CodeLang::Tailwind => (Shape::Jsx, &TS),
        CodeLang::Vue | CodeLang::Angular | CodeLang::Svelte => (Shape::Component, &TS),
        CodeLang::SwiftUi | CodeLang::SwiftUiKit => (Shape::Code, &SWIFT),
        CodeLang::Compose => (Shape::Code, &KOTLIN),
        CodeLang::Flutter => (Shape::Code, &DART),
        CodeLang::Leptos => (Shape::Code, &RUST),
    }
}

/// The code as coloured pieces: each one the text and the kind it is.
pub fn pieces(code: &str, lang: CodeLang) -> Vec<(Kind, &str)> {
    let (shape, kw) = language(lang);
    let mut out = Vec::new();
    match shape {
        Shape::Code => code_tokens(code, kw, &mut out),
        Shape::Markup => markup_tokens(code, kw, &mut out),
        Shape::Jsx => jsx_tokens(code, kw, &mut out),
        Shape::Styles => style_tokens(code, &mut out),
        Shape::Component => component_tokens(code, kw, &mut out),
    }
    out
}

/// The curly-brace languages: keywords, strings, comments, numbers and names.
fn code_tokens<'a>(code: &'a str, kw: &Keywords, out: &mut Vec<(Kind, &'a str)>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let char_len = code[i..].chars().next().map_or(1, char::len_utf8);
        let _ = char_len;
        let c = code[i..].chars().next().unwrap();
        let _ = char_len;
        let start = i;
        if c == '/' && bytes.get(i + 1) == Some(&b'/') {
            i = line_end(bytes, i);
            out.push((Kind::Comment, &code[start..i]));
            continue;
        }
        if c == '/' && bytes.get(i + 1) == Some(&b'*') {
            i = block_end(bytes, i);
            out.push((Kind::Comment, &code[start..i]));
            continue;
        }
        if c == '"' || c == '`' || (c == '\'' && kw.single_quote) {
            i = string_end(bytes, i);
            out.push((Kind::String, &code[start..i]));
            continue;
        }
        if c == '@' || c == '#' {
            let word = word_end(bytes, i + 1);
            if word > i + 1 {
                out.push((Kind::Punctuation, &code[start..i + 1]));
                out.push((Kind::Type, &code[i + 1..word]));
                i = word;
                continue;
            }
        }
        if c.is_ascii_digit() {
            i = number_end(bytes, i);
            out.push((Kind::Number, &code[start..i]));
            continue;
        }
        if is_word_start(c) {
            let word = word_end(bytes, i);
            let text = &code[i..word];
            let kind = if kw.words.contains(&text) {
                Kind::Keyword
            } else if bytes.get(word).is_some_and(|b| *b == b'(') {
                Kind::Function
            } else if kw.types && text.chars().next().is_some_and(|f| f.is_uppercase()) {
                Kind::Type
            } else {
                Kind::Plain
            };
            out.push((kind, text));
            i = word;
            continue;
        }
        if c.is_whitespace() {
            i = whitespace_end(bytes, i);
            out.push((Kind::Plain, &code[start..i]));
            continue;
        }
        i += char_len;
        out.push((Kind::Punctuation, &code[start..i]));
    }
}

/// The JSX ones: the tags as markup, everything around them as code.
fn jsx_tokens<'a>(code: &'a str, kw: &Keywords, out: &mut Vec<(Kind, &'a str)>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    // Inside a tag's children the text is text; around it, it is code.
    let mut children = false;
    while i < bytes.len() {
        let char_len = code[i..].chars().next().map_or(1, char::len_utf8);
        let _ = char_len;
        let c = code[i..].chars().next().unwrap();
        let _ = char_len;
        if c == '<' {
            let end = tag_end(bytes, i);
            let tag = &code[i..end];
            tag_tokens(tag, out);
            children = !tag.starts_with("</") && !tag.ends_with("/>");
            i = end;
            continue;
        }
        if c == '{' {
            let end = brace_end(bytes, i);
            out.push((Kind::Punctuation, &code[i..i + 1]));
            code_tokens(&code[i + 1..end.saturating_sub(1)], kw, out);
            if end > i + 1 {
                out.push((Kind::Punctuation, &code[end - 1..end]));
            }
            i = end;
            children = false;
            continue;
        }
        if children {
            let start = i;
            while i < bytes.len() && bytes[i] != b'<' && bytes[i] != b'{' {
                i += char_len;
            }
            out.push((Kind::Plain, &code[start..i]));
            continue;
        }
        // A run of code: it is coloured as a whole, so a name can be seen with the `(` after it.
        let start = i;
        while i < bytes.len() && bytes[i] != b'<' && bytes[i] != b'{' {
            i += char_len;
        }
        code_tokens(&code[start..i], kw, out);
    }
}
/// The markup languages: tags, their attributes and the text between them. A `{…}` holds code.
fn markup_tokens<'a>(code: &'a str, kw: &Keywords, out: &mut Vec<(Kind, &'a str)>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut text = 0;
    while i < bytes.len() {
        let char_len = code[i..].chars().next().map_or(1, char::len_utf8);
        let _ = char_len;
        let c = code[i..].chars().next().unwrap();
        let _ = char_len;
        if c == '<' && bytes.get(i + 1) == Some(&b'!') && code[i..].starts_with("<!--") {
            if i > text {
                out.push((Kind::Plain, &code[text..i]));
            }
            let end = code[i..].find("-->").map_or(bytes.len(), |k| i + k + 3);
            out.push((Kind::Comment, &code[i..end]));
            i = end;
            text = i;
            continue;
        }
        if c == '<' {
            if i > text {
                out.push((Kind::Plain, &code[text..i]));
            }
            let end = tag_end(bytes, i);
            tag_tokens(&code[i..end], out);
            i = end;
            text = i;
            continue;
        }
        if c == '{' {
            if i > text {
                out.push((Kind::Plain, &code[text..i]));
            }
            let end = brace_end(bytes, i);
            out.push((Kind::Punctuation, &code[i..i + 1]));
            code_tokens(&code[i + 1..end.saturating_sub(1)], kw, out);
            if end > i + 1 {
                out.push((Kind::Punctuation, &code[end - 1..end]));
            }
            i = end;
            text = i;
            continue;
        }
        i += char_len;
    }
    if text < bytes.len() {
        out.push((Kind::Plain, &code[text..]));
    }
}

/// A tag: its name, its attributes and their values.
fn tag_tokens<'a>(tag: &'a str, out: &mut Vec<(Kind, &'a str)>) {
    let bytes = tag.as_bytes();
    let mut i = 0;
    // The name right after the `<` is the tag; the rest are its attributes.
    let mut named = false;
    while i < bytes.len() {
        let char_len = tag[i..].chars().next().map_or(1, char::len_utf8);
        let c = tag[i..].chars().next().unwrap();
        let start = i;
        if c == '<' {
            i += char_len;
            named = false;
            out.push((Kind::Punctuation, &tag[start..i]));
            continue;
        }
        if c == '>' || (c == '/' && bytes.get(i + 1) == Some(&b'>')) {
            let end = if c == '/' { i + 2 } else { i + 1 };
            out.push((Kind::Punctuation, &tag[start..end]));
            i = end;
            continue;
        }
        if c == '"' || c == '\'' {
            i = string_end(bytes, i);
            out.push((Kind::String, &tag[start..i]));
            continue;
        }
        if c == '=' {
            i += char_len;
            out.push((Kind::Punctuation, &tag[start..i]));
            continue;
        }
        if is_word_start(c) {
            let word = word_end(bytes, i);
            out.push((if named { Kind::Attribute } else { Kind::Tag }, &tag[i..word]));
            named = true;
            i = word;
            continue;
        }
        i += char_len;
        out.push((Kind::Plain, &tag[start..i]));
    }
}

/// A single-file component: the markup, with the script and the style sections coloured as code
/// and as a stylesheet.
fn component_tokens<'a>(code: &'a str, kw: &Keywords, out: &mut Vec<(Kind, &'a str)>) {
    let lower = code.to_ascii_lowercase();
    let mut i = 0;
    while i < code.len() {
        let script = lower[i..].find("<script").map(|k| i + k);
        let style = lower[i..].find("<style").map(|k| i + k);
        let next = match (script, style) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(at) = next else { break };
        if at > i {
            markup_tokens(&code[i..at], kw, out);
        }
        let section = if lower[at..].starts_with("<script") { "script" } else { "style" };
        let close = format!("</{section}");
        let end = lower[at..].find(&close).map_or(code.len(), |k| at + k);
        if section == "script" {
            code_tokens(&code[at..end], kw, out);
        } else {
            style_tokens(&code[at..end], out);
        }
        i = end;
    }
    if i < code.len() {
        markup_tokens(&code[i..], kw, out);
    }
}

/// The stylesheet languages: selectors, properties and their values.
fn style_tokens<'a>(code: &'a str, out: &mut Vec<(Kind, &'a str)>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    // Inside a block the words before a `:` are properties; outside they are selectors.
    let mut depth = 0usize;
    while i < bytes.len() {
        let char_len = code[i..].chars().next().map_or(1, char::len_utf8);
        let _ = char_len;
        let c = code[i..].chars().next().unwrap();
        let _ = char_len;
        let start = i;
        if c == '/' && bytes.get(i + 1) == Some(&b'*') {
            i = block_end(bytes, i);
            out.push((Kind::Comment, &code[start..i]));
            continue;
        }
        if c == '"' || c == '\'' {
            i = string_end(bytes, i);
            out.push((Kind::String, &code[start..i]));
            continue;
        }
        if c == '{' {
            depth += 1;
            i += char_len;
            out.push((Kind::Punctuation, &code[start..i]));
            continue;
        }
        if c == '}' {
            depth = depth.saturating_sub(1);
            i += char_len;
            out.push((Kind::Punctuation, &code[start..i]));
            continue;
        }
        if c == '@' {
            let word = word_end(bytes, i + 1);
            out.push((Kind::Keyword, &code[start..word.max(i + 1)]));
            i = word.max(i + 1);
            continue;
        }
        if c == '#' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_hexdigit()) {
            i = word_end(bytes, i + 1);
            out.push((Kind::Number, &code[start..i]));
            continue;
        }
        if c == '$' || (c == '-' && bytes.get(i + 1) == Some(&b'-')) {
            let word = word_end(bytes, i + 1);
            out.push((Kind::Variable, &code[start..word.max(i + 1)]));
            i = word.max(i + 1);
            continue;
        }
        if c.is_ascii_digit() {
            i = number_end(bytes, i);
            out.push((Kind::Number, &code[start..i]));
            continue;
        }
        if is_word_start(c) {
            let word = word_end(bytes, i);
            let text = &code[i..word];
            // A word followed by a `:` inside a block is a property.
            let is_property = depth > 0 && bytes.get(word).is_some_and(|b| *b == b':') && !text.starts_with("--");
            out.push((if is_property { Kind::Property } else { Kind::Plain }, text));
            i = word;
            continue;
        }
        i += char_len;
        out.push((Kind::Punctuation, &code[start..i]));
    }
}

// ------------------------------------------------------------------ helpers

/// The end of a word (letters, digits, `_`, `$`, `-` and `.`).
fn word_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'-' || b == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// The end of a number: its digits, its dot and its unit.
fn number_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut dot = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            i += 1;
        } else if b == b'.' && !dot && bytes.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
            dot = true;
            i += 1;
        } else if b == b'%' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// The end of a run of spaces.
fn whitespace_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    i
}

/// The end of a line (the `\n` is left out).
fn line_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// The end of a block comment.
fn block_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from + 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

/// The end of a string, the quotes included.
fn string_end(bytes: &[u8], from: usize) -> usize {
    let quote = bytes[from];
    let mut i = from + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        if bytes[i] == b'\n' && quote != b'`' {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// The end of a tag, the `>` included.
fn tag_end(bytes: &[u8], from: usize) -> usize {
    let mut i = from + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'>' => return i + 1,
            b'"' | b'\'' => {
                i = string_end(bytes, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// The end of a `{…}` expression, the `}` included.
fn brace_end(bytes: &[u8], from: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            b'"' | b'\'' => {
                i = string_end(bytes, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Whether a character starts a name.
fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}
