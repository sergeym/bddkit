use crate::world::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyKind {
    Json,
    Xml,
    Html,
    Plain,
}

/// Substring order matters: `application/xhtml+xml` contains both `html` and
/// `xml` — it must land in `Html`, so `html` is checked before `xml`.
pub(crate) fn classify(content_type: Option<&str>) -> BodyKind {
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if ct.contains("json") {
        BodyKind::Json
    } else if ct.contains("html") {
        BodyKind::Html
    } else if ct.contains("xml") {
        BodyKind::Xml
    } else {
        BodyKind::Plain
    }
}

pub(crate) fn content_type(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
}

fn headers_table(headers: &[(String, String)]) -> String {
    let width = headers.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    headers
        .iter()
        .map(|(k, v)| format!("{k:width$} : {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_headers(w: &World) -> Result<(), String> {
    let ex = super::assert::last(w)?;
    eprintln!("=== Response headers ===\n{}", headers_table(&ex.resp_headers));
    Ok(())
}

/// Naive tag-by-tag line breaking for XML/HTML with nesting-based indentation.
/// It does not really parse the syntax (a text node containing `<` would
/// distort the result) — good enough for debug printing; a real
/// parse is only needed for XPath (see `xpath_select`).
fn beautify_markup(raw: &str) -> String {
    let mut out = String::new();
    let mut depth: usize = 0;
    for chunk in raw.split('<').filter(|s| !s.is_empty()) {
        let Some(gt) = chunk.find('>') else {
            out.push_str(&"  ".repeat(depth));
            out.push('<');
            out.push_str(chunk);
            out.push('\n');
            continue;
        };
        let (tag_body, text) = chunk.split_at(gt + 1);
        let tag = format!("<{tag_body}");
        let is_closing = tag.starts_with("</");
        let is_self_closing = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!");
        if is_closing && depth > 0 {
            depth -= 1;
        }
        out.push_str(&"  ".repeat(depth));
        out.push_str(&tag);
        out.push('\n');
        if !is_closing && !is_self_closing {
            depth += 1;
        }
        let text = text.trim();
        if !text.is_empty() {
            out.push_str(&"  ".repeat(depth));
            out.push_str(text);
            out.push('\n');
        }
    }
    out
}

static SYNTAX_SET: std::sync::LazyLock<syntect::parsing::SyntaxSet> =
    std::sync::LazyLock::new(syntect::parsing::SyntaxSet::load_defaults_newlines);
static THEME_SET: std::sync::LazyLock<syntect::highlighting::ThemeSet> =
    std::sync::LazyLock::new(syntect::highlighting::ThemeSet::load_defaults);

/// Syntax highlighting via ANSI codes. Only enabled when stderr is a terminal;
/// when redirected to a file/log (CI), escape codes are unwanted — they'd corrupt the output.
fn highlight(code: &str, extension: &str) -> String {
    use std::io::IsTerminal;
    highlight_inner(code, extension, std::io::stderr().is_terminal())
}

fn highlight_inner(code: &str, extension: &str, colorize: bool) -> String {
    if !colorize {
        return code.to_string();
    }
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    let mut h = syntect::easy::HighlightLines::new(syntax, theme);
    let mut out = String::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        match h.highlight_line(line, &SYNTAX_SET) {
            Ok(ranges) => out.push_str(&syntect::util::as_24_bit_terminal_escaped(&ranges[..], false)),
            Err(_) => out.push_str(line),
        }
    }
    out.push_str("\x1b[0m");
    out
}

pub fn print_body(w: &World) -> Result<(), String> {
    let ex = super::assert::last(w)?;
    match classify(content_type(&ex.resp_headers)) {
        BodyKind::Json => {
            let v = ex.json()?;
            let pretty = serde_json::to_string_pretty(&v)
                .map_err(|e| format!("failed to serialize JSON: {e}"))?;
            eprintln!("{}", highlight(&pretty, "json"));
        }
        BodyKind::Xml => eprintln!("{}", highlight(&beautify_markup(&ex.body), "xml")),
        BodyKind::Html => eprintln!("{}", highlight(&beautify_markup(&ex.body), "html")),
        BodyKind::Plain => eprintln!("{}", ex.body),
    }
    Ok(())
}

fn xpath_select(xml: &str, expr: &str) -> Result<String, String> {
    let package = sxd_document::parser::parse(xml)
        .map_err(|e| format!("failed to parse XML/HTML: {e}"))?;
    let document = package.as_document();
    let xpath = sxd_xpath::Factory::new()
        .build(expr)
        .map_err(|e| format!("invalid XPath {expr:?}: {e}"))?
        .ok_or_else(|| format!("empty XPath expression {expr:?}"))?;
    let context = sxd_xpath::Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|e| format!("failed to evaluate XPath {expr:?}: {e}"))?;
    match value {
        sxd_xpath::Value::Nodeset(nodes) => {
            let ordered = nodes.document_order();
            if ordered.is_empty() {
                return Err(format!("XPath {expr:?} found no nodes"));
            }
            Ok(ordered
                .iter()
                .map(|n| n.string_value())
                .collect::<Vec<_>>()
                .join("\n"))
        }
        other => Ok(other.string()),
    }
}

pub fn print_body_as(w: &World, path: &str) -> Result<(), String> {
    let ex = super::assert::last(w)?;
    match classify(content_type(&ex.resp_headers)) {
        BodyKind::Json => {
            let root = ex.json()?;
            let sub = crate::json::path::read(&root, path)?;
            let pretty = serde_json::to_string_pretty(sub)
                .map_err(|e| format!("failed to serialize JSON: {e}"))?;
            eprintln!("{}", highlight(&pretty, "json"));
        }
        BodyKind::Xml | BodyKind::Html => {
            eprintln!("{}", xpath_select(&ex.body, path)?);
        }
        BodyKind::Plain => {
            return Err("path selection is not supported: content-type is not JSON/XML/HTML".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_json() {
        assert_eq!(classify(Some("application/json; charset=utf-8")), BodyKind::Json);
    }

    #[test]
    fn classifies_xml() {
        assert_eq!(classify(Some("application/xml")), BodyKind::Xml);
    }

    #[test]
    fn classifies_html() {
        assert_eq!(classify(Some("text/html; charset=utf-8")), BodyKind::Html);
    }

    #[test]
    fn xhtml_xml_is_classified_as_html_not_xml() {
        assert_eq!(classify(Some("application/xhtml+xml")), BodyKind::Html);
    }

    #[test]
    fn unrecognized_content_type_is_plain() {
        assert_eq!(classify(Some("application/octet-stream")), BodyKind::Plain);
    }

    #[test]
    fn missing_content_type_is_plain() {
        assert_eq!(classify(None), BodyKind::Plain);
    }

    #[test]
    fn content_type_reads_the_header_case_insensitively() {
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        assert_eq!(content_type(&headers), Some("application/json"));
    }

    #[test]
    fn content_type_is_none_when_absent() {
        let headers = vec![("x-trace".to_string(), "1".to_string())];
        assert_eq!(content_type(&headers), None);
    }

    #[test]
    fn headers_table_aligns_names_to_the_longest_one() {
        let headers = vec![
            ("x-trace".to_string(), "abc".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ];
        let table = headers_table(&headers);
        assert_eq!(
            table,
            "x-trace      : abc\ncontent-type : application/json"
        );
    }

    #[test]
    fn headers_table_of_no_headers_is_empty() {
        assert_eq!(headers_table(&[]), "");
    }

    #[test]
    fn beautify_markup_indents_nested_elements_and_text() {
        let out = beautify_markup("<root><name>Acme</name></root>");
        assert_eq!(out, "<root>\n  <name>\n    Acme\n  </name>\n</root>\n");
    }

    #[test]
    fn beautify_markup_handles_self_closing_and_attributes() {
        let out = beautify_markup(r#"<root><br/><img src="x.png"/></root>"#);
        assert_eq!(
            out,
            "<root>\n  <br/>\n  <img src=\"x.png\"/>\n</root>\n"
        );
    }

    #[test]
    fn highlight_wraps_code_in_ansi_escapes_when_colorize_is_on() {
        let out = highlight_inner("{\"a\": 1}", "json", true);
        assert!(out.contains('\u{1b}'), "highlighted output should contain an ANSI escape: {out:?}");
        assert!(out.len() > "{\"a\": 1}".len(), "highlighting should add escape bytes");
    }

    #[test]
    fn highlight_falls_back_to_plain_text_for_an_unknown_extension() {
        // Does not panic or fail — simply no highlighting.
        let out = highlight_inner("hello", "not-a-real-extension", true);
        assert!(out.contains("hello"));
    }

    #[test]
    fn highlight_returns_code_unmodified_when_colorize_is_off() {
        let out = highlight_inner("{\"a\": 1}", "json", false);
        assert_eq!(out, "{\"a\": 1}");
    }

    fn users_xml() -> &'static str {
        r#"<users><user id="1"><email>a@b.net</email></user><user id="2"><email>c@d.net</email></user></users>"#
    }

    #[test]
    fn xpath_select_reads_element_text() {
        let got = xpath_select(users_xml(), "//user[@id='2']/email").unwrap();
        assert_eq!(got, "c@d.net");
    }

    #[test]
    fn xpath_select_reads_an_attribute() {
        let got = xpath_select(users_xml(), "//user[1]/@id").unwrap();
        assert_eq!(got, "1");
    }

    #[test]
    fn xpath_select_joins_multiple_matches_with_newlines() {
        let got = xpath_select(users_xml(), "//user/email").unwrap();
        assert_eq!(got, "a@b.net\nc@d.net");
    }

    #[test]
    fn xpath_select_with_no_matches_is_an_error() {
        assert!(xpath_select(users_xml(), "//missing").is_err());
    }

    #[test]
    fn xpath_select_on_malformed_xml_is_an_error() {
        let err = xpath_select("<not-closed>", "/x").unwrap_err();
        assert!(err.contains("failed to parse"), "{err}");
    }

    #[test]
    fn xpath_select_with_an_invalid_expression_is_an_error() {
        assert!(xpath_select(users_xml(), "///[[[").is_err());
    }
}
