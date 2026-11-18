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
        ssh_host -> Text,
        ssh_port -> Integer,
        ssh_user -> Text,
        ssh_password -> Text,
        ssh_key_path -> Text,
        kube_context -> Text,
        kube_namespace -> Text,
        kube_resource -> Text,
        kube_config -> Text,
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

diesel::joinable!(db_connections -> connection_groups (group_id));
diesel::joinable!(query_history -> db_connections (connection_id));

diesel::allow_tables_to_appear_in_same_query!(
    connection_groups,
    db_connections,
    query_history,
    settings,
);
