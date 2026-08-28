//! The step vocabulary as a human reads it: `bddkit steps`.

use std::collections::BTreeMap;

/// Only non-English locales are files. English is the `description` in the step
/// table itself, fifteen characters from the pattern it describes, where the
/// compiler makes a new step declare one.
const LOCALES: &[(&str, &str)] = &[
    ("ru", include_str!("../../locales/steps.ru.yaml")),
    ("lv", include_str!("../../locales/steps.lv.yaml")),
];

/// The overlay for one language, empty for English and for anything unknown.
pub fn translations(lang: &str) -> BTreeMap<String, String> {
    LOCALES
        .iter()
        .find(|(code, _)| *code == lang)
        .map(|(code, text)| {
            serde_yaml_ng::from_str(text)
                .unwrap_or_else(|error| panic!("embedded locale {code} is malformed: {error}"))
        })
        .unwrap_or_default()
}

/// The translated description, else the English one. The fallback is per step
/// and not per file: a translation always lags the steps added since it was
/// written, and a blank line helps nobody.
pub fn describe<'a>(id: &str, english: &'a str, overlay: &'a BTreeMap<String, String>) -> &'a str {
    overlay.get(id).map(String::as_str).unwrap_or(english)
}

/// `--lang`, else `BDDKIT_LANG`, else English.
pub fn language(explicit: Option<&str>) -> String {
    explicit
        .map(str::to_string)
        .or_else(|| std::env::var("BDDKIT_LANG").ok())
        .unwrap_or_else(|| "en".to_string())
}

/// One listed step. `pattern` is the raw regex, carried in the machine form
/// only: the text form is read by agents, where every character costs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StepRow {
    pub group: String,
    pub template: String,
    pub pattern: String,
    pub kind: &'static str,
    pub description: Option<String>,
}

/// Every builtin. Plugin rows are appended by the caller — they are the only
/// half that needs a config to exist at all.
pub fn builtin_rows(overlay: &BTreeMap<String, String>) -> Vec<StepRow> {
    crate::steps::BUILTIN_STEPS
        .iter()
        .map(|def| StepRow {
            group: def.group.to_string(),
            template: template(def.pattern),
            pattern: def.pattern.to_string(),
            kind: match def.kind {
                crate::steps::StepKind::Action => "action",
                crate::steps::StepKind::Assertion(_) => "assertion",
            },
            description: Some(
                describe(&format!("{:?}", def.id), def.description, overlay).to_string(),
            ),
        })
        .collect()
}

/// Case-insensitive substring over the template, and over the description too
/// when it is on screen — filtering on text the caller cannot see is worse
/// than not filtering on it at all.
pub fn matches_filter(row: &StepRow, filter: &str, verbose: bool) -> bool {
    let needle = filter.to_lowercase();
    row.template.to_lowercase().contains(&needle)
        || (verbose
            && row
                .description
                .as_deref()
                .is_some_and(|text| text.to_lowercase().contains(&needle)))
}

/// Grouped, one step per line, the group name printed once instead of a prefix
/// repeated on every line.
pub fn render(rows: &[StepRow], verbose: bool) -> String {
    let mut by_group: BTreeMap<&str, Vec<&StepRow>> = BTreeMap::new();
    for row in rows {
        by_group.entry(row.group.as_str()).or_default().push(row);
    }
    let mut out = String::new();
    for (group, rows) in by_group {
        out.push_str(group);
        out.push_str(":\n");
        for row in rows {
            out.push_str("  ");
            out.push_str(&row.template);
            out.push('\n');
            if verbose && let Some(description) = &row.description {
                out.push_str("    ");
                out.push_str(description);
                out.push('\n');
            }
        }
    }
    out
}

/// A regex pattern rendered as a step template: anchors dropped, every capture
/// group replaced by `<name>`, every other character left exactly as it is.
///
/// An unnamed group falls back to `<value1>`, `<value2>`, … by position, so a
/// plugin pattern — or a builtin nobody has annotated — degrades instead of
/// breaking the listing.
///
/// ponytail: no nesting and no escaped parentheses, because no pattern in the
/// step table or in any plugin manifest has either. A nested group would render
/// as its outer span; give this a depth counter if one ever appears.
pub fn template(pattern: &str) -> String {
    let body = pattern.trim_start_matches('^').trim_end_matches('$');
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    let mut index = 0usize;
    while let Some(start) = rest.find('(') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(')') else {
            // Unbalanced: not a pattern this function can interpret, so hand
            // the remainder back verbatim rather than inventing a parameter.
            out.push_str(&rest[start..]);
            return out;
        };
        index += 1;
        let group = &rest[start + 1..start + end];
        out.push('<');
        match group_name(group) {
            Some(name) => out.push_str(name),
            None => out.push_str(&format!("value{index}")),
        }
        out.push('>');
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn group_name(group: &str) -> Option<&str> {
    let inner = group
        .strip_prefix("?P<")
        .or_else(|| group.strip_prefix("?<"))?;
    inner.split_once('>').map(|(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_group_becomes_its_name() {
        assert_eq!(
            template(r#"^the "(?P<name>[^"]*)" request header is "(?P<value>[^"]*)"$"#),
            r#"the "<name>" request header is "<value>""#
        );
    }

    #[test]
    fn an_unnamed_group_falls_back_to_its_position() {
        // A plugin's pattern, or a builtin nobody has annotated yet: the
        // listing must degrade, never break.
        assert_eq!(
            template(r#"^I upload "([^"]*)" to "([^"]*)"$"#),
            r#"I upload "<value1>" to "<value2>""#
        );
    }

    #[test]
    fn a_digit_group_and_an_alternation_read_like_the_step() {
        assert_eq!(
            template(r#"^the response code is (?P<code>\d+)$"#),
            "the response code is <code>"
        );
        assert_eq!(
            template(r#"^I request "(?P<path>[^"]*)" using HTTP (?P<method>GET|POST)$"#),
            r#"I request "<path>" using HTTP <method>"#
        );
    }

    #[test]
    fn a_pattern_without_groups_keeps_its_trailing_colon() {
        // The colon is what tells a reader the step takes a docstring or table.
        assert_eq!(template("^the request body is:$"), "the request body is:");
    }

    #[test]
    fn every_embedded_locale_parses_and_names_real_steps() {
        // The files are compiled in, so a malformed one is a build-time bug of
        // ours — and a key naming a step that no longer exists is dead weight
        // that will never be printed. Completeness is deliberately NOT checked:
        // falling back to English per step is what lets a translation lag.
        let ids: Vec<String> = crate::steps::BUILTIN_STEPS
            .iter()
            .map(|def| format!("{:?}", def.id))
            .collect();
        for (code, _) in LOCALES {
            let map = translations(code);
            assert!(!map.is_empty(), "locale {code} is empty");
            for key in map.keys() {
                assert!(
                    ids.contains(key),
                    "locale {code} names an unknown step {key:?}"
                );
            }
        }
    }

    #[test]
    fn a_translation_wins_and_a_missing_key_falls_back_to_english() {
        let english = "sets a variable for the current scenario";
        let ru = translations("ru");
        assert_ne!(describe("SetVariable", english, &ru), english);
        assert_eq!(describe("NoSuchStep", english, &ru), english);
        // English is not a file: asking for it means asking for the table.
        assert_eq!(describe("SetVariable", english, &translations("en")), english);
    }

    #[test]
    fn every_builtin_pattern_renders_without_leaking_regex() {
        for def in crate::steps::BUILTIN_STEPS {
            let rendered = template(def.pattern);
            for leak in ["[^", "\\d", "?P<", "^", "$"] {
                assert!(
                    !rendered.contains(leak),
                    "{:?} renders as {rendered:?}, which still contains {leak:?}",
                    def.id
                );
            }
        }
    }
}
