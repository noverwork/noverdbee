//! Builds the SQL the app runs on the user's behalf. Values go in as quoted
//! literals rather than bound parameters, so the statement shown for review is
//! exactly the statement that runs.

/// A cell value: `None` is NULL.
pub type Value = Option<String>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableRef {
    /// `None` for SQLite, which has no schemas worth naming.
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
