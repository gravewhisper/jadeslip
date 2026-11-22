use crate::sanitize::ResourceUrls;
use cssparser::{Delimiter, ParseError, Parser, ToCss, Token, serialize_identifier};

type Failed = ParseError<()>;

const SCOPE: &str = ".ss-book";
const ROOT: &str = ".ss-body";
const MAX_NESTING: u32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Declarations {
    Style,
    FontFace,
}

fn property_allowed(name: &str, kind: Declarations) -> bool {
    match kind {
        Declarations::FontFace => matches!(
            name,
            "font-family" | "src" | "font-weight" | "font-style" | "font-stretch" | "font-display"
        ),
        Declarations::Style => matches!(
            name,
            "background"
                | "background-attachment"
                | "background-clip"
                | "background-color"
                | "background-image"
                | "background-origin"
                | "background-position"
                | "background-repeat"
                | "background-size"
                | "border"
                | "border-bottom"
                | "border-bottom-color"
                | "border-bottom-left-radius"
                | "border-bottom-right-radius"
                | "border-bottom-style"
                | "border-bottom-width"
                | "border-collapse"
                | "border-color"
                | "border-image"
                | "border-left"
                | "border-left-color"
                | "border-left-style"
                | "border-left-width"
                | "border-radius"
                | "border-right"
                | "border-right-color"
                | "border-right-style"
                | "border-right-width"
                | "border-spacing"
                | "border-style"
                | "border-top"
                | "border-top-color"
                | "border-top-left-radius"
                | "border-top-right-radius"
                | "border-top-style"
                | "border-top-width"
                | "border-width"
                | "bottom"
                | "box-shadow"
                | "box-sizing"
                | "break-after"
                | "break-before"
                | "break-inside"
                | "caption-side"
                | "clear"
                | "color"
                | "column-count"
                | "column-gap"
                | "column-rule"
                | "column-width"
                | "columns"
                | "content"
                | "counter-increment"
                | "counter-reset"
                | "direction"
                | "display"
                | "empty-cells"
                | "flex"
                | "flex-direction"
                | "flex-flow"
                | "flex-wrap"
                | "float"
                | "fill"
                | "fill-opacity"
                | "fill-rule"
                | "font"
                | "font-family"
                | "font-feature-settings"
                | "font-kerning"
                | "font-size"
                | "font-stretch"
                | "font-style"
                | "font-variant"
                | "font-variant-caps"
                | "font-variant-ligatures"
                | "font-variant-numeric"
                | "font-weight"
                | "height"
                | "hyphens"
                | "left"
                | "letter-spacing"
                | "line-height"
                | "list-style"
                | "list-style-image"
                | "list-style-position"
                | "list-style-type"
                | "margin"
                | "margin-bottom"
                | "margin-left"
                | "margin-right"
                | "margin-top"
                | "max-height"
                | "max-width"
                | "min-height"
                | "min-width"
                | "object-fit"
                | "object-position"
                | "opacity"
                | "orphans"
                | "outline"
                | "outline-color"
                | "outline-offset"
                | "outline-style"
                | "outline-width"
                | "overflow"
                | "overflow-wrap"
                | "overflow-x"
                | "overflow-y"
                | "padding"
                | "padding-bottom"
                | "padding-left"
                | "padding-right"
                | "padding-top"
                | "page-break-after"
                | "page-break-before"
                | "page-break-inside"
                | "position"
                | "quotes"
                | "right"
                | "stop-color"
                | "stop-opacity"
                | "stroke"
                | "stroke-dasharray"
                | "stroke-dashoffset"
                | "stroke-linecap"
                | "stroke-linejoin"
                | "stroke-miterlimit"
                | "stroke-opacity"
                | "stroke-width"
                | "tab-size"
                | "table-layout"
                | "text-align"
                | "text-align-last"
                | "text-anchor"
                | "text-decoration"
                | "text-decoration-color"
                | "text-decoration-line"
                | "text-decoration-style"
                | "text-indent"
                | "text-orientation"
                | "text-rendering"
                | "text-shadow"
                | "text-transform"
                | "top"
                | "unicode-bidi"
                | "vertical-align"
                | "visibility"
                | "white-space"
                | "widows"
                | "width"
                | "word-break"
                | "word-spacing"
                | "word-wrap"
                | "writing-mode"
                | "z-index"
                | "-epub-hyphens"
                | "-webkit-hyphens"
        ),
    }
}

fn value_allowed(property: &str, value: &str) -> bool {
    if property != "position" {
        return true;
    }
    let plain = value.trim_end_matches("!important").trim_end();
    plain.eq_ignore_ascii_case("static")
        || plain.eq_ignore_ascii_case("relative")
        || plain.eq_ignore_ascii_case("absolute")
}

fn function_allowed(name: &str, kind: Declarations) -> bool {
    match kind {
        Declarations::FontFace => matches!(name, "format" | "local"),
        Declarations::Style => matches!(
            name,
            "rgb"
                | "rgba"
                | "hsl"
                | "hsla"
                | "calc"
                | "min"
                | "max"
                | "clamp"
                | "linear-gradient"
                | "radial-gradient"
                | "repeating-linear-gradient"
                | "repeating-radial-gradient"
                | "counter"
                | "counters"
        ),
    }
}

fn pseudo_class_allowed(name: &str) -> bool {
    matches!(
        name,
        "active"
            | "empty"
            | "first-child"
            | "first-of-type"
            | "focus"
            | "hover"
            | "last-child"
            | "last-of-type"
            | "link"
            | "only-child"
            | "only-of-type"
    )
}

fn pseudo_element_allowed(name: &str) -> bool {
    matches!(name, "after" | "before" | "first-letter" | "first-line")
}

fn is_nth_function(name: &str) -> bool {
    matches!(
        name,
        "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" | "lang"
    )
}

fn push_token(out: &mut String, token: &Token<'_>) {
    token
        .to_css(out)
        .expect("writing to a String does not fail");
}

fn push_ident(out: &mut String, name: &str) {
    serialize_identifier(name, out).expect("writing to a String does not fail");
}

fn push_string(out: &mut String, text: &str) {
    cssparser::serialize_string(text, out).expect("writing to a String does not fail");
}

fn refuse<T>(_: &mut Parser<'_>) -> Result<T, Failed> {
    Err(ParseError::custom(()))
}

pub(crate) fn sanitize_stylesheet(css: &str, urls: &ResourceUrls<'_>) -> String {
    let mut parser = Parser::new(css);
    let mut out = String::with_capacity(css.len() / 2);
    rules(&mut parser, &mut out, urls, 0);
    out
}

pub(crate) fn sanitize_inline(style: &str, urls: &ResourceUrls<'_>) -> String {
    let mut parser = Parser::new(style);
    declarations(&mut parser, urls, Declarations::Style, true)
}

pub(crate) fn sanitize_root_style(style: &str, urls: &ResourceUrls<'_>) -> String {
    let mut parser = Parser::new(style);
    declarations(&mut parser, urls, Declarations::Style, false)
}

fn rules(p: &mut Parser<'_>, out: &mut String, urls: &ResourceUrls<'_>, depth: u32) {
    loop {
        p.skip_whitespace();
        let start = p.state();
        let token = match p.next() {
            Ok(token) => token.clone(),
            Err(_) => return,
        };
        match token {
            Token::CDO | Token::CDC | Token::Semicolon => {}
            Token::AtKeyword(name) => at_rule(p, &name.to_ascii_lowercase(), out, urls, depth),
            _ => {
                p.reset(&start);
                qualified_rule(p, out, urls);
            }
        }
    }
}

fn skip_at_rule(p: &mut Parser<'_>) {
    while let Ok(token) = p.next() {
        if matches!(token, Token::Semicolon | Token::CurlyBracketBlock) {
            return;
        }
    }
}

fn at_rule(p: &mut Parser<'_>, name: &str, out: &mut String, urls: &ResourceUrls<'_>, depth: u32) {
    match name {
        "media" if depth < MAX_NESTING => {
            let prelude = p.parse_until_before(
                Delimiter::CurlyBracketBlock | Delimiter::Semicolon,
                media_prelude,
            );
            if p.expect_curly_bracket_block().is_err() {
                return;
            }
            let inner = p
                .parse_nested_block(|p| {
                    let mut inner = String::new();
                    rules(p, &mut inner, urls, depth + 1);
                    Ok::<_, Failed>(inner)
                })
                .unwrap_or_default();
            if let (Ok(prelude), false) = (prelude, inner.is_empty()) {
                out.push_str("@media ");
                out.push_str(&prelude);
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
        }
        "font-face" => {
            if p.expect_curly_bracket_block().is_err() {
                return;
            }
            let body = p
                .parse_nested_block(|p| {
                    Ok::<_, Failed>(declarations(p, urls, Declarations::FontFace, true))
                })
                .unwrap_or_default();
            if body.split(';').any(|item| item.starts_with("src:")) {
                out.push_str("@font-face{");
                out.push_str(&body);
                out.push('}');
            }
        }
        _ => skip_at_rule(p),
    }
}

fn media_prelude<'i>(p: &mut Parser<'i>) -> Result<String, Failed> {
    let mut out = String::new();
    media_tokens(p, &mut out, 0)?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return refuse(p);
    }
    Ok(trimmed.to_owned())
}

fn follows_a_word(out: &str) -> bool {
    out.trim_end()
        .ends_with(|ch: char| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '%' | '\\'))
}

fn media_tokens<'i>(p: &mut Parser<'i>, out: &mut String, depth: u32) -> Result<(), Failed> {
    if depth >= MAX_NESTING {
        return refuse(p);
    }
    while let Ok(token) = p.next_including_whitespace_and_comments() {
        let token = token.clone();
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => out.push(' '),
            Token::ParenthesisBlock => {
                out.push('(');
                p.parse_nested_block(|p| media_tokens(p, out, depth + 1))?;
                out.push(')');
            }
            Token::Ident(_)
            | Token::Number { .. }
            | Token::Dimension { .. }
            | Token::Comma
            | Token::Colon
            | Token::Delim('/' | '<' | '>' | '=') => push_token(out, &token),
            _ => return refuse(p),
        }
    }
    Ok(())
}

fn qualified_rule(p: &mut Parser<'_>, out: &mut String, urls: &ResourceUrls<'_>) {
    let selectors = p
        .parse_until_before(Delimiter::CurlyBracketBlock, |p| {
            Ok::<_, Failed>(selector_list(p, 0))
        })
        .unwrap_or_default();
    if p.expect_curly_bracket_block().is_err() {
        return;
    }
    let body = p
        .parse_nested_block(|p| Ok::<_, Failed>(declarations(p, urls, Declarations::Style, true)))
        .unwrap_or_default();
    if selectors.is_empty() || body.is_empty() {
        return;
    }
    for (ix, selector) in selectors.iter().enumerate() {
        if ix > 0 {
            out.push(',');
        }
        out.push_str(SCOPE);
        out.push(' ');
        out.push_str(selector);
    }
    out.push('{');
    out.push_str(&body);
    out.push('}');
}

fn selector_list(p: &mut Parser<'_>, depth: u32) -> Vec<String> {
    let mut selectors = Vec::new();
    loop {
        if let Ok(selector) = p.parse_until_before(Delimiter::Comma, |p| complex_selector(p, depth))
        {
            selectors.push(selector);
        }
        if p.next().is_err() {
            return selectors;
        }
    }
}

fn strict_selector_list<'i>(p: &mut Parser<'i>, depth: u32) -> Result<String, Failed> {
    let mut joined = String::new();
    loop {
        let part = p.parse_until_before(Delimiter::Comma, |p| complex_selector(p, depth))?;
        joined.push_str(&part);
        if p.next().is_err() {
            return Ok(joined);
        }
        joined.push(',');
    }
}

fn complex_selector<'i>(p: &mut Parser<'i>, depth: u32) -> Result<String, Failed> {
    if depth >= MAX_NESTING {
        return refuse(p);
    }
    let mut out = String::new();
    let mut in_compound = false;
    let mut combinator: Option<char> = None;
    let mut compounds = 0u32;
    let mut first_is_html = false;
    while let Ok(token) = p.next_including_whitespace() {
        let token = token.clone();
        match token {
            Token::WhiteSpace(_) => {
                if in_compound {
                    in_compound = false;
                    combinator = Some(' ');
                }
            }
            Token::Delim(symbol @ ('>' | '+' | '~')) => {
                if out.is_empty() {
                    return refuse(p);
                }
                in_compound = false;
                combinator = Some(symbol);
            }
            token => {
                let lowered_type = match &token {
                    Token::Ident(name) => Some(name.to_ascii_lowercase()),
                    Token::Delim('*') => Some("*".to_owned()),
                    _ => None,
                };
                if in_compound && lowered_type.is_some() {
                    return refuse(p);
                }
                let merges_into_html = compounds == 1
                    && first_is_html
                    && lowered_type.as_deref() == Some("body")
                    && matches!(combinator, Some(' ' | '>'));
                if !in_compound {
                    if let Some(symbol) = combinator.take()
                        && !merges_into_html
                    {
                        out.push(symbol);
                    }
                    if !merges_into_html {
                        compounds += 1;
                    }
                    in_compound = true;
                }
                match lowered_type.as_deref() {
                    Some("html") => {
                        first_is_html = compounds == 1;
                        out.push_str(ROOT);
                    }
                    Some("body") if merges_into_html => {}
                    Some("body") => out.push_str(ROOT),
                    Some("*") => out.push('*'),
                    Some(name) => push_ident(&mut out, name),
                    None => simple_selector(p, token, &mut out, depth)?,
                }
            }
        }
    }
    if out.is_empty() || matches!(combinator, Some('>' | '+' | '~')) {
        return refuse(p);
    }
    Ok(out)
}

fn simple_selector<'i>(
    p: &mut Parser<'i>,
    token: Token<'i>,
    out: &mut String,
    depth: u32,
) -> Result<(), Failed> {
    match token {
        Token::IDHash(name) => {
            out.push('#');
            push_ident(out, &name);
        }
        Token::Delim('.') => {
            let Ok(Token::Ident(name)) = p.next_including_whitespace().cloned() else {
                return refuse(p);
            };
            out.push('.');
            push_ident(out, &name);
        }
        Token::SquareBracketBlock => {
            out.push('[');
            p.parse_nested_block(|p| attribute_selector(p, out))?;
            out.push(']');
        }
        Token::Colon => pseudo(p, out, depth)?,
        _ => return refuse(p),
    }
    Ok(())
}

fn attribute_selector<'i>(p: &mut Parser<'i>, out: &mut String) -> Result<(), Failed> {
    let Ok(Token::Ident(name)) = p.next().cloned() else {
        return refuse(p);
    };
    push_ident(out, &name);
    let Ok(operator) = p.next().cloned() else {
        return Ok(());
    };
    if !matches!(
        operator,
        Token::Delim('=')
            | Token::IncludeMatch
            | Token::DashMatch
            | Token::PrefixMatch
            | Token::SuffixMatch
            | Token::SubstringMatch
    ) {
        return refuse(p);
    }
    push_token(out, &operator);
    let Ok(value) = p.next().cloned() else {
        return refuse(p);
    };
    if !matches!(value, Token::Ident(_) | Token::QuotedString(_)) {
        return refuse(p);
    }
    push_token(out, &value);
    match p.next().cloned() {
        Err(_) => Ok(()),
        Ok(Token::Ident(flag))
            if flag.eq_ignore_ascii_case("i") || flag.eq_ignore_ascii_case("s") =>
        {
            out.push(' ');
            push_ident(out, &flag);
            Ok(())
        }
        Ok(_) => refuse(p),
    }
}

fn pseudo<'i>(p: &mut Parser<'i>, out: &mut String, depth: u32) -> Result<(), Failed> {
    let Ok(token) = p.next_including_whitespace().cloned() else {
        return refuse(p);
    };
    match token {
        Token::Colon => {
            let Ok(Token::Ident(name)) = p.next_including_whitespace().cloned() else {
                return refuse(p);
            };
            let name = name.to_ascii_lowercase();
            if !pseudo_element_allowed(&name) {
                return refuse(p);
            }
            out.push_str("::");
            out.push_str(&name);
        }
        Token::Ident(name) => {
            let name = name.to_ascii_lowercase();
            if !pseudo_class_allowed(&name) && !pseudo_element_allowed(&name) {
                return refuse(p);
            }
            out.push(':');
            out.push_str(&name);
        }
        Token::Function(name) => {
            let name = name.to_ascii_lowercase();
            out.push(':');
            out.push_str(&name);
            out.push('(');
            if matches!(name.as_str(), "not" | "is" | "where") {
                let inner = p.parse_nested_block(|p| strict_selector_list(p, depth + 1))?;
                out.push_str(&inner);
            } else if is_nth_function(&name) {
                p.parse_nested_block(|p| nth_tokens(p, out))?;
            } else {
                return refuse(p);
            }
            out.push(')');
        }
        _ => return refuse(p),
    }
    Ok(())
}

fn nth_tokens<'i>(p: &mut Parser<'i>, out: &mut String) -> Result<(), Failed> {
    while let Ok(token) = p.next().cloned() {
        match token {
            Token::Ident(_)
            | Token::Number { .. }
            | Token::Dimension { .. }
            | Token::QuotedString(_)
            | Token::Delim('+' | '-') => {
                push_token(out, &token);
                out.push(' ');
            }
            _ => return refuse(p),
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    Ok(())
}

fn declarations(
    p: &mut Parser<'_>,
    urls: &ResourceUrls<'_>,
    kind: Declarations,
    keep_important: bool,
) -> String {
    let mut out = String::new();
    loop {
        p.skip_whitespace();
        let name = match p.next() {
            Ok(Token::Ident(name)) => Some(name.to_ascii_lowercase()),
            Ok(Token::Semicolon) => continue,
            Ok(_) => None,
            Err(_) => return out,
        };
        let value = p.parse_until_after(Delimiter::Semicolon, |p| {
            let Some(name) = name.as_deref().filter(|name| property_allowed(name, kind)) else {
                return refuse(p);
            };
            p.expect_colon()?;
            let mut value = String::new();
            value_tokens(p, &mut value, urls, kind, 0)?;
            let value = value.trim();
            if value.is_empty() || !value_allowed(name, value) {
                return refuse(p);
            }
            let value = match value.strip_suffix("!important") {
                Some(normal) if !keep_important => normal.trim_end(),
                _ => value,
            };
            Ok(value.to_owned())
        });
        if let (Some(name), Ok(value)) = (&name, value) {
            if !out.is_empty() {
                out.push(';');
            }
            out.push_str(name);
            out.push(':');
            out.push_str(&value);
        }
    }
}

fn value_tokens<'i>(
    p: &mut Parser<'i>,
    out: &mut String,
    urls: &ResourceUrls<'_>,
    kind: Declarations,
    depth: u32,
) -> Result<(), Failed> {
    if depth >= MAX_NESTING {
        return refuse(p);
    }
    while let Ok(token) = p.next_including_whitespace_and_comments() {
        let token = token.clone();
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {
                if !out.is_empty() && !out.ends_with([' ', '(']) {
                    out.push(' ');
                }
            }
            Token::Delim('!') => {
                if !p.expect_ident()?.eq_ignore_ascii_case("important") {
                    return refuse(p);
                }
                let kept = out.trim_end().len();
                out.truncate(kept);
                out.push_str("!important");
            }
            Token::UnquotedUrl(url) => push_url(p, out, urls, &url)?,
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let url = p.parse_nested_block(|p| {
                    let url = p.expect_string()?.to_string();
                    p.expect_exhausted()?;
                    Ok::<_, Failed>(url)
                })?;
                push_url(p, out, urls, &url)?;
            }
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                if !function_allowed(&name, kind) {
                    return refuse(p);
                }
                out.push_str(&name);
                out.push('(');
                p.parse_nested_block(|p| value_tokens(p, out, urls, kind, depth + 1))?;
                out.push(')');
            }
            Token::ParenthesisBlock => {
                if follows_a_word(out) {
                    return refuse(p);
                }
                out.push('(');
                p.parse_nested_block(|p| value_tokens(p, out, urls, kind, depth + 1))?;
                out.push(')');
            }
            Token::Ident(_)
            | Token::Number { .. }
            | Token::Percentage { .. }
            | Token::Dimension { .. }
            | Token::Hash(_)
            | Token::IDHash(_)
            | Token::QuotedString(_)
            | Token::Comma
            | Token::Delim('/' | '+' | '-' | '*') => push_token(out, &token),
            _ => return refuse(p),
        }
    }
    Ok(())
}

fn push_url<'i>(
    p: &mut Parser<'i>,
    out: &mut String,
    urls: &ResourceUrls<'_>,
    url: &str,
) -> Result<(), Failed> {
    let trimmed = url.trim();
    let target = if trimmed.starts_with('#') && !trimmed.contains(['(', ')', '\\']) {
        Some(trimmed.to_owned())
    } else {
        urls.resolve(trimmed)
    };
    let Some(target) = target else {
        return refuse(p);
    };
    out.push_str("url(");
    push_string(out, &target);
    out.push(')');
    Ok(())
}
