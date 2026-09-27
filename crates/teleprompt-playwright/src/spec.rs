//! A Playwright test file, as `include=checkout.spec.ts#pays by card`
//! reads it: one test's body, which is the steps a scene runs.
//!
//! Not a JavaScript parser. It finds `test('title', async ({ page }) => {`
//! and the brace that closes it, stepping over strings, template literals
//! and comments; that is enough for the tests people write.

/// Whether `source` is a test file rather than a bare script.
pub fn is_spec(source: &str) -> bool {
    source.contains("@playwright/test") && !tests(source).is_empty()
}

/// The body of the test titled `title`, its indentation removed.
pub fn test_body(source: &str, title: &str) -> Result<String, String> {
    let found = tests(source);
    let Some(t) = found.iter().find(|t| t.title == title) else {
        let titles: Vec<String> = found.iter().map(|t| format!("`{}`", t.title)).collect();
        return Err(if titles.is_empty() {
            format!("`#{title}` names no test: this file has none")
        } else {
            format!(
                "`#{title}` names no test here; its tests are {}",
                titles.join(", ")
            )
        });
    };
    Ok(dedent(&source[t.body.clone()]))
}

struct Test {
    title: String,
    body: std::ops::Range<usize>,
}

/// Every `test(…)` call with a literal title and a function body.
fn tests(source: &str) -> Vec<Test> {
    let code = code_mask(source);
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = source[at..].find("test") {
        let start = at + found;
        at = start + 4;
        let boundary = start == 0 || !is_ident(bytes[start - 1]);
        if !code[start] || !boundary {
            continue;
        }
        // `test(`, `test.only(`, `test.skip(`: anything but `test.describe(`.
        let mut i = start + 4;
        if bytes.get(i) == Some(&b'.') {
            let name_end = (i + 1..bytes.len())
                .find(|&j| !is_ident(bytes[j]))
                .unwrap_or(bytes.len());
            if &source[i + 1..name_end] == "describe" {
                continue;
            }
            i = name_end;
        }
        i = skip_space(bytes, i);
        if bytes.get(i) != Some(&b'(') {
            continue;
        }
        i = skip_space(bytes, i + 1);
        let Some((title, after)) = literal(source, i) else {
            continue;
        };
        // The function's opening brace, then the one that closes it.
        let Some(open) = (after..bytes.len()).find(|&j| code[j] && bytes[j] == b'{') else {
            continue;
        };
        let Some(open) = arrow_body(source, &code, open) else {
            continue;
        };
        let Some(close) = closing(bytes, &code, open) else {
            continue;
        };
        out.push(Test {
            title,
            body: open + 1..close,
        });
        at = close;
    }
    out
}

/// The `{` that opens the function body: past a destructured `({ page })`.
fn arrow_body(source: &str, code: &[bool], first: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let arrow = (first..bytes.len().saturating_sub(1))
        .find(|&j| code[j] && bytes[j] == b'=' && bytes[j + 1] == b'>');
    match arrow {
        // `async ({ page }) => {`: the brace after the arrow.
        Some(a) if a > first => {
            let j = skip_space(bytes, a + 2);
            (bytes.get(j) == Some(&b'{')).then_some(j)
        }
        _ => Some(first),
    }
}

fn closing(bytes: &[u8], code: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for j in open..bytes.len() {
        if !code[j] {
            continue;
        }
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// A string literal at `i`, unescaped, and where it ends.
fn literal(source: &str, i: usize) -> Option<(String, usize)> {
    let quote = *source.as_bytes().get(i)?;
    if !matches!(quote, b'\'' | b'"' | b'`') {
        return None;
    }
    let mut out = String::new();
    let mut chars = source[i + 1..].char_indices();
    while let Some((k, c)) = chars.next() {
        match c {
            '\\' => out.extend(chars.next().map(|(_, c)| c)),
            c if c == char::from(quote) => return Some((out, i + 1 + k + 1)),
            c => out.push(c),
        }
    }
    None
}

/// Which bytes are code: not inside a string, template or comment.
fn code_mask(source: &str) -> Vec<bool> {
    let bytes = source.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        let end = match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let q = bytes[i];
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != q {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                j + 1
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => (i..bytes.len())
                .find(|&j| bytes[j] == b'\n')
                .unwrap_or(bytes.len()),
            b'/' if bytes.get(i + 1) == Some(&b'*') => source[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |k| i + 2 + k + 2),
            _ => {
                i += 1;
                continue;
            }
        };
        let end = end.min(bytes.len());
        mask[i..end].fill(false);
        i = end;
    }
    mask
}

fn skip_space(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The lines between the braces, without their shared indentation.
fn dedent(body: &str) -> String {
    let lines: Vec<&str> = body.trim_start_matches('\n').trim_end().lines().collect();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for line in lines {
        out.push_str(line.get(indent..).unwrap_or("").trim_end());
        out.push('\n');
    }
    out.trim_start_matches('\n').to_string()
}
