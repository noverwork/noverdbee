//! Query plans: `EXPLAIN` run for each statement and read into a tree.

use serde_json::Value as Json;

use super::{sql, Outcome, ResultSet, Session};

/// One step of a plan. Costs and estimates are PostgreSQL's; the `actual_*`
/// fields are filled only by `EXPLAIN ANALYZE`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlanNode {
    /// `Index Scan using users_pkey on users u`
    pub operation: String,
    /// Conditions, keys, and filters: `Index Cond: (id = 1)`
    pub detail: String,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub rows: Option<f64>,
    /// Milliseconds for one loop, as PostgreSQL reports it.
    pub actual_time: Option<f64>,
    pub actual_rows: Option<f64>,
    pub loops: Option<f64>,
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// The statement that was explained.
    pub sql: String,
    pub roots: Vec<PlanNode>,
    /// The plan as the database gave it.
    pub raw: String,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
}

/// The name of the savepoint `EXPLAIN ANALYZE` runs under in an open
/// transaction.
const SAVEPOINT: &str = "noverdbee_explain";

/// The plan of each statement in `script`, or the error that stopped it.
/// `analyze` runs the statements (PostgreSQL only) inside a transaction or
/// savepoint that is always rolled back, so their writes never stay.
// ponytail: another tab on the same connection could slip a statement in
// between BEGIN and ROLLBACK; give EXPLAIN ANALYZE its own lock if that bites.
pub async fn explain(session: &Session, script: &str, analyze: bool) -> Vec<Result<Plan, String>> {
    let statements: Vec<String> = sql::split_statements(script)
        .into_iter()
        .map(|range| script[range].trim().to_string())
        .collect();
    let postgres = matches!(session, Session::Postgres(_));
    let prefix = match (postgres, analyze) {
        (false, _) => "EXPLAIN QUERY PLAN",
        (true, false) => "EXPLAIN (FORMAT JSON)",
        (true, true) => "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)",
    };
    let explains = statements
        .iter()
        .map(|statement| format!("{prefix} {statement}"))
        .collect::<Vec<_>>()
        .join(";\n");

    let (results, closed) = if postgres && analyze {
        let (open, close) = if session.in_transaction().await {
            (
                format!("SAVEPOINT {SAVEPOINT}"),
                format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"),
            )
        } else {
            ("BEGIN".to_string(), "ROLLBACK".to_string())
        };
        let results = session.run(format!("{open};\n{explains}")).await;
        // Rolled back even when EXPLAIN failed, which leaves the block aborted.
        (results, session.run(close).await)
    } else {
        (session.run(explains).await, Vec::new())
    };

    let mut statements = statements.into_iter();
    let mut plans = Vec::new();
    for result in results.into_iter().chain(closed) {
        match result.outcome {
            Outcome::Rows(rows) => {
                let sql = statements.next().unwrap_or_default();
                let plan = if postgres {
                    from_postgres(&rows)
                } else {
                    Ok(from_sqlite(&rows))
                };
                plans.push(plan.map(|plan| Plan { sql, ..plan }));
            }
            Outcome::Error { message, .. } => plans.push(Err(message)),
            // BEGIN, SAVEPOINT, ROLLBACK.
            Outcome::Affected(_) => {}
        }
    }
    plans
}

/// Reads the one-cell answer of `EXPLAIN (FORMAT JSON)`.
fn from_postgres(rows: &ResultSet) -> Result<Plan, String> {
    let text = rows
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.as_deref())
        .ok_or("EXPLAIN returned no plan")?;
    let json: Json = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let top = json.get(0).ok_or("EXPLAIN returned an empty plan")?;
    let root = top.get("Plan").ok_or("EXPLAIN returned no Plan")?;
    Ok(Plan {
        sql: String::new(),
        roots: vec![postgres_node(root)],
        raw: serde_json::to_string_pretty(&json).unwrap_or_else(|_| text.to_string()),
        planning_ms: top.get("Planning Time").and_then(Json::as_f64),
        execution_ms: top.get("Execution Time").and_then(Json::as_f64),
    })
}

/// One node of PostgreSQL's JSON plan, named the way its text format names it.
fn postgres_node(node: &Json) -> PlanNode {
    let text = |key: &str| node.get(key).and_then(Json::as_str);
    let number = |key: &str| node.get(key).and_then(Json::as_f64);
    // ModifyTable reads as `Insert on t`, as in the text format.
    let name = match text("Node Type") {
        Some("ModifyTable") => text("Operation"),
        other => other,
    };
    let mut operation = name.unwrap_or("?").to_string();
    match text("Join Type") {
        Some(join) if join != "Inner" && operation == "Nested Loop" => {
            operation = format!("Nested Loop {join} Join");
        }
        Some(join) if join != "Inner" => {
            operation = operation.replace("Join", &format!("{join} Join"))
        }
        _ => {}
    }
    if let Some(index) = text("Index Name") {
        operation.push_str(&format!(" using {index}"));
    }
    if let Some(relation) = text("Relation Name")
        .or(text("CTE Name"))
        .or(text("Function Name"))
    {
        operation.push_str(&format!(" on {relation}"));
        if let Some(alias) = text("Alias").filter(|alias| *alias != relation) {
            operation.push_str(&format!(" {alias}"));
        }
    }

    const CONDITIONS: [&str; 9] = [
        "Index Cond",
        "Recheck Cond",
        "Hash Cond",
        "Merge Cond",
        "Join Filter",
        "Filter",
        "Sort Key",
        "Group Key",
        "Rows Removed by Filter",
    ];
    let detail = CONDITIONS
        .iter()
        .filter_map(|key| {
            let value = match node.get(*key)? {
                Json::String(text) => text.clone(),
                Json::Array(items) => items
                    .iter()
                    .filter_map(Json::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            Some(format!("{key}: {value}"))
        })
        .collect::<Vec<_>>()
        .join("; ");

    PlanNode {
        operation,
        detail,
        startup_cost: number("Startup Cost"),
        total_cost: number("Total Cost"),
        rows: number("Plan Rows"),
        actual_time: number("Actual Total Time"),
        actual_rows: number("Actual Rows"),
        loops: number("Actual Loops"),
        children: node
            .get("Plans")
            .and_then(Json::as_array)
            .map(|plans| plans.iter().map(postgres_node).collect())
            .unwrap_or_default(),
    }
}

/// Builds the tree from `EXPLAIN QUERY PLAN` rows: `id, parent, notused,
/// detail`, parents before their children.
fn from_sqlite(rows: &ResultSet) -> Plan {
    let cell = |row: &Vec<Option<String>>, index: usize| {
        row.get(index).cloned().flatten().unwrap_or_default()
    };
    // (id, parent, node), in the order SQLite listed them.
    let mut flat: Vec<(String, String, PlanNode)> = rows
        .rows
        .iter()
        .map(|row| {
            let node = PlanNode {
                operation: cell(row, 3),
                ..PlanNode::default()
            };
            (cell(row, 0), cell(row, 1), node)
        })
        .collect();
    // Children come after their parent, so folding from the end attaches
    // every node before its parent is moved.
    let mut roots = Vec::new();
    while let Some((_, parent, node)) = flat.pop() {
        match flat.iter_mut().rev().find(|(id, ..)| *id == parent) {
            Some((.., parent)) => parent.children.insert(0, node),
            None => roots.insert(0, node),
        }
    }
    let mut raw = String::from("QUERY PLAN\n");
    fn draw(nodes: &[PlanNode], indent: &str, raw: &mut String) {
        for (index, node) in nodes.iter().enumerate() {
            let last = index + 1 == nodes.len();
            raw.push_str(&format!(
                "{indent}{}{}\n",
                if last { "`--" } else { "|--" },
                node.operation
            ));
            let deeper = format!("{indent}{}", if last { "   " } else { "|  " });
            draw(&node.children, &deeper, raw);
        }
    }
    draw(&roots, "", &mut raw);
    Plan {
        roots,
        raw,
        ..Plan::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Column;

    fn cells(rows: &[&[&str]]) -> ResultSet {
        ResultSet {
            columns: vec![Column {
                name: String::new(),
                type_name: String::new(),
                numeric: false,
            }],
            rows: rows
                .iter()
                .map(|row| row.iter().map(|cell| Some(cell.to_string())).collect())
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn reads_a_postgres_plan_with_actual_times() {
        let json = r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left",
            "Startup Cost": 1.5, "Total Cost": 40.25, "Plan Rows": 10,
            "Actual Total Time": 0.42, "Actual Rows": 3, "Actual Loops": 1,
            "Hash Cond": "(o.user_id = u.id)",
            "Plans": [
              {"Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o",
               "Filter": "(total > 0)", "Rows Removed by Filter": 2},
              {"Node Type": "Index Scan", "Index Name": "users_pkey",
               "Relation Name": "users", "Alias": "users"}
            ]},
            "Planning Time": 0.1, "Execution Time": 0.5}]"#;
        let plan = from_postgres(&cells(&[&[json]])).expect("plan");
        let root = &plan.roots[0];
        assert_eq!(root.operation, "Hash Left Join");
        assert_eq!(root.detail, "Hash Cond: (o.user_id = u.id)");
        assert_eq!((root.total_cost, root.actual_rows), (Some(40.25), Some(3.)));
        assert_eq!(root.children[0].operation, "Seq Scan on orders o");
        assert_eq!(
            root.children[0].detail,
            "Filter: (total > 0); Rows Removed by Filter: 2"
        );
        assert_eq!(
            root.children[1].operation,
            "Index Scan using users_pkey on users"
        );
        assert_eq!(
            (plan.planning_ms, plan.execution_ms),
            (Some(0.1), Some(0.5))
        );
    }

    #[test]
    fn nests_sqlite_rows_under_their_parents() {
        let rows = cells(&[
            &["2", "0", "0", "SCAN a"],
            &["3", "0", "0", "SEARCH b USING INDEX b_a (a=?)"],
            &["5", "0", "0", "SCALAR SUBQUERY 1"],
            &["7", "5", "0", "SCAN c"],
        ]);
        let plan = from_sqlite(&rows);
        let operations: Vec<_> = plan
            .roots
            .iter()
            .map(|node| node.operation.as_str())
            .collect();
        assert_eq!(
            operations,
            [
                "SCAN a",
                "SEARCH b USING INDEX b_a (a=?)",
                "SCALAR SUBQUERY 1"
            ]
        );
        assert_eq!(plan.roots[2].children[0].operation, "SCAN c");
        assert!(
            plan.raw.ends_with("`--SCALAR SUBQUERY 1\n   `--SCAN c\n"),
            "{}",
            plan.raw
        );
    }
}
