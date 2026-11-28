//! Code blocks (#2237): the languages a block is highlighted as, the numbers of its look, and
//! which stretches of its code are keywords, strings, numbers and comments. Pure data: the engine
//! makes the block ([`crate::BoardItem::CodeBlock`]) and colours its text.

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

/// What a stretch of code is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeToken {
    Keyword,
    String,
    Number,
    Comment,
}

impl CodeToken {
    /// Its colour on the panel (FigCraft's own, each at least 4.5:1 on [`CODE_PANEL`]).
    pub fn color(self) -> Color {
        match self {
            CodeToken::Keyword => Color::from_rgb8_const(0x86, 0xB0, 0xFF),
            CodeToken::String => Color::from_rgb8_const(0xA3, 0xD9, 0x8F),
            CodeToken::Number => Color::from_rgb8_const(0xF5, 0xC5, 0x6E),
            CodeToken::Comment => Color::from_rgb8_const(0x9C, 0x9C, 0x9C),
        }
    }
}

/// How a language writes keywords, comments and strings.
#[derive(Clone, Copy)]
struct Syntax {
    /// Its keywords, apart by spaces.
    keywords: &'static str,
    /// Keywords in any case (SQL).
    any_case: bool,
    /// What starts a comment to the end of the line.
    line: &'static [&'static str],
    /// Stretches from an opening to a closing mark, over lines.
    blocks: &'static [(&'static str, &'static str, CodeToken)],
    /// What opens and closes a string on one line.
    quotes: &'static [char],
    /// Markup: the name after `<` or `</` is a keyword, and quotes are strings only inside a tag.
    tags: bool,
}

const PLAIN: Syntax = Syntax { keywords: "", any_case: false, line: &[], blocks: &[], quotes: &[], tags: false };
const C_BLOCK: (&str, &str, CodeToken) = ("/*", "*/", CodeToken::Comment);
const TRIPLE: (&str, &str, CodeToken) = ("\"\"\"", "\"\"\"", CodeToken::String);
const BACKTICK: (&str, &str, CodeToken) = ("`", "`", CodeToken::String);
const BOTH_QUOTES: &[char] = &['"', '\''];
const JS: &str = "async await break case catch class const continue debugger default delete do else export extends false finally for from function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while yield";
const TS: &str = "abstract any as async await boolean break case catch class const continue debugger declare default delete do else enum export extends false finally for from function if implements import in instanceof interface keyof let namespace never new null number of private protected public readonly return static string super switch this throw true try type typeof undefined unknown var void while yield";

/// A language with `//` and `/* */` comments, and `blocks` beside them.
const fn c_like(keywords: &'static str, quotes: &'static [char], blocks: &'static [(&'static str, &'static str, CodeToken)]) -> Syntax {
    Syntax { keywords, line: &["//"], blocks, quotes, ..PLAIN }
}

impl CodeLanguage {
    fn syntax(self) -> Syntax {
        match self {
            CodeLanguage::PlainText => PLAIN,
            CodeLanguage::Bash => Syntax {
                keywords: "alias break case continue declare do done elif else esac exit export false fi for function if in local readonly return select shift source then true unset until while",
                line: &["#"],
                quotes: BOTH_QUOTES,
                ..PLAIN
            },
            CodeLanguage::Cpp => c_like(
                "auto bool break case catch char class const constexpr continue default define delete do double else enum explicit extern false float for friend goto if include inline int long mutable namespace new noexcept nullptr operator private protected public return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile while",
                BOTH_QUOTES,
                &[C_BLOCK],
            ),
            CodeLanguage::Css => Syntax { keywords: "import important keyframes media supports", blocks: &[C_BLOCK], quotes: BOTH_QUOTES, ..PLAIN },
            CodeLanguage::Go => c_like(
                "break case chan const continue default defer else fallthrough false for func go goto if import interface iota map nil package range return select struct switch true type var",
                BOTH_QUOTES,
                &[C_BLOCK, BACKTICK],
            ),
            CodeLanguage::Graphql => Syntax {
                keywords: "directive enum extend false fragment implements input interface mutation null on query scalar schema subscription true type union",
                line: &["#"],
                blocks: &[TRIPLE],
                quotes: &['"'],
                ..PLAIN
            },
            CodeLanguage::Html => Syntax { blocks: &[("<!--", "-->", CodeToken::Comment)], quotes: BOTH_QUOTES, tags: true, ..PLAIN },
            CodeLanguage::Javascript => c_like(JS, BOTH_QUOTES, &[C_BLOCK, BACKTICK]),
            CodeLanguage::Typescript => c_like(TS, BOTH_QUOTES, &[C_BLOCK, BACKTICK]),
            CodeLanguage::Json => Syntax { keywords: "false null true", quotes: &['"'], ..PLAIN },
            CodeLanguage::Kotlin => c_like(
                "as break by class companion continue data do else enum false for fun if import in init interface internal is null object open override package private protected public return sealed super suspend this throw true try typealias val var when while",
                BOTH_QUOTES,
                &[C_BLOCK, TRIPLE],
            ),
            CodeLanguage::Python => Syntax {
                keywords: "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return self try while with yield",
                line: &["#"],
                blocks: &[TRIPLE, ("'''", "'''", CodeToken::String)],
                quotes: BOTH_QUOTES,
                ..PLAIN
            },
            CodeLanguage::Ruby => Syntax {
                keywords: "alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo require rescue retry return self super then true undef unless until when while yield",
                line: &["#"],
                quotes: BOTH_QUOTES,
                ..PLAIN
            },
            // Only double quotes: a single one also starts a lifetime.
            CodeLanguage::Rust => c_like(
                "Self as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self static struct super trait true type unsafe use where while",
                &['"'],
                &[C_BLOCK],
            ),
            CodeLanguage::Sql => Syntax {
                keywords: "all alter and as asc between by case count create default delete desc distinct drop else end exists foreign from full group having in index inner insert into is join key left like limit not null offset on or order outer primary references right select set table then union update values when where",
                any_case: true,
                line: &["--"],
                blocks: &[C_BLOCK],
                quotes: &['\''],
                ..PLAIN
            },
            CodeLanguage::Swift => c_like(
                "Self as associatedtype break case catch class continue default defer do else enum extension fallthrough false for func guard if import in init inout internal is let nil operator private protocol public repeat rethrows return self static struct subscript super switch throw throws true try typealias var where while",
                &['"'],
                &[C_BLOCK, TRIPLE],
            ),
        }
    }
}

/// Where a Rust character literal opening at `i` ends (`'a'`, `'\n'`, `'\u{1F600}'`), or `None`
/// for a lifetime (`'a`), whose quote opens nothing.
fn rust_char(chars: &[char], i: usize) -> Option<usize> {
    let close = if chars.get(i + 1) == Some(&'\\') { (i + 3..(i + 12).min(chars.len())).find(|&j| chars.get(j) == Some(&'\''))? } else { i + 2 };
    (chars.get(close) == Some(&'\'') && !chars.get(i + 1..close)?.contains(&'\n')).then_some(close + 1)
}

/// The stretches of `text` that `language` colours, as char ranges (end exclusive), in order and
/// apart: keywords, strings (a quoted one ends with its line), numbers and comments. Plain text
/// colours nothing.
pub fn code_tokens(language: CodeLanguage, text: &str) -> Vec<(usize, usize, CodeToken)> {
    if language == CodeLanguage::PlainText {
        return Vec::new();
    }
    let syn = language.syntax();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let at = |i: usize, mark: &str| mark.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c));
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    let (mut i, mut in_tag) = (0, false);
    while let Some(&c) = chars.get(i) {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = if let Some(&(open, close, token)) = syn.blocks.iter().find(|(open, ..)| at(i, open)) {
            let from = i + open.chars().count();
            // A string's closing mark escaped by a backslash doesn't close it.
            let escaped = |j: usize| token == CodeToken::String && j.checked_sub(1).and_then(|p| chars.get(p)) == Some(&'\\');
            let end = (from..n).find(|&j| at(j, close) && !escaped(j)).map_or(n, |j| j + close.chars().count());
            out.push((i, end, token));
            end
        } else if syn.line.iter().any(|m| at(i, m))
            && (c != '#' || language != CodeLanguage::Bash || prev.is_none_or(|p| p.is_whitespace() || ";|&()".contains(p)))
        {
            // In Bash a `#` starts a comment only at the start of a word, after a space or an
            // operator (`$#` and `a#b` are not).
            let end = (i..n).find(|&j| chars.get(j) == Some(&'\n')).unwrap_or(n);
            out.push((i, end, CodeToken::Comment));
            end
        } else if let Some(end) = (language == CodeLanguage::Rust && c == '\'').then(|| rust_char(&chars, i)).flatten() {
            out.push((i, end, CodeToken::String));
            end
        } else if syn.quotes.contains(&c) && (!syn.tags || in_tag) {
            let mut j = i + 1;
            while let Some(&d) = chars.get(j).filter(|d| **d != c && **d != '\n') {
                // An escape skips the next character, but never the end of the line.
                j += if d == '\\' && chars.get(j + 1).is_some_and(|e| *e != '\n') { 2 } else { 1 };
            }
            let end = if chars.get(j) == Some(&c) { j + 1 } else { j.min(n) };
            out.push((i, end, CodeToken::String));
            end
        } else if c.is_ascii_digit() && !prev.is_some_and(word) && !syn.tags {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|d| word(*d) || (*d == '.' && chars.get(j + 1).is_some_and(char::is_ascii_digit))) {
                j += 1;
            }
            out.push((i, j, CodeToken::Number));
            j
        } else if word(c) && !c.is_ascii_digit() {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|d| word(*d)) {
                j += 1;
            }
            let w: String = chars.get(i..j).unwrap_or(&[]).iter().collect();
            let tag_name = syn.tags && (prev == Some('<') || (prev == Some('/') && i.checked_sub(2).and_then(|p| chars.get(p)) == Some(&'<')));
            let keyword = syn.keywords.split_whitespace().any(|k| if syn.any_case { k.eq_ignore_ascii_case(&w) } else { k == w });
            if tag_name || keyword {
                out.push((i, j, CodeToken::Keyword));
            }
            j
        } else {
            if syn.tags && (c == '<' || c == '>') {
                in_tag = c == '<';
            }
            i + 1
        };
        i = next;
    }
    out
}
