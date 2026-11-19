//! JSON values: read (agents' commands, KiCad's `.kicad_pro`) and written (what agents get back).

pub fn list(it: impl Iterator<Item = String>) -> String {
    format!("[{}]", it.collect::<Vec<_>>().join(","))
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Val>),
    Obj(Vec<(String, Val)>),
}

impl Val {
    pub fn parse(s: &str) -> Option<Self> {
        let mut p = Json { b: s.as_bytes(), i: 0, depth: 0 };
        let v = p.val()?;
        p.ws();
        (p.i == p.b.len()).then_some(v)
    }
    pub fn get(&self, k: &str) -> Option<&Val> {
        let Val::Obj(m) = self else { return None };
        m.iter().find(|(key, _)| key == k).map(|(_, v)| v)
    }
    /// The value down a path of keys: `at(&["board", "design_settings"])`.
    pub fn at(&self, path: &[&str]) -> Option<&Val> {
        path.iter().try_fold(self, |v, k| v.get(k))
    }
    pub fn str(&self, k: &str) -> Option<&str> {
        let Val::Str(s) = self.get(k)? else { return None };
        Some(s)
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        let Val::Num(n) = self.get(k)? else { return None };
        Some(*n)
    }
    pub fn bool(&self, k: &str) -> Option<bool> {
        let Val::Bool(b) = self.get(k)? else { return None };
        Some(*b)
    }
    pub fn arr(&self, k: &str) -> Option<&[Val]> {
        let Val::Arr(a) = self.get(k)? else { return None };
        Some(a)
    }
    /// The value at `path`, made (null) where missing: an object's key, or in an array the
    /// object of that `name` (KiCad's net classes).
    pub fn slot(&mut self, path: &[&str]) -> &mut Val {
        let Some((key, rest)) = path.split_first() else { return self };
        if !matches!(self, Val::Obj(_) | Val::Arr(_)) {
            *self = Val::Obj(vec![]);
        }
        let next = match self {
            Val::Obj(fields) => {
                let i = fields.iter().position(|(k, _)| k == key).unwrap_or_else(|| {
                    fields.push((key.to_string(), Val::Null));
                    fields.len() - 1
                });
                &mut fields[i].1
            }
            Val::Arr(items) => {
                let i = items.iter().position(|it| it.str("name") == Some(key)).unwrap_or_else(|| {
                    items.push(Val::Obj(vec![("name".into(), Val::Str(key.to_string()))]));
                    items.len() - 1
                });
                &mut items[i]
            }
            _ => unreachable!("made an object above"),
        };
        next.slot(rest)
    }

    /// As KiCad writes its JSON files (`.kicad_pro`): two spaces a level, an item a line.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out + "\n"
    }

    fn write(&self, out: &mut String, depth: usize) {
        let (open, close, n) = match self {
            Val::Arr(a) if !a.is_empty() => ('[', ']', a.len()),
            Val::Obj(m) if !m.is_empty() => ('{', '}', m.len()),
            v => return out.push_str(&v.to_json()),
        };
        out.push(open);
        for k in 0..n {
            out.push('\n');
            out.push_str(&"  ".repeat(depth + 1));
            let v = match self {
                Val::Arr(a) => &a[k],
                Val::Obj(m) => {
                    out.push_str(&format!("{}: ", json_str(&m[k].0)));
                    &m[k].1
                }
                _ => unreachable!("an array or object"),
            };
            v.write(out, depth + 1);
            if k + 1 < n {
                out.push(',');
            }
        }
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
        out.push(close);
    }

    pub fn to_json(&self) -> String {
        match self {
            Val::Null => "null".into(),
            Val::Bool(b) => b.to_string(),
            Val::Num(n) => format!("{n}"),
            Val::Str(s) => json_str(s),
            Val::Arr(a) => list(a.iter().map(Val::to_json)),
            Val::Obj(m) => {
                format!("{{{}}}", m.iter().map(|(k, v)| format!("{}:{}", json_str(k), v.to_json())).collect::<Vec<_>>().join(","))
            }
        }
    }
}

struct Json<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl<'a> Json<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn eat(&mut self, ch: u8) -> bool {
        self.ws();
        (self.b.get(self.i) == Some(&ch)).then(|| self.i += 1).is_some()
    }
    /// A value; `None` for what isn't JSON, or lists inside lists deeper than a file of a
    /// project's goes (each level is a call: a megabyte of `[` would end the program).
    fn val(&mut self) -> Option<Val> {
        self.depth += 1;
        let val = if self.depth > 128 { None } else { self.value() };
        self.depth -= 1;
        val
    }
    fn value(&mut self) -> Option<Val> {
        self.ws();
        match *self.b.get(self.i)? {
            b'"' => self.string().map(Val::Str),
            b'{' => {
                self.i += 1;
                let mut m = vec![];
                if !self.eat(b'}') {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        m.push((k, self.val()?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Some(Val::Obj(m))
            }
            b'[' => {
                self.i += 1;
                let mut a = vec![];
                if !self.eat(b']') {
                    loop {
                        a.push(self.val()?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Some(Val::Arr(a))
            }
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Val::Bool(true))
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Val::Bool(false))
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(Val::Null)
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok().map(Val::Num)
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        if *self.b.get(self.i)? != b'"' {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            // The plain characters up to the next quote or escape, at once.
            let run = self.b[self.i..].iter().position(|&c| c == b'"' || c == b'\\')?;
            out += std::str::from_utf8(&self.b[self.i..self.i + run]).ok()?;
            self.i += run + 1;
            if self.b[self.i - 1] == b'"' {
                return Some(out);
            }
            let escaped = *self.b.get(self.i)?;
            self.i += 1;
            out.push(match escaped {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'b' => '\x08',
                b'f' => '\x0c',
                b'u' => {
                    let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
                    self.i += 4;
                    char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
                }
                _ => return None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_strings_in_one_go() {
        // Escapes as JSON writes them; a long string costs what its length does (a `.kicad_pro`
        // of 50 KB took 10 ms when each character checked all the text after it).
        let v = Val::parse(r#"{"k": "a\"b\\c\/d\n\u00b5é"}"#).unwrap();
        assert_eq!(v.str("k"), Some("a\"b\\c/d\nµé"));
        let long = format!(r#"["{}"]"#, "é".repeat(200_000));
        let t = std::time::Instant::now();
        let v = Val::parse(&long).unwrap();
        assert!(t.elapsed().as_millis() < 100, "{:?}", t.elapsed());
        assert_eq!(v.to_json().len(), long.len());
        // Lists inside lists: as deep as a project's files go, no deeper (each is a call).
        assert!(Val::parse(&format!("{}1{}", "[".repeat(100), "]".repeat(100))).is_some());
        assert!(Val::parse(&"[".repeat(1_000_000)).is_none());
    }

    #[test]
    fn writes_json_as_kicad_does() {
        let pro = "{\n  \"board\": {\n    \"layers\": [],\n    \"rules\": {\n      \"min_clearance\": 0.1,\n      \"on\": true\n    }\n  },\n  \"classes\": [\n    {\n      \"name\": \"Default\",\n      \"pcb\": null\n    },\n    {}\n  ]\n}\n";
        assert_eq!(Val::parse(pro).unwrap().pretty(), pro);
    }
}
