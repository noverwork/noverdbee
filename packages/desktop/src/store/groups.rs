use diesel::prelude::*;
use orm::models::ConnectionGroup;
use orm::schema::connection_groups;
use shared::GroupRecord;

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
