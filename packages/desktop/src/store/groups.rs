use chrono::Utc;
use diesel::prelude::*;
use orm::models::{ConnectionGroup, NewConnectionGroup};
use orm::schema::connection_groups;
use shared::GroupRecord;
use uuid::Uuid;

use super::{internal_error, run_db, DbPool};

pub async fn group_list(pool: crate::app::State<'_, DbPool>) -> Result<Vec<GroupRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        connection_groups::table
            .order(connection_groups::name.asc())
            .load::<ConnectionGroup>(connection)
            .map(|groups| groups.into_iter().map(to_record).collect())
            .map_err(internal_error)
    })
    .await
}

pub async fn group_create(
    name: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<GroupRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("group name is required".to_string());
        }

        let now = Utc::now().naive_utc();
        diesel::insert_into(connection_groups::table)
            .values(&NewConnectionGroup {
                id: Uuid::new_v4().to_string(),
                name,
                created_at: now,
                updated_at: now,
            })
            .get_result::<ConnectionGroup>(connection)
            .map(to_record)
            .map_err(|error| match error {
                diesel::result::Error::DatabaseError(
                    diesel::result::DatabaseErrorKind::UniqueViolation,
                    _,
                ) => "host group already exists".to_string(),
                other => internal_error(other),
            })
    })
    .await
}

pub async fn group_delete(id: String, pool: crate::app::State<'_, DbPool>) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        diesel::delete(connection_groups::table.filter(connection_groups::id.eq(id)))
            .execute(connection)
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

fn to_record(group: ConnectionGroup) -> GroupRecord {
    GroupRecord {
        id: group.id,
        name: group.name,
    }
}
