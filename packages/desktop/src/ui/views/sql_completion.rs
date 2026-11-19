//! Completions for the SQL editor: keywords, and the tables and columns of the
//! editor's connection.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui_kit::component::input::{CompletionProvider, Rope, RopeExt as _};
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};

use crate::db::sql::{literal_at, quote_ident, split_statements};
use crate::db::{ObjectKind, TableInfo};

const KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "AS",
    "ASC",
    "AVG",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COALESCE",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "COUNT",
    "CREATE",
    "CROSS",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FIRST",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LAST",
    "LEFT",
    "LIKE",
    "LIMIT",
    "MAX",
    "MIN",
    "NOT",
    "NULL",
    "NULLS",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "RECURSIVE",
    "REFERENCES",
    "RENAME",
    "RETURNING",
    "RIGHT",
    "ROLLBACK",
    "SELECT",
    "SET",
    "SUM",
    "TABLE",
    "THEN",
    "TO",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USER",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

/// The tables of the editor's connection; empty until they load.
#[derive(Default)]
pub struct SqlCompletion {
    pub tables: RefCell<Vec<TableInfo>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keyword,
    Table,
    Column,
}

#[derive(Debug, PartialEq, Eq)]
struct Suggestion {
    label: String,
    insert: String,
    kind: Kind,
    detail: String,
}

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_keyword(word: &str) -> bool {
    KEYWORDS
        .iter()
        .any(|keyword| keyword.eq_ignore_ascii_case(word))
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        || text.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// `name` as SQL: bare when it is a plain lowercase word, quoted otherwise.
fn ident(name: &str) -> String {
    let plain = name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !is_keyword(name);
    if plain {
        name.to_string()
    } else {
        quote_ident(name)
    }
}

/// A table to offer. Inside a quoted name only its own name can go, as it
/// is written.
fn table_suggestion(info: &TableInfo, qualified: bool, quoted: bool) -> Suggestion {
    let table = &info.table;
    let insert = match table.schema.as_deref() {
        _ if quoted => in_quotes(&table.name),
        Some(schema) if qualified && schema != "public" => {
            format!("{}.{}", ident(schema), ident(&table.name))
        }
        _ => ident(&table.name),
    };
    let qualified = qualified && !quoted;
    Suggestion {
        label: if qualified {
            table.label()
        } else {
            table.name.clone()
        },
        insert,
        kind: Kind::Table,
        detail: match info.kind {
            ObjectKind::Table => "table".to_string(),
            ObjectKind::View => "view".to_string(),
        },
    }
}

/// Every name the text gives a table: its own name, `schema.name`, and an
/// alias written after it (`users u`, `users AS u`), lowercased.
fn names<'a>(text: &str, tables: &'a [TableInfo]) -> HashMap<String, &'a TableInfo> {
    // Words, keeping `schema.name` together, with where each one starts and ends.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, is_word(c) || c == '.') {
            (None, true) => start = Some(at),
            (Some(from), false) => {
                words.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    let word = |index: usize| words.get(index).map(|&(from, to)| &text[from..to]);
    // Only spaces, or the quotes of a quoted name, between two words: no
    // comma, parenthesis or operator.
    let adjacent = |index: usize| {
        words.get(index + 1).is_some_and(|next| {
            text[words[index].1..next.0]
                .chars()
                .all(|c| c.is_whitespace() || c == '"')
        })
    };

    let mut names = HashMap::new();
    for (index, _) in words.iter().enumerate() {
        let Some(info) = word(index).and_then(|word| {
            tables.iter().find(|info| {
                info.table.name.eq_ignore_ascii_case(word)
                    || info.table.label().eq_ignore_ascii_case(word)
            })
        }) else {
            continue;
        };
        names.insert(info.table.name.to_lowercase(), info);
        let mut alias = index + 1;
        if adjacent(index) && word(alias).is_some_and(|word| word.eq_ignore_ascii_case("as")) {
            if !adjacent(alias) {
                continue;
            }
            alias += 1;
        } else if !adjacent(index) {
            continue;
        }
        if let Some(alias) = word(alias).filter(|word| !is_keyword(word) && !word.contains('.')) {
            names.insert(alias.to_lowercase(), info);
        }
    }
    names
}

/// Whether the word after `head` must name a table: it follows FROM, JOIN,
/// INTO, UPDATE or TABLE, or a comma in a FROM list.
fn wants_table(head: &str) -> bool {
    let head = head.trim_end();
    let keyword = match head.strip_suffix(',') {
        Some(list) => list
            .split(|c: char| !is_word(c))
            .rev()
            .find(|word| is_keyword(word))
            .filter(|word| word.eq_ignore_ascii_case("FROM")),
        None => Some(&head[head.trim_end_matches(is_word).len()..]),
    };
    keyword.is_some_and(|keyword| {
        ["FROM", "JOIN", "INTO", "UPDATE", "TABLE"]
            .iter()
            .any(|table| table.eq_ignore_ascii_case(keyword))
    })
}

/// `name` as it goes between the quotes of a quoted name.
fn in_quotes(name: &str) -> String {
    name.replace('"', "\"\"")
}

/// The name a qualifier `head.` ends with: a word, or a quoted name.
fn qualifier_of(head: &str) -> Option<&str> {
    let name = match head.strip_suffix('"') {
        Some(quoted) => &quoted[quoted.rfind('"')? + 1..],
        None => &head[head.trim_end_matches(is_word).len()..],
    };
    (!name.is_empty()).then_some(name)
}

/// What to offer at byte `offset` of `text`, and where the word it replaces
/// starts.
fn suggest(text: &str, offset: usize, tables: &[TableInfo]) -> (usize, Vec<Suggestion>) {
    let before = text.get(..offset).unwrap_or_default();
    // Inside a quoted name the word runs from the opening quote, spaces and
    // all; strings and comments offer nothing.
    let quote = match literal_at(text, offset) {
        Some(open) if text.as_bytes()[open] == b'"' => Some(open),
        Some(_) => return (offset, Vec::new()),
        None => None,
    };
    let quoted = quote.is_some();
    let start = quote.map_or_else(|| before.trim_end_matches(is_word).len(), |open| open + 1);
    let prefix = &before[start..];
    // What comes before the word, or before its opening quote.
    let head = &before[..quote.unwrap_or(start)];
    let qualifier = head.strip_suffix('.').and_then(qualifier_of);
    // Only the statement under the cursor gives tables their names.
    let statement = split_statements(text)
        .into_iter()
        .find(|range| range.start <= offset && offset <= range.end)
        .map_or("", |range| &text[range]);
    let names = names(statement, tables);
    let column = |info: &TableInfo, out: &mut Vec<Suggestion>| {
        for column in &info.columns {
            if starts_with_ignore_case(&column.name, prefix)
                && !out
                    .iter()
                    .any(|seen: &Suggestion| seen.label == column.name)
            {
                out.push(Suggestion {
                    label: column.name.clone(),
                    insert: if quoted {
                        in_quotes(&column.name)
                    } else {
                        ident(&column.name)
                    },
                    kind: Kind::Column,
                    // Before FROM names a table, say which table it is from.
                    detail: if names.is_empty() {
                        format!("{} · {}", info.table.name, column.type_name)
                    } else {
                        column.type_name.clone()
                    },
                });
            }
        }
    };

    let mut out = Vec::new();
    if let Some(qualifier) = qualifier {
        // `alias.`, `table.` or `schema.`
        if let Some(info) = names.get(&qualifier.to_lowercase()) {
            column(info, &mut out);
        }
        for info in tables {
            let in_schema = info
                .table
                .schema
                .as_deref()
                .is_some_and(|schema| schema.eq_ignore_ascii_case(qualifier));
            if in_schema && starts_with_ignore_case(&info.table.name, prefix) {
                out.push(table_suggestion(info, false, quoted));
            }
        }
        return (start, out);
    }
    // An empty word would open the menu after every space.
    if prefix.is_empty() {
        return (start, out);
    }
    if wants_table(head) {
        for info in tables {
            if starts_with_ignore_case(&info.table.name, prefix) {
                out.push(table_suggestion(info, true, quoted));
            }
        }
        return (start, out);
    }

    // Columns of the tables the statement names; before it names any, as
    // in `SELECT na` with FROM yet to come, those of every table.
    let mut used: Vec<&TableInfo> = if names.is_empty() {
        tables.iter().collect()
    } else {
        names.values().copied().collect()
    };
    used.sort_by_key(|info| info.table.label());
    used.dedup_by(|a, b| a.table == b.table);
    for info in used {
        column(info, &mut out);
    }
    for info in tables {
        if starts_with_ignore_case(&info.table.name, prefix) {
            out.push(table_suggestion(info, true, quoted));
        }
    }
    if quoted {
        return (start, out);
    }
    // Keywords follow the case the user types in.
    let lower = !prefix.chars().any(char::is_uppercase);
    for keyword in KEYWORDS {
        if starts_with_ignore_case(keyword, prefix) && !keyword.eq_ignore_ascii_case(prefix) {
            let keyword = if lower {
                keyword.to_lowercase()
            } else {
                keyword.to_string()
            };
            out.push(Suggestion {
                label: keyword.clone(),
                insert: keyword,
                kind: Kind::Keyword,
                detail: String::new(),
            });
        }
    }
    (start, out)
}

impl CompletionProvider for SqlCompletion {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let (start, suggestions) = suggest(&text.to_string(), offset, &self.tables.borrow());
        let range = lsp_types::Range {
            start: text.offset_to_position(start),
            end: text.offset_to_position(offset),
        };
        let items = suggestions
            .into_iter()
            .map(|suggestion| CompletionItem {
                label: suggestion.label,
                kind: Some(match suggestion.kind {
                    Kind::Keyword => CompletionItemKind::KEYWORD,
                    Kind::Table => CompletionItemKind::STRUCT,
                    Kind::Column => CompletionItemKind::FIELD,
                }),
                detail: (!suggestion.detail.is_empty()).then_some(suggestion.detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: suggestion.insert,
                })),
                ..CompletionItem::default()
            })
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text.ends_with(|c: char| c == '.' || is_word(c))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would pull in GPUI's own `test` attribute.
    use super::suggest;
    use crate::db::sql::TableRef;
    use crate::db::{ColumnInfo, ObjectKind, TableInfo};

    fn table(schema: Option<&str>, name: &str, columns: &[&str]) -> TableInfo {
        TableInfo {
            table: TableRef {
                schema: schema.map(str::to_string),
                name: name.to_string(),
            },
            kind: ObjectKind::Table,
            columns: columns
                .iter()
                .map(|name| ColumnInfo {
                    name: name.to_string(),
                    type_name: "text".to_string(),
                    nullable: true,
                    default: None,
                    primary_key: false,
                })
                .collect(),
        }
    }

    fn tables() -> Vec<TableInfo> {
        vec![
            table(Some("public"), "users", &["id", "name", "email"]),
            table(Some("public"), "orders", &["id", "user_id", "total"]),
            table(Some("audit"), "Events", &["id", "kind"]),
        ]
    }

    /// The text each suggestion inserts, with `|` marking the cursor.
    fn inserts(text: &str) -> Vec<String> {
        let offset = text.find('|').expect("a cursor");
        let text = text.replace('|', "");
        let (start, suggestions) = suggest(&text, offset, &tables());
        let word = text[start..offset].to_string();
        assert!(text[..offset].ends_with(&word));
        suggestions.into_iter().map(|s| s.insert).collect()
    }

    #[test]
    fn columns_follow_an_alias_or_a_table_name() {
        assert_eq!(inserts("SELECT u.| FROM users u"), ["id", "name", "email"]);
        assert_eq!(
            inserts("SELECT o.u| FROM users AS u JOIN orders o"),
            ["user_id"]
        );
        assert_eq!(inserts("SELECT users.e| FROM users"), ["email"]);
        assert_eq!(
            inserts("SELECT x.| FROM users x WHERE x.id = 1"),
            ["id", "name", "email"]
        );
        // A comma ends the table: `orders` is a second table, not an alias.
        assert_eq!(inserts("SELECT orders.t| FROM users, orders"), ["total"]);
        assert_eq!(inserts("SELECT nobody.| FROM users"), Vec::<String>::new());
    }

    #[test]
    fn only_the_statement_under_the_cursor_and_only_sql_code_count() {
        assert_eq!(
            inserts("SELECT * FROM users u; SELECT u.|"),
            Vec::<String>::new()
        );
        assert_eq!(inserts("SELECT u.n| FROM users u; SELECT 1"), ["name"]);
        assert_eq!(inserts("SELECT 'na|' FROM users"), Vec::<String>::new());
        assert_eq!(inserts("SELECT 1 -- se|"), Vec::<String>::new());
        assert_eq!(inserts("SELECT 1 -- x\nSE|"), ["SELECT", "SET"]);
    }

    #[test]
    fn a_table_place_offers_tables_only() {
        assert_eq!(inserts("SELECT * FROM users, or|"), ["orders"]);
        assert_eq!(inserts("SELECT * FROM users u JOIN or|"), ["orders"]);
        assert_eq!(inserts("UPDATE us|"), ["users"]);
        // Past the table, keywords and columns come back.
        assert_eq!(inserts("SELECT * FROM users u WH|"), ["WHEN", "WHERE"]);
        assert_eq!(
            inserts("INSERT INTO orders (id, us|"),
            ["user_id", "users", "user", "using"]
        );
        assert_eq!(
            inserts("SELECT id, us| FROM orders"),
            ["user_id", "users", "user", "using"]
        );
    }

    #[test]
    fn a_schema_lists_its_tables_quoted_when_needed() {
        assert_eq!(inserts("SELECT * FROM audit.|"), ["\"Events\""]);
        assert_eq!(inserts("SELECT * FROM ev|"), ["audit.\"Events\""]);
    }

    #[test]
    fn a_word_offers_used_columns_tables_and_keywords_in_the_users_case() {
        assert_eq!(inserts("SELECT na| FROM users"), ["name"]);
        assert_eq!(inserts("SELECT * FROM us|"), ["users"]);
        assert_eq!(inserts("SEL|"), ["SELECT"]);
        assert_eq!(inserts("sel|"), ["select"]);
        // Before FROM names a table, every table's columns are offered;
        // after, only those of the tables it names.
        assert_eq!(inserts("SELECT tot|"), ["total"]);
        assert_eq!(inserts("SELECT tot| FROM users"), Vec::<String>::new());
        assert_eq!(inserts("SELECT | FROM users"), Vec::<String>::new());
        assert_eq!(inserts("SELECT|"), Vec::<String>::new());
    }

    #[test]
    fn a_quoted_name_completes_inside_its_quotes() {
        assert_eq!(inserts(r#"select "na|" from users"#), ["name"]);
        // Before FROM, as the query is typed left to right.
        assert_eq!(inserts(r#"select "na|"#), ["name"]);
        // Still open, as it is while being typed.
        assert_eq!(inserts(r#"select "na| from users"#), ["name"]);
        // No quotes of its own and no schema inside the quotes.
        assert_eq!(inserts(r#"SELECT * FROM "Ev|"#), ["Events"]);
        assert_eq!(inserts(r#"SELECT u."e| FROM users u"#), ["email"]);
        // A quoted alias, and a quoted name before the word.
        assert_eq!(inserts(r#"SELECT "u".n| FROM users "u""#), ["name"]);
        assert_eq!(inserts(r#"SELECT "name", em| FROM users"#), ["email"]);
    }
}
