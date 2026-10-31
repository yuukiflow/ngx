//! Minimal nginx configuration lexer + parser producing a directive tree.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<String>,
    pub line: usize,
    /// Line of the closing `}` (equal to `line` for simple directives).
    pub end_line: usize,
    pub block: Option<Vec<Directive>>,
}

impl Directive {
    pub fn arg(&self, i: usize) -> Option<&str> {
        self.args.get(i).map(String::as_str)
    }
    pub fn children(&self) -> &[Directive] {
        self.block.as_deref().unwrap_or(&[])
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Directive> + 'a {
        self.children().iter().filter(move |d| d.name == name)
    }
    pub fn first(&self, name: &str) -> Option<&Directive> {
        self.children().iter().find(|d| d.name == name)
    }
    pub fn first_arg(&self, name: &str) -> Option<&str> {
        self.first(name).and_then(|d| d.arg(0))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (line {})", self.msg, self.line)
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Semi,
    Open,
    Close,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ';' => {
                toks.push((Tok::Semi, line));
                i += 1;
            }
            '{' => {
                toks.push((Tok::Open, line));
                i += 1;
            }
            '}' => {
                toks.push((Tok::Close, line));
                i += 1;
            }
            '"' | '\'' => {
                let start = line;
                let q = c;
                let mut s = String::new();
                i += 1;
                loop {
                    let Some(&ch) = chars.get(i) else {
                        return Err(ParseError { line: start, msg: "unterminated quoted string".into() });
                    };
                    i += 1;
                    match ch {
                        '\\' if i < chars.len() => {
                            let n = chars[i];
                            i += 1;
                            if n != q {
                                s.push('\\');
                            }
                            s.push(n);
                        }
                        ch if ch == q => break,
                        '\n' => {
                            line += 1;
                            s.push('\n');
                        }
                        ch => s.push(ch),
                    }
                }
                toks.push((Tok::Word(s), start));
            }
            _ => {
                let mut s = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if ch.is_whitespace() || ch == ';' || ch == '{' || ch == '}' {
                        // `${var}` inside a bare word
                        if ch == '{' && s.ends_with('$') {
                            while i < chars.len() && chars[i] != '}' {
                                s.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() {
                                s.push('}');
                                i += 1;
                            }
                            continue;
                        }
                        break;
                    }
                    if ch == '\\' && i + 1 < chars.len() {
                        s.push(ch);
                        s.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    s.push(ch);
                    i += 1;
                }
                toks.push((Tok::Word(s), line));
            }
        }
    }
    Ok(toks)
}

pub fn parse(src: &str) -> Result<Vec<Directive>, ParseError> {
    let toks = lex(src)?;
    let mut pos = 0;
    let out = parse_block(&toks, &mut pos, false)?;
    Ok(out)
}

fn parse_block(toks: &[(Tok, usize)], pos: &mut usize, nested: bool) -> Result<Vec<Directive>, ParseError> {
    let mut out = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut start_line = 0;
    while *pos < toks.len() {
        let (tok, line) = &toks[*pos];
        *pos += 1;
        match tok {
            Tok::Word(w) => {
                if words.is_empty() {
                    start_line = *line;
                }
                words.push(w.clone());
            }
            Tok::Semi => {
                if words.is_empty() {
                    continue;
                }
                let mut it = std::mem::take(&mut words).into_iter();
                out.push(Directive {
                    name: it.next().unwrap(),
                    args: it.collect(),
                    line: start_line,
                    end_line: *line,
                    block: None,
                });
            }
            Tok::Open => {
                if words.is_empty() {
                    return Err(ParseError { line: *line, msg: "unexpected \"{\"".into() });
                }
                let children = parse_block(toks, pos, true)?;
                let end_line = toks.get(*pos - 1).map(|t| t.1).unwrap_or(*line);
                let mut it = std::mem::take(&mut words).into_iter();
                out.push(Directive {
                    name: it.next().unwrap(),
                    args: it.collect(),
                    line: start_line,
                    end_line,
                    block: Some(children),
                });
            }
            Tok::Close => {
                if !words.is_empty() {
                    return Err(ParseError { line: *line, msg: format!("directive \"{}\" is not terminated by \";\"", words[0]) });
                }
                if !nested {
                    return Err(ParseError { line: *line, msg: "unexpected \"}\"".into() });
                }
                return Ok(out);
            }
        }
    }
    if nested {
        let line = toks.last().map(|t| t.1).unwrap_or(1);
        return Err(ParseError { line, msg: "unexpected end of file, expecting \"}\"".into() });
    }
    if !words.is_empty() {
        return Err(ParseError { line: start_line, msg: format!("directive \"{}\" is not terminated by \";\"", words[0]) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_blocks_and_quotes() {
        let src = r#"
# comment
server {
    listen 443 ssl; # trailing
    server_name a.com "b.com";
    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.3-fpm.sock;
    }
    add_header X-Test 'it\'s';
    return 301 https://${host}$request_uri;
}
"#;
        let d = parse(src).unwrap();
        assert_eq!(d.len(), 1);
        let s = &d[0];
        assert_eq!(s.name, "server");
        assert_eq!(s.line, 3);
        assert_eq!(s.end_line, 11);
        assert_eq!(s.first("listen").unwrap().args, vec!["443", "ssl"]);
        assert_eq!(s.first("server_name").unwrap().args, vec!["a.com", "b.com"]);
        let loc = s.first("location").unwrap();
        assert_eq!(loc.args, vec!["~", "\\.php$"]);
        assert_eq!(loc.first_arg("fastcgi_pass"), Some("unix:/run/php/php8.3-fpm.sock"));
        assert_eq!(s.first("add_header").unwrap().args[1], "it's");
        assert_eq!(s.first("return").unwrap().args[1], "https://${host}$request_uri");
    }

    #[test]
    fn reports_errors_with_lines() {
        let e = parse("server {\n listen 80;\n").unwrap_err();
        assert!(e.msg.contains("end of file"));
        let e = parse("server {\n listen 80\n}\n").unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse("}\n").unwrap_err();
        assert!(e.msg.contains("unexpected"));
    }
}
