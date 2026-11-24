//! Attribute dictionaries inside attribute dictionaries, which skpwright's model
//! leaves out: a classification keeps each IFC attribute's value there
//! (`"IFC 2x3" > "OverallHeight" > "IfcPositiveLengthMeasure" > "IfcLengthMeasure"`
//! holds `value = 2.1`). They are read here from skpwright's listing of a
//! stream-container file's records, which shows every dictionary with its own
//! dictionaries; the 2026 container is not listed, so it gives none.

use skpwright::Value;
use std::collections::HashMap;

/// A dictionary with the dictionaries it holds.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Dict {
    pub name: String,
    pub entries: Vec<(String, Value)>,
    pub children: Vec<Dict>,
}

/// The top-level dictionaries that hold others, by the GUID (hex, as skpwright
/// gives `Definition::guid` and `PlacedInstance::guid`) of the definition or
/// placement that owns them. Empty for a file whose records are not listed.
pub fn read(bytes: &[u8]) -> HashMap<String, Vec<Dict>> {
    skpwright::StreamDocument::open(bytes)
        .and_then(|d| d.records())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

struct Frame {
    indent: usize,
    kind: String,
    dict: Dict,
    owned: Vec<Dict>,
    guid: Option<String>,
}

/// The listing is Rust's pretty debug form, four spaces a level: a record opens
/// with `body: <Kind>(` and closes with `)` at the same indent; its fields sit
/// eight deeper, a dictionary's entries' keys and values sixteen deeper.
pub fn parse(text: &str) -> HashMap<String, Vec<Dict>> {
    let mut out: HashMap<String, Vec<Dict>> = HashMap::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut key: Option<String> = None;
    let mut value_kind: Option<String> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if t.starts_with(')') && stack.last().is_some_and(|f| f.indent == indent) {
            let f = stack.pop().expect("a frame");
            match f.kind.as_str() {
                "AttrNamed" => {
                    // The nearest record that is not a container: a dictionary
                    // (this one is its child) or the owner.
                    match stack.iter_mut().rev().find(|p| p.kind != "AttrContainer") {
                        Some(p) if p.kind == "AttrNamed" => p.dict.children.push(f.dict),
                        Some(p) if !f.dict.children.is_empty() => p.owned.push(f.dict),
                        _ => {}
                    }
                }
                _ => {
                    if let (Some(guid), false) = (f.guid, f.owned.is_empty()) {
                        out.entry(guid).or_default().extend(f.owned);
                    }
                }
            }
            continue;
        }
        if let Some(kind) = t.strip_prefix("body: ").and_then(|r| r.strip_suffix('(')) {
            stack.push(Frame {
                indent,
                kind: kind.to_string(),
                dict: Dict::default(),
                owned: Vec::new(),
                guid: None,
            });
            continue;
        }
        let Some(f) = stack.last_mut() else { continue };
        let field = f.indent + 8;
        if f.kind == "AttrNamed" {
            if indent == field {
                if let Some(s) = t.strip_prefix("name: ") {
                    f.dict.name = unquote(s.trim_end_matches(','));
                }
            } else if indent == field + 8 {
                if let Some(s) = t.strip_prefix("key: ") {
                    key = Some(unquote(s.trim_end_matches(',')));
                } else if let Some(s) = t.strip_prefix("value: ") {
                    let kind = s.trim_end_matches(['(', ',']);
                    if kind == "Nil" {
                        if let Some(k) = key.take() {
                            f.dict.entries.push((k, Value::Nil));
                        }
                    } else {
                        value_kind = Some(kind.to_string());
                    }
                }
            } else if indent == field + 12
                && let (Some(kind), Some(k)) = (value_kind.take(), key.take())
            {
                let s = t.trim_end_matches(',');
                let v = match kind.as_str() {
                    "Str" => Value::Text(unquote(s)),
                    "F64" => s.parse().map_or(Value::Nil, Value::Double),
                    "F32" => s.parse().map_or(Value::Nil, Value::Float),
                    "I32" => s.parse().map_or(Value::Nil, Value::Int),
                    "Bool" => Value::Bool(s != "0"),
                    // Arrays and the rest are not attribute values.
                    _ => Value::Nil,
                };
                f.dict.entries.push((k, v));
            }
        } else if indent == field
            && let Some(list) = t.strip_prefix("guid: [")
        {
            let bytes: Vec<u8> = list
                .trim_end_matches(['[', ']', ','])
                .split(',')
                .filter_map(|b| b.trim().parse().ok())
                .collect();
            if bytes.len() == 16 {
                f.guid = Some(bytes.iter().map(|b| format!("{b:02x}")).collect());
            }
        }
    }
    out
}

/// A string as Rust's debug form prints it, unquoted and unescaped.
fn unquote(s: &str) -> String {
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let hex: String = chars
                    .by_ref()
                    .skip_while(|c| *c == '{')
                    .take_while(|c| *c != '}')
                    .collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the listing, cut to a definition whose classification holds
    /// one attribute (two levels down) beside a flat dictionary.
    const LISTING: &str = r#"Obj {
    id: 10,
    body: Def(
        Def17 {
            space: Space {
                pre: Pre {
                    attrs: New(
                        Obj {
                            id: 11,
                            body: AttrContainer(
                                AttrContainer {
                                    dictionaries: [
                                        New(
                                            Obj {
                                                id: 12,
                                                body: AttrNamed(
                                                    AttrNamed {
                                                        pre: Pre {
                                                            attrs: Null,
                                                        },
                                                        name: "Flat",
                                                        entries: [
                                                            Entry {
                                                                key: "k",
                                                                value: Str(
                                                                    "v",
                                                                ),
                                                            },
                                                        ],
                                                    },
                                                ),
                                            },
                                        ),
                                        New(
                                            Obj {
                                                id: 13,
                                                body: AttrNamed(
                                                    AttrNamed {
                                                        pre: Pre {
                                                            attrs: New(
                                                                Obj {
                                                                    id: 14,
                                                                    body: AttrContainer(
                                                                        AttrContainer {
                                                                            dictionaries: [
                                                                                New(
                                                                                    Obj {
                                                                                        id: 15,
                                                                                        body: AttrNamed(
                                                                                            AttrNamed {
                                                                                                name: "Width \"W\"",
                                                                                                entries: [
                                                                                                    Entry {
                                                                                                        key: "value",
                                                                                                        value: F64(
                                                                                                            0.9,
                                                                                                        ),
                                                                                                    },
                                                                                                    Entry {
                                                                                                        key: "is_hidden",
                                                                                                        value: Bool(
                                                                                                            0,
                                                                                                        ),
                                                                                                    },
                                                                                                ],
                                                                                            },
                                                                                        ),
                                                                                    },
                                                                                ),
                                                                            ],
                                                                        },
                                                                    ),
                                                                },
                                                            ),
                                                        },
                                                        name: "IFC 2x3",
                                                        entries: [],
                                                    },
                                                ),
                                            },
                                        ),
                                    ],
                                },
                            ),
                        },
                    ),
                },
            },
            guid: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 255],
            name: "Door",
        },
    ),
}
"#;

    #[test]
    fn nested_dictionaries_are_found_with_their_owner() {
        let got = parse(LISTING);
        assert_eq!(got.len(), 1, "{got:?}");
        let dicts = &got["0102030405060708090a0b0c0d0e0fff"];
        assert_eq!(dicts.len(), 1, "only the dictionary holding others");
        assert_eq!(dicts[0].name, "IFC 2x3");
        let child = &dicts[0].children[0];
        assert_eq!(child.name, "Width \"W\"");
        assert_eq!(
            child.entries,
            vec![
                ("value".to_string(), Value::Double(0.9)),
                ("is_hidden".to_string(), Value::Bool(false)),
            ]
        );
        assert!(parse("not a listing").is_empty());
    }
}
