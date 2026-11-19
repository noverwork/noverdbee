//! Builds the SQL the app runs on the user's behalf. Values go in as quoted
//! literals rather than bound parameters, so the statement shown for review is
//! exactly the statement that runs.

use std::ops::Range;

/// A cell value: `None` is NULL.
pub type Value = Option<String>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableRef {
    /// `None` for SQLite, which has no schemas worth naming.
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
    /// The name to show: `schema.name`, or the bare name in the default schema.
    pub fn label(&self) -> String {
        match self.schema.as_deref() {
            None | Some("public") => self.name.clone(),
            Some(schema) => format!("{schema}.{}", self.name),
        }
    }

    pub fn quoted(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", quote_ident(schema), quote_ident(&self.name)),
            None => quote_ident(&self.name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ascending,
    Descending,
}

/// `"name"`, with embedded double quotes doubled.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `'text'` with embedded single quotes doubled, or `NULL`. Backslashes need
/// no escaping: SQLite never treats them specially, and PostgreSQL has not
/// since `standard_conforming_strings` became the default in 9.1.
pub fn quote_literal(value: &Value) -> String {
    match value {
        Some(text) => format!("'{}'", text.replace('\'', "''")),
        None => "NULL".to_string(),
    }
}

/// One page of a table. `filter` is the user's own WHERE condition.
pub fn page_query(
    table: &TableRef,
    filter: &str,
    order: Option<(&str, Direction)>,
    limit: usize,
    offset: usize,
) -> String {
    let mut sql = format!("SELECT * FROM {}", table.quoted());
    let filter = filter.trim();
    if !filter.is_empty() {
        sql.push_str(&format!(" WHERE {filter}"));
    }
    if let Some((column, direction)) = order {
        let direction = match direction {
            Direction::Ascending => "ASC",
            Direction::Descending => "DESC",
        };
        sql.push_str(&format!(" ORDER BY {} {direction}", quote_ident(column)));
    }
    sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    sql
}

/// `column = value` pairs joined by `separator`. `null_operator` is how a NULL
/// value is written: `=` when assigning, `IS` when matching.
fn pairs(columns: &[(String, Value)], separator: &str, null_operator: &str) -> String {
    columns
        .iter()
        .map(|(column, value)| {
            let operator = if value.is_none() { null_operator } else { "=" };
            format!(
                "{} {operator} {}",
                quote_ident(column),
                quote_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(separator)
}

/// Updates the row identified by `key` (its primary-key columns and values).
pub fn update(table: &TableRef, key: &[(String, Value)], changes: &[(String, Value)]) -> String {
    format!(
        "UPDATE {} SET {} WHERE {}",
        table.quoted(),
        pairs(changes, ", ", "="),
        pairs(key, " AND ", "IS")
    )
}

pub fn delete(table: &TableRef, key: &[(String, Value)]) -> String {
    format!(
        "DELETE FROM {} WHERE {}",
        table.quoted(),
        pairs(key, " AND ", "IS")
    )
}

/// Inserts a row; columns left out take their defaults.
pub fn insert(table: &TableRef, values: &[(String, Value)]) -> String {
    if values.is_empty() {
        return format!("INSERT INTO {} DEFAULT VALUES", table.quoted());
    }
    let columns: Vec<String> = values
        .iter()
        .map(|(column, _)| quote_ident(column))
        .collect();
    let literals: Vec<String> = values
        .iter()
        .map(|(_, value)| quote_literal(value))
        .collect();
    format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table.quoted(),
        columns.join(", "),
        literals.join(", ")
    )
}

/// The byte ranges of the statements in `sql`: it splits at semicolons outside
/// strings, quoted names, comments, and PostgreSQL dollar quotes. Blank
/// statements are left out.
pub fn split_statements(sql: &str) -> Vec<Range<usize>> {
    let mut statements = scan(sql).0;
    statements.retain(|range| !sql[range.clone()].trim().is_empty());
    statements
}

/// Whether `sql` may change data or schema: it names a writing keyword outside
/// strings, quoted names, and comments. `FOR UPDATE` does not count. It errs
/// toward yes, since it only decides whether to ask first.
pub fn writes(sql: &str) -> bool {
    const WRITES: [&str; 13] = [
        "insert", "update", "delete", "merge", "drop", "truncate", "alter", "create", "grant",
        "revoke", "replace", "upsert", "copy",
    ];
    let mut code = sql.as_bytes().to_vec();
    for range in scan(sql).1 {
        let end = range.end.min(code.len());
        code[range.start..end].fill(b' ');
    }
    let code = String::from_utf8_lossy(&code).to_ascii_lowercase();
    let mut previous = "";
    for word in code.split(|c: char| !(c == '_' || c.is_ascii_alphanumeric())) {
        if word.is_empty() {
            continue;
        }
        if WRITES.contains(&word) && !(word == "update" && previous == "for") {
            return true;
        }
        previous = word;
    }
    false
}

/// Where the string, quoted name, comment, or dollar-quoted body that a
/// cursor at byte `offset` sits inside begins: the byte of its opening quote
/// or marker. `None` when the cursor is in SQL code.
pub fn literal_at(sql: &str, offset: usize) -> Option<usize> {
    scan(sql)
        .1
        .into_iter()
        .find(|inside| inside.contains(&offset))
        .map(|inside| inside.start - 1)
}

/// The statements of `sql`, and the cursor offsets that fall inside
/// something other than SQL code.
fn scan(sql: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let bytes = sql.as_bytes();
    let ident = |byte: u8| byte == b'_' || byte.is_ascii_alphanumeric() || byte >= 0x80;
    let mut statements = Vec::new();
    let mut literals = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let from = at;
        // Whether a cursor at the very end is still inside: true for a line
        // comment and for anything left unclosed.
        let mut open = true;
        match bytes[at] {
            quote @ (b'\'' | b'"') => {
                // Only E'...' strings treat a backslash as an escape.
                let escapes = quote == b'\''
                    && at > 0
                    && bytes[at - 1].eq_ignore_ascii_case(&b'e')
                    && (at < 2 || !ident(bytes[at - 2]));
                at += 1;
                while at < bytes.len() {
                    if escapes && bytes[at] == b'\\' {
                        at += 2;
                    } else if bytes[at] == quote {
                        at += 1;
                        // A doubled quote stands for itself.
                        if bytes.get(at) != Some(&quote) {
                            open = false;
                            break;
                        }
                        at += 1;
                    } else {
                        at += 1;
                    }
                }
            }
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                at = sql[at..].find('\n').map_or(bytes.len(), |end| at + end);
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                // PostgreSQL nests block comments.
                let mut depth = 0;
                while at < bytes.len() {
                    if bytes[at..].starts_with(b"/*") {
                        depth += 1;
                        at += 2;
                    } else if bytes[at..].starts_with(b"*/") {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            open = false;
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
            }
            b'$' => {
                // `$tag$` or `$$`; `$1` is a parameter.
                let mut end = at + 1;
                if bytes.get(end).is_some_and(|byte| !byte.is_ascii_digit()) {
                    while bytes.get(end).copied().is_some_and(ident) {
                        end += 1;
                    }
                }
                if bytes.get(end) != Some(&b'$') {
                    at += 1;
                    continue;
                }
                let tag = &sql[at..=end];
                at = match sql[end + 1..].find(tag) {
                    Some(close) => {
                        open = false;
                        end + 1 + close + tag.len()
                    }
                    None => bytes.len(),
                };
            }
            b';' => {
                statements.push(start..at);
                at += 1;
                start = at;
                continue;
            }
            _ => {
                at += 1;
                continue;
            }
        }
        literals.push(from + 1..at.min(bytes.len()) + usize::from(open));
    }
    statements.push(start..bytes.len());
    (statements, literals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_writing_sql_from_reading_sql() {
        assert!(!writes("SELECT * FROM users WHERE name = 'drop table'"));
        assert!(!writes("select 1 -- delete later\n; /* update */"));
        assert!(!writes(r#"SELECT "delete" FROM t FOR UPDATE"#));
        assert!(writes("delete from users"));
        assert!(writes(
            "WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone"
        ));
        assert!(writes("EXPLAIN ANALYZE UPDATE t SET a = 1"));
        assert!(writes("select 1; DROP TABLE t"));
    }

    fn pair(column: &str, value: Option<&str>) -> (String, Value) {
        (column.to_string(), value.map(str::to_string))
    }

    #[test]
    fn quotes_cannot_be_broken_out_of() {
        assert_eq!(quote_ident(r#"we"ird"#), r#""we""ird""#);
        assert_eq!(quote_literal(&Some("O'Brien".into())), "'O''Brien'");
        assert_eq!(
            quote_literal(&Some("'; DROP TABLE t; --".into())),
            "'''; DROP TABLE t; --'"
        );
        assert_eq!(quote_literal(&Some(String::new())), "''");
        assert_eq!(quote_literal(&None), "NULL");
    }

    #[test]
    fn builds_statements_keyed_on_the_primary_key() {
        let table = TableRef {
            schema: Some("public".into()),
            name: "users".into(),
        };
        assert_eq!(
            update(
                &table,
                &[pair("id", Some("7")), pair("region", None)],
                &[pair("name", Some("bo")), pair("deleted_at", None)],
            ),
            r#"UPDATE "public"."users" SET "name" = 'bo', "deleted_at" = NULL WHERE "id" = '7' AND "region" IS NULL"#
        );
        assert_eq!(
            delete(&table, &[pair("id", Some("7"))]),
            r#"DELETE FROM "public"."users" WHERE "id" = '7'"#
        );
        assert_eq!(
            insert(&table, &[pair("name", Some("cy")), pair("age", None)]),
            r#"INSERT INTO "public"."users" ("name", "age") VALUES ('cy', NULL)"#
        );
        assert_eq!(
            insert(&table, &[]),
            r#"INSERT INTO "public"."users" DEFAULT VALUES"#
        );
    }

    #[test]
    fn pages_with_the_users_filter_and_order() {
        let table = TableRef {
            schema: None,
            name: "users".into(),
        };
        assert_eq!(
            page_query(&table, "", None, 200, 0),
            r#"SELECT * FROM "users" LIMIT 200 OFFSET 0"#
        );
        assert_eq!(
            page_query(
                &table,
                " age > 30 ",
                Some(("name", Direction::Descending)),
                200,
                400
            ),
            r#"SELECT * FROM "users" WHERE age > 30 ORDER BY "name" DESC LIMIT 200 OFFSET 400"#
        );
    }
    #[test]
    fn splits_statements_outside_quotes_and_comments() {
        let split = |sql: &str| {
            split_statements(sql)
                .into_iter()
                .map(|range| sql[range].trim().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(split("SELECT 1; SELECT 2;\n"), ["SELECT 1", "SELECT 2"]);
        assert_eq!(split("SELECT 'a;b', \"c;d\"; ;"), ["SELECT 'a;b', \"c;d\""]);
        assert_eq!(
            split("SELECT 'it''s;'; SELECT E'\\';'"),
            ["SELECT 'it''s;'", "SELECT E'\\';'"]
        );
        assert_eq!(
            split("SELECT 1 -- a;b\n; /* c; /* d; */ e; */ SELECT 2"),
            ["SELECT 1 -- a;b", "/* c; /* d; */ e; */ SELECT 2"]
        );
        assert_eq!(
            split("CREATE FUNCTION f() RETURNS int AS $body$ SELECT 1; $body$ LANGUAGE sql; SELECT $$;$$, $1"),
            ["CREATE FUNCTION f() RETURNS int AS $body$ SELECT 1; $body$ LANGUAGE sql", "SELECT $$;$$, $1"]
        );
        // `e'` inside a name is not an escape string.
        assert_eq!(
            split("SELECT name'a\\'; SELECT 2"),
            ["SELECT name'a\\'", "SELECT 2"]
        );
        assert_eq!(
            split("SELECT 'unterminated; SELECT 2"),
            ["SELECT 'unterminated; SELECT 2"]
        );
    }

    #[test]
    fn knows_when_a_cursor_is_inside_a_string_or_comment() {
        // Each `|` is a cursor: true inside, false outside.
        let cases: [(&str, &[bool]); 3] = [
            ("SELECT |'a|b|'| x|", &[false, true, true, false, false]),
            ("SELECT 1 |-- n|o|\n|x", &[false, true, true, false]),
            (
                "SELECT /* a|b */| $$x|$$ |'open|",
                &[true, false, true, false, true],
            ),
        ];
        for (marked, expected) in cases {
            let sql = marked.replace('|', "");
            let found: Vec<bool> = marked
                .match_indices('|')
                .enumerate()
                .map(|(n, (at, _))| literal_at(&sql, at - n).is_some())
                .collect();
            assert_eq!(found, expected, "{marked}");
        }
    }
}
