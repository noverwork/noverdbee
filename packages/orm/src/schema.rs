// @generated automatically by Diesel CLI.

diesel::table! {
    connection_groups (id) {
        id -> Text,
        name -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    db_connections (id) {
        id -> Text,
        name -> Text,
        driver -> Text,
        group_id -> Nullable<Text>,
        host -> Text,
        port -> Integer,
        database -> Text,
        username -> Text,
        password -> Text,
        ssl_mode -> Text,
        path -> Text,
        read_only -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        environment -> Text,
        mcp_select -> Bool,
        mcp_insert -> Bool,
        mcp_update -> Bool,
        mcp_delete -> Bool,
        tunnel -> Text,
        kube_context -> Text,
        kube_namespace -> Text,
        kube_resource -> Text,
        ssh_host_id -> Nullable<Text>,
        kubeconfig_id -> Nullable<Text>,
    }
}

diesel::table! {
    kubeconfigs (id) {
        id -> Text,
        name -> Text,
        config -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    query_history (id) {
        id -> Text,
        connection_id -> Text,
        sql -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    settings (key) {
        key -> Text,
        value -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    ssh_hosts (id) {
        id -> Text,
        name -> Text,
        host -> Text,
        port -> Integer,
        username -> Text,
        password -> Text,
        ssh_key_id -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    ssh_keys (id) {
        id -> Text,
        name -> Text,
        private_key -> Text,
        passphrase -> Text,
        fingerprint -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(db_connections -> connection_groups (group_id));
diesel::joinable!(db_connections -> kubeconfigs (kubeconfig_id));
diesel::joinable!(db_connections -> ssh_hosts (ssh_host_id));
diesel::joinable!(ssh_hosts -> ssh_keys (ssh_key_id));
diesel::joinable!(query_history -> db_connections (connection_id));

diesel::allow_tables_to_appear_in_same_query!(
    connection_groups,
    db_connections,
    kubeconfigs,
    query_history,
    settings,
    ssh_hosts,
    ssh_keys,
);
