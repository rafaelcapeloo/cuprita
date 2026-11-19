//! Zero-copy S-expression reading for KiCad files.

use crate::tr::tr;
use std::fmt;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok<'a> {
    Open,
    Close,
    /// Bare symbol or number.
    Atom(&'a str),
    /// Quoted string without the quotes, escapes left as written.
    Str(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub msg: &'static str,
}

impl Error {
    /// 1-based line and column of the error in `src`.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let before = &src.as_bytes()[..self.offset.min(src.len())];
        let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = before.iter().rev().take_while(|&&b| b != b'\n').count() + 1;
        (line, col)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: {}", self.offset, self.msg)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// `s` as a quoted string writes it, which `unescape` reads back.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// Text of a quoted string as written in the file: `\n` is a line break, `\x` is `x`.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        out.push(if ch == '\\' { chars.next().map_or('\\', |e| if e == 'n' { '\n' } else { e }) } else { ch });
    }
    out
}

/// Splits text into tokens. Every byte is inside a token or is whitespace between tokens.
#[derive(Clone)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    /// Next token with its byte range, or `None` at the end of input.
    pub fn next_tok(&mut self) -> Result<Option<(Tok<'a>, usize, usize)>> {
        let b = self.src.as_bytes();
        while self.pos < b.len() && matches!(b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
        let start = self.pos;
        let Some(&c) = b.get(start) else { return Ok(None) };
        let tok = match c {
            b'(' => {
                self.pos += 1;
                Tok::Open
            }
            b')' => {
                self.pos += 1;
                Tok::Close
            }
            b'"' => {
                let mut i = start + 1;
                loop {
                    match b.get(i) {
                        None => return Err(Error { offset: start, msg: tr("unterminated string") }),
                        Some(b'\\') => i += 2,
                        Some(b'"') => break,
                        Some(_) => i += 1,
                    }
                }
                self.pos = i + 1;
                // Slicing at ASCII bytes always lands on char boundaries.
                Tok::Str(&self.src[start + 1..i])
            }
            _ => {
                let mut i = start + 1;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')') {
                    i += 1;
                }
                self.pos = i;
                Tok::Atom(&self.src[start..i])
            }
        };
        Ok(Some((tok, start, self.pos)))
    }
}

/// Walks nested lists: `child()` steps into the next sub-list, `close()` skips the rest of one.
#[derive(Clone)]
pub struct Cursor<'a> {
    lex: Lexer<'a>,
    peeked: Option<(Tok<'a>, Range<usize>)>,
    last: Range<usize>,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { lex: Lexer::new(src), peeked: None, last: 0..0 }
    }

    pub fn err(&self, msg: &'static str) -> Error {
        Error { offset: self.last.start, msg }
    }

    /// Byte range of the last token read (with the quotes of a string): after `close()`, its
    /// end is the end of the list.
    pub fn span(&self) -> Range<usize> {
        self.last.clone()
    }

    fn peek(&mut self) -> Result<Tok<'a>> {
        if self.peeked.is_none() {
            let (t, start, end) = self.lex.next_tok()?.ok_or_else(|| Error { offset: self.lex.pos, msg: tr("unexpected end of file") })?;
            self.peeked = Some((t, start..end));
        }
        Ok(self.peeked.as_ref().unwrap().0)
    }

    pub fn tok(&mut self) -> Result<Tok<'a>> {
        self.peek()?;
        let (t, span) = self.peeked.take().unwrap();
        self.last = span;
        Ok(t)
    }

    /// Enters the top-level list, which must start with `head`.
    pub fn open(&mut self, head: &str) -> Result<()> {
        if self.tok()? != Tok::Open || self.tok()? != Tok::Atom(head) {
            return Err(self.err(tr("unexpected file type")));
        }
        Ok(())
    }

    /// Skips atoms up to the next sub-list and returns its head, or `None` when the current
    /// list ends. Callers always finish a list they entered with `close()`.
    pub fn child(&mut self) -> Result<Option<&'a str>> {
        loop {
            match self.peek()? {
                Tok::Open => {
                    self.tok()?;
                    return match self.tok()? {
                        Tok::Atom(h) | Tok::Str(h) => Ok(Some(h)),
                        _ => Err(self.err(tr("list without a head"))),
                    };
                }
                Tok::Close => return Ok(None),
                _ => {
                    self.tok()?;
                }
            }
        }
    }

    /// Skips whatever is left of the current list, including its `)`.
    pub fn close(&mut self) -> Result<()> {
        let mut depth = 0u32;
        loop {
            match self.tok()? {
                Tok::Open => depth += 1,
                Tok::Close if depth == 0 => return Ok(()),
                Tok::Close => depth -= 1,
                _ => {}
            }
        }
    }

    /// Next atom or string as raw text.
    pub fn atom(&mut self) -> Result<&'a str> {
        match self.tok()? {
            Tok::Atom(s) | Tok::Str(s) => Ok(s),
            _ => Err(self.err(tr("expected a value"))),
        }
    }

    /// Next value if the list has one before its next sub-list or end.
    pub fn opt_atom(&mut self) -> Result<Option<&'a str>> {
        match self.peek()? {
            Tok::Atom(_) | Tok::Str(_) => self.atom().map(Some),
            _ => Ok(None),
        }
    }

    pub fn nm(&mut self) -> Result<i32> {
        let s = self.atom()?;
        crate::units::mm_to_nm(s).ok_or_else(|| self.err(tr("bad length")))
    }

    pub fn point(&mut self) -> Result<crate::board::Pt> {
        Ok(crate::board::Pt { x: self.nm()?, y: self.nm()? })
    }

    pub fn f64(&mut self) -> Result<f64> {
        let s = self.atom()?;
        s.parse().map_err(|_| self.err(tr("bad number")))
    }

    /// Optional trailing angle as in `(at x y 90)`; anything else (e.g. `unlocked`) means 0.
    pub fn opt_angle(&mut self) -> Result<f64> {
        Ok(self.opt_atom()?.and_then(|s| s.parse().ok()).unwrap_or(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Result<Vec<Tok<'_>>> {
        let mut l = Lexer::new(src);
        let mut v = vec![];
        while let Some((t, _, _)) = l.next_tok()? {
            v.push(t);
        }
        Ok(v)
    }

    #[test]
    fn lexes_strings_with_parens_and_escapes() {
        let t = toks(r#"(net "a (b) \"c\"" 12.5)"#).unwrap();
        assert_eq!(t, [Tok::Open, Tok::Atom("net"), Tok::Str(r#"a (b) \"c\""#), Tok::Atom("12.5"), Tok::Close]);
    }

    #[test]
    fn unescape_resolves_quotes_backslashes_and_newlines() {
        assert_eq!(unescape(r#"a\"b\\c\nd"#), "a\"b\\c\nd");
    }

    #[test]
    fn unterminated_string_reports_line_and_column() {
        let src = "(a\n  (b \"oops)";
        let e = toks(src).unwrap_err();
        assert_eq!((e.msg, e.line_col(src)), ("unterminated string", (2, 6)));
    }

    #[test]
    fn cursor_skips_unknown_children() {
        let mut c = Cursor::new(r#"(root x (skip (deep 1)) (keep 2 "3"))"#);
        c.open("root").unwrap();
        assert_eq!(c.child().unwrap(), Some("skip"));
        c.close().unwrap();
        assert_eq!(c.span(), 22..23);
        assert_eq!(c.child().unwrap(), Some("keep"));
        assert_eq!((c.atom().unwrap(), c.span()), ("2", 30..31));
        assert_eq!((c.atom().unwrap(), c.span()), ("3", 32..35));
        c.close().unwrap();
        assert_eq!(c.child().unwrap(), None);
        c.close().unwrap();
    }
}
