use chrono::Utc;
use diesel::prelude::*;
use orm::models::ConnectionGroup;
use orm::schema::{connection_groups, db_connections};
use shared::GroupRecord;

use super::connections::drop_empty_groups;
use super::{internal_error, run_db, DbPool};

/// Groups are made and removed by saving connections; see
/// [`super::connections::connection_save`].
pub async fn group_list(pool: crate::app::State<'_, DbPool>) -> Result<Vec<GroupRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        connection_groups::table
            .order(connection_groups::name.asc())
            .load::<ConnectionGroup>(connection)
            .map(|groups| {
                groups
                    .into_iter()
                    .map(|group| GroupRecord {
                        id: group.id,
                        name: group.name,
                    })
                    .collect()
            })
            .map_err(internal_error)
    })
    .await
}

/// Renames a group. Renaming it to another group's name moves its
/// connections into that group.
pub async fn group_rename(
    id: String,
    name: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("group name is required".to_string());
        }
        connection
            .transaction(|connection| {
                let existing = connection_groups::table
                    .filter(connection_groups::name.eq(&name))
                    .filter(connection_groups::id.ne(&id))
                    .select(connection_groups::id)
                    .first::<String>(connection)
                    .optional()?;
                match existing {
                    Some(target) => {
                        diesel::update(
                            db_connections::table.filter(db_connections::group_id.eq(&id)),
                        )
                        .set(db_connections::group_id.eq(target))
                        .execute(connection)?;
                        drop_empty_groups(connection)?;
                    }
                    None => {
                        diesel::update(connection_groups::table.find(&id))
                            .set((
                                connection_groups::name.eq(&name),
                                connection_groups::updated_at.eq(Utc::now().naive_utc()),
                            ))
                            .execute(connection)?;
                    }
                }
                Ok(())
            })
            .map_err(internal_error)
    })
    .await
}

/// Takes every connection out of a group, which then goes away.
pub async fn group_dissolve(id: String, pool: crate::app::State<'_, DbPool>) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        connection
            .transaction(|connection| {
                diesel::update(db_connections::table.filter(db_connections::group_id.eq(&id)))
                    .set(db_connections::group_id.eq(None::<String>))
                    .execute(connection)?;
                drop_empty_groups(connection)
            })
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}
