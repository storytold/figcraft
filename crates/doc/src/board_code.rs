//! Code blocks (#2237): the languages a block is highlighted as, and the numbers of its look.
//! Pure data: the engine makes the block ([`crate::BoardItem::CodeBlock`]).

use figcraft_color::Color;
use serde::{Deserialize, Serialize};

/// The language a code block is highlighted as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeLanguage {
    Bash,
    Cpp,
    Css,
    Go,
    Graphql,
    Html,
    Javascript,
    Json,
    Kotlin,
    PlainText,
    Python,
    Ruby,
    Rust,
    Sql,
    Swift,
    #[default]
    Typescript,
}

impl CodeLanguage {
    /// In menu order: by name.
    pub const ALL: [CodeLanguage; 16] = [
        CodeLanguage::Bash,
        CodeLanguage::Cpp,
        CodeLanguage::Css,
        CodeLanguage::Go,
        CodeLanguage::Graphql,
        CodeLanguage::Html,
        CodeLanguage::Javascript,
        CodeLanguage::Json,
        CodeLanguage::Kotlin,
        CodeLanguage::PlainText,
        CodeLanguage::Python,
        CodeLanguage::Ruby,
        CodeLanguage::Rust,
        CodeLanguage::Sql,
        CodeLanguage::Swift,
        CodeLanguage::Typescript,
    ];

    /// The id used in commands and saved files ("cpp", "plainText", "typescript").
    pub fn id(self) -> &'static str {
        self.names().0
    }

    /// The name shown in the UI ("C++", "Plain text", "TypeScript").
    pub fn label(self) -> &'static str {
        self.names().1
    }

    /// The id and the name shown.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            CodeLanguage::Bash => ("bash", "Bash"),
            CodeLanguage::Cpp => ("cpp", "C++"),
            CodeLanguage::Css => ("css", "CSS"),
            CodeLanguage::Go => ("go", "Go"),
            CodeLanguage::Graphql => ("graphql", "GraphQL"),
            CodeLanguage::Html => ("html", "HTML"),
            CodeLanguage::Javascript => ("javascript", "JavaScript"),
            CodeLanguage::Json => ("json", "JSON"),
            CodeLanguage::Kotlin => ("kotlin", "Kotlin"),
            CodeLanguage::PlainText => ("plainText", "Plain text"),
            CodeLanguage::Python => ("python", "Python"),
            CodeLanguage::Ruby => ("ruby", "Ruby"),
            CodeLanguage::Rust => ("rust", "Rust"),
            CodeLanguage::Sql => ("sql", "SQL"),
            CodeLanguage::Swift => ("swift", "Swift"),
            CodeLanguage::Typescript => ("typescript", "TypeScript"),
        }
    }

    /// By id or by name, in any case, trimmed ("cpp", "C++", "plain text").
    pub fn from_id(s: &str) -> Option<CodeLanguage> {
        let s = s.trim();
        CodeLanguage::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(s) || l.label().eq_ignore_ascii_case(s))
    }
}

/// The font code is set in (always this family; the machine finds the nearest when it is absent).
pub const CODE_FONT: &str = "Liberation Mono";
pub const CODE_FONT_SIZE: f64 = 16.0;
/// A line of code: 150% of the size.
pub const CODE_LINE: f64 = 24.0;
/// Room around the code: top, right, bottom, left (the left holds the line numbers).
pub const CODE_PAD: [f64; 4] = [16.0, 16.0, 16.0, 64.0];
pub const CODE_MIN_LINES: usize = 4;
/// A block's least height: four lines.
pub const CODE_MIN_H: f64 = 128.0;
pub const CODE_WIDTH: f64 = 576.0;
pub const CODE_MIN_W: f64 = 210.0;
pub const CODE_MAX_W: f64 = 100_000.0;
/// The room between a block and the next one made beside it.
pub const CODE_GAP: f64 = 128.0;
pub const CODE_RADIUS: f64 = 10.0;
/// What Tab types in a block.
pub const CODE_TAB: &str = "  ";
/// The most characters `board.codeBlock` takes.
pub const CODE_MAX_CHARS: usize = 100_000;
/// The panel: the board's ink.
pub const CODE_PANEL: Color = Color::from_rgb8_const(0x1E, 0x1E, 0x1E);
/// Plain code: the board's gray.
pub const CODE_TEXT: Color = Color::from_rgb8_const(0xE3, 0xE1, 0xDD);

/// A block's height holding `lines` visual lines (four at least): 32 + 24 a line.
pub fn code_height(lines: usize) -> f64 {
    CODE_PAD[0] + CODE_PAD[2] + CODE_LINE * lines.clamp(CODE_MIN_LINES, 1_000_000) as f64
}
