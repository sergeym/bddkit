//! `bddkit resource fields`: what a `resources.<kind>.<name>` body takes.
//!
//! The host's three kinds come from the hand-written tables in `config`; a
//! plugin group's come from the plugin's own manifest, which the host prints
//! and interprets in no other way. Both halves are one listing on purpose —
//! the reader's question is "what does this entry take", and which side of the
//! FFI boundary answers it is not part of the question.

use crate::plugin::Plugins;
use serde::Serialize;

/// One resource kind: a host kind or a plugin group.
#[derive(Debug, Serialize)]
pub struct Kind {
    pub kind: String,
    /// Who described it — the host itself, or the plugin serving the group.
    pub source: &'static str,
    /// `None` for a plugin whose manifest declares no `fields`. Not an error:
    /// the key is optional, so a plugin published before it existed still
    /// loads, and the host has nothing of its own to say about the group.
    pub fields: Option<Vec<Field>>,
}

#[derive(Debug, Serialize)]
pub struct Field {
    pub name: String,
    pub required: bool,
    pub description: Option<String>,
    pub example: Option<String>,
}

/// The three kinds the host serves itself, always available: they need no
/// config, and describing them cannot fail.
pub fn host_kinds() -> Vec<Kind> {
    [
        ("api", crate::config::API_FIELDS),
        ("db", crate::config::DB_FIELDS),
        ("srp", crate::config::SRP_FIELDS),
    ]
    .into_iter()
    .map(|(kind, table)| Kind {
        kind: kind.to_string(),
        source: "host",
        fields: Some(
            table
                .iter()
                .map(|(name, required, description)| Field {
                    name: name.to_string(),
                    required: *required,
                    description: Some(description.to_string()),
                    example: None,
                })
                .collect(),
        ),
    })
    .collect()
}

/// Every group the loaded plugins serve, described or not.
pub fn plugin_kinds(plugins: &Plugins) -> Vec<Kind> {
    plugins
        .group_names()
        .into_iter()
        .map(|group| Kind {
            fields: plugins.fields_for(&group).map(|fields| {
                fields
                    .iter()
                    .map(|field| Field {
                        name: field.name.clone(),
                        required: field.required,
                        description: field.description.clone(),
                        example: field.example.clone(),
                    })
                    .collect()
            }),
            kind: group,
            source: "plugin",
        })
        .collect()
}

/// One kind per block, one field per line, `required` in its own column. A
/// plugin that describes nothing says so in place of its field list, because
/// an empty block reads as "this group takes no keys" — which is a different
/// and usually wrong answer.
pub fn render(kinds: &[Kind]) -> String {
    let mut out = String::new();
    for kind in kinds {
        out.push_str(&kind.kind);
        out.push_str(":\n");
        let Some(fields) = &kind.fields else {
            out.push_str("  the plugin serving this group does not describe its fields\n");
            continue;
        };
        for field in fields {
            let mark = if field.required { "required" } else { "" };
            let mut detail = field.description.clone().unwrap_or_default();
            if let Some(example) = &field.example {
                detail.push_str(&format!(" (e.g. {example})"));
            }
            out.push_str(&format!(
                "  {:<18}{mark:<10}{}\n",
                field.name,
                detail.trim_start()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_host_kind_is_described_and_marks_its_required_keys() {
        let kinds = host_kinds();
        let out = render(&kinds);
        for kind in ["api:", "db:", "srp:"] {
            assert!(out.contains(kind), "{out}");
        }
        assert!(out.contains("base_url"), "{out}");
        assert!(out.contains("required"), "{out}");
        // Every host kind's config is fully described, or the table is stale.
        assert!(kinds.iter().all(|kind| kind.fields.is_some()));
    }

    #[test]
    fn an_undescribed_group_says_so_instead_of_showing_an_empty_block() {
        let out = render(&[Kind {
            kind: "widget".into(),
            source: "plugin",
            fields: None,
        }]);
        assert!(out.contains("widget:"), "{out}");
        assert!(out.contains("does not describe"), "{out}");
    }

    #[test]
    fn an_example_is_shown_beside_the_description() {
        let out = render(&[Kind {
            kind: "s3".into(),
            source: "plugin",
            fields: Some(vec![Field {
                name: "bucket".into(),
                required: true,
                description: Some("bucket the steps read and write".into()),
                example: Some("acceptance".into()),
            }]),
        }]);
        assert!(out.contains("(e.g. acceptance)"), "{out}");
    }
}
