//! What connections tunnel through: saved SSH keys, SSH hosts and kubeconfigs.

use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use orm::models::{
    Kubeconfig, NewKubeconfig, NewSshHost, NewSshKey, SshHost, SshKey, UpdateKubeconfig,
    UpdateSshHost, UpdateSshKey,
};
use orm::schema::{kubeconfigs, ssh_hosts, ssh_keys};
use shared::{KubeconfigRecord, SshHostRecord, SshKeyRecord};
use uuid::Uuid;

use super::{internal_error, run_db, DbPool};
use crate::db::tunnel;

fn named(name: &str, what: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("{what} name is required"));
    }
    Ok(name.to_string())
}

// --- SSH keys ---------------------------------------------------------------

pub(super) fn load_keys(connection: &mut SqliteConnection) -> QueryResult<Vec<SshKeyRecord>> {
    let keys = ssh_keys::table
        .order(ssh_keys::name.asc())
        .load::<SshKey>(connection)?;
    Ok(keys.into_iter().map(key_record).collect())
}

pub async fn ssh_key_list(
    pool: crate::app::State<'_, DbPool>,
) -> Result<Vec<SshKeyRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        load_keys(connection).map_err(internal_error)
    })
    .await
}

/// Creates the key when its `id` is empty, otherwise updates it. A key that
/// cannot be read with its passphrase is not saved.
pub async fn ssh_key_save(
    record: SshKeyRecord,
    pool: crate::app::State<'_, DbPool>,
) -> Result<SshKeyRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = named(&record.name, "key")?;
        let fingerprint = tunnel::key_fingerprint(&record.private_key, &record.passphrase)?;
        let now = Utc::now().naive_utc();
        let fields = UpdateSshKey {
            name,
            private_key: record.private_key,
            passphrase: record.passphrase,
            fingerprint,
            updated_at: now,
        };
        if record.id.is_empty() {
            diesel::insert_into(ssh_keys::table)
                .values(&NewSshKey {
                    id: Uuid::new_v4().to_string(),
                    created_at: now,
                    fields,
                })
                .get_result::<SshKey>(connection)
        } else {
            diesel::update(ssh_keys::table.find(record.id))
                .set(&fields)
                .get_result::<SshKey>(connection)
        }
        .map(key_record)
        .map_err(internal_error)
    })
    .await
}

/// Deletes the key. Hosts that used it keep their password, if any.
pub async fn ssh_key_delete(id: String, pool: crate::app::State<'_, DbPool>) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        diesel::delete(ssh_keys::table.find(id))
            .execute(connection)
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

fn key_record(row: SshKey) -> SshKeyRecord {
    SshKeyRecord {
        id: row.id,
        name: row.name,
        private_key: row.private_key,
        passphrase: row.passphrase,
        fingerprint: row.fingerprint,
    }
}

// --- SSH hosts --------------------------------------------------------------

pub(super) fn load_hosts(connection: &mut SqliteConnection) -> QueryResult<Vec<SshHostRecord>> {
    let hosts = ssh_hosts::table
        .order(ssh_hosts::name.asc())
        .load::<SshHost>(connection)?;
    Ok(hosts.into_iter().map(host_record).collect())
}

pub async fn ssh_host_list(
    pool: crate::app::State<'_, DbPool>,
) -> Result<Vec<SshHostRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        load_hosts(connection).map_err(internal_error)
    })
    .await
}

/// Creates the host when its `id` is empty, otherwise updates it.
pub async fn ssh_host_save(
    record: SshHostRecord,
    pool: crate::app::State<'_, DbPool>,
) -> Result<SshHostRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let now = Utc::now().naive_utc();
        let fields = UpdateSshHost {
            name: named(&record.name, "host")?,
            host: record.host,
            port: record.port,
            username: record.username,
            password: record.password,
            ssh_key_id: record.key_id,
            updated_at: now,
        };
        if record.id.is_empty() {
            diesel::insert_into(ssh_hosts::table)
                .values(&NewSshHost {
                    id: Uuid::new_v4().to_string(),
                    created_at: now,
                    fields,
                })
                .get_result::<SshHost>(connection)
        } else {
            diesel::update(ssh_hosts::table.find(record.id))
                .set(&fields)
                .get_result::<SshHost>(connection)
        }
        .map(host_record)
        .map_err(internal_error)
    })
    .await
}

/// Deletes the host. Connections that tunnelled through it must pick another.
pub async fn ssh_host_delete(
    id: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        diesel::delete(ssh_hosts::table.find(id))
            .execute(connection)
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

fn host_record(row: SshHost) -> SshHostRecord {
    SshHostRecord {
        id: row.id,
        name: row.name,
        host: row.host,
        port: row.port,
        username: row.username,
        password: row.password,
        key_id: row.ssh_key_id,
    }
}

// --- Kubeconfigs ------------------------------------------------------------

pub(super) fn load_kubeconfigs(
    connection: &mut SqliteConnection,
) -> QueryResult<Vec<KubeconfigRecord>> {
    let kubeconfigs = kubeconfigs::table
        .order(kubeconfigs::name.asc())
        .load::<Kubeconfig>(connection)?;
    Ok(kubeconfigs.into_iter().map(kubeconfig_record).collect())
}

pub async fn kubeconfig_list(
    pool: crate::app::State<'_, DbPool>,
) -> Result<Vec<KubeconfigRecord>, String> {
    let pool = pool.inner().clone();
    run_db(pool, |connection| {
        load_kubeconfigs(connection).map_err(internal_error)
    })
    .await
}

/// Creates the kubeconfig when its `id` is empty, otherwise updates it. YAML
/// that is not a kubeconfig is not saved.
pub async fn kubeconfig_save(
    record: KubeconfigRecord,
    pool: crate::app::State<'_, DbPool>,
) -> Result<KubeconfigRecord, String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        let name = named(&record.name, "kubeconfig")?;
        tunnel::check_kubeconfig(&record.config)?;
        let now = Utc::now().naive_utc();
        let fields = UpdateKubeconfig {
            name,
            config: record.config,
            updated_at: now,
        };
        if record.id.is_empty() {
            diesel::insert_into(kubeconfigs::table)
                .values(&NewKubeconfig {
                    id: Uuid::new_v4().to_string(),
                    created_at: now,
                    fields,
                })
                .get_result::<Kubeconfig>(connection)
        } else {
            diesel::update(kubeconfigs::table.find(record.id))
                .set(&fields)
                .get_result::<Kubeconfig>(connection)
        }
        .map(kubeconfig_record)
        .map_err(internal_error)
    })
    .await
}

/// Deletes the kubeconfig. Connections that used it must pick another.
pub async fn kubeconfig_delete(
    id: String,
    pool: crate::app::State<'_, DbPool>,
) -> Result<(), String> {
    let pool = pool.inner().clone();
    run_db(pool, move |connection| {
        diesel::delete(kubeconfigs::table.find(id))
            .execute(connection)
            .map(|_| ())
            .map_err(internal_error)
    })
    .await
}

fn kubeconfig_record(row: Kubeconfig) -> KubeconfigRecord {
    KubeconfigRecord {
        id: row.id,
        name: row.name,
        config: row.config,
    }
}
