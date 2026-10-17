//! Small text helpers shared by the assembler: splitting at top-level commas, matching parentheses,
//! stripping comments, string literals.

/// Splits at commas that are not inside parentheses or quotes. Empty input gives no parts.
pub fn split_top(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    if text.trim().is_empty() {
        return parts;
    }
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

/// For text starting with `(`: the index of the `)` that closes it.
pub fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The line without its `;` comment (a `;` inside quotes does not start one).
pub fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' => quote = Some(c),
            // A quote after a letter or digit is an apostrophe (`AF'`), not a character literal.
            '\'' if !line[..i].ends_with(|p: char| p.is_ascii_alphanumeric()) => quote = Some(c),
            ';' => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Decodes the contents of a `"..."` literal (without the quotes).
pub fn unescape(body: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            if !c.is_ascii() {
                return Err(format!("\"{c}\" is not ASCII"));
            }
            out.push(c as u8);
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('\\') => out.push(b'\\'),
            Some('"') => out.push(b'"'),
            Some('\'') => out.push(b'\''),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(u8::from_str_radix(&hex, 16).map_err(|_| format!("bad escape \\x{hex}"))?);
            }
            Some(other) => return Err(format!("unknown escape \\{other}")),
            None => return Err("string ends in a backslash".to_string()),
        }
    }
    Ok(out)
}

/// If `text` is exactly one `"..."` literal, its decoded bytes.
pub fn as_string_literal(text: &str) -> Option<Result<Vec<u8>, String>> {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        // The closing quote must be the first unescaped one.
        let body = &t[1..t.len() - 1];
        let mut escaped = false;
        for c in body.chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return None;
            }
        }
        return Some(unescape(body));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splits_only_at_top_level_commas() {
        assert_eq!(split_top("A,B"), ["A", "B"]);
        assert_eq!(split_top("(IX+1), 'x'"), ["(IX+1)", "'x'"]);
        assert_eq!(split_top("f(1,2),3"), ["f(1,2)", "3"]);
        assert_eq!(split_top("','"), ["','"]);
        assert_eq!(split_top("\"a,b\", 2"), ["\"a,b\"", "2"]);
        assert!(split_top("  ").is_empty());
    }

    fn finds_the_closing_paren() {
        assert_eq!(matching_paren("(1+(2))*3"), Some(6));
        assert_eq!(matching_paren("(abc"), None);
        assert_eq!(matching_paren("(')')"), Some(4));
    }
    /// Merged (every check kept): `splits_only_at_top_level_commas`, `finds_the_closing_paren`.
    #[test]
    fn splitting() {
        splits_only_at_top_level_commas();
        finds_the_closing_paren();
    }

    fn comments_end_at_a_semicolon_outside_quotes() {
        assert_eq!(strip_comment("lda #1 ; set"), "lda #1 ");
        assert_eq!(strip_comment(".text \"a;b\" ; c"), ".text \"a;b\" ");
        assert_eq!(strip_comment("cmp #';'"), "cmp #';'");
        assert_eq!(strip_comment("ex af,af' ; swap"), "ex af,af' ");
    }

    fn strings_decode_escapes() {
        assert_eq!(unescape("a\\n\\x41\\\\").unwrap(), b"a\nA\\");
        assert!(unescape("\\q").is_err());
        assert!(unescape("é").is_err());
        assert_eq!(as_string_literal("\"hi\"").unwrap().unwrap(), b"hi");
        assert!(as_string_literal("\"a\" + \"b\"").is_none());
        assert!(as_string_literal("1").is_none());
    }
    /// Merged (every check kept): `comments_end_at_a_semicolon_outside_quotes`, `strings_decode_escapes`.
    #[test]
    fn comments_and_strings() {
        comments_end_at_a_semicolon_outside_quotes();
        strings_decode_escapes();
    }
}
