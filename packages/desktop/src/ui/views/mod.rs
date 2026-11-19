//! One module per page, plus the update check.

pub mod common;
pub mod db_connection_form;
pub mod kubeconfigs;
pub mod mcp;
pub mod navigator;
pub mod query_plan;
pub mod result_grid;
pub mod selection;
pub mod settings;
pub mod sql_completion;
pub mod sql_editor;
pub mod ssh_hosts;
pub mod ssh_keys;
pub mod table_view;
pub mod updater;
pub mod workspace;

use gpui_kit::*;

use crate::ui::i18n::t;
use crate::ui::store::Route;

/// Builds the page for `route`. The Databases page is not built here: the
/// shell keeps its workspace alive across routes.
pub fn build(route: &Route, window: &mut Window, cx: &mut App) -> AnyView {
    match route {
        Route::Databases => cx.new(|cx| workspace::Workspace::new(window, cx)).into(),
        Route::ConnectionForm(id) => {
            let id = id.clone();
            cx.new(|cx| db_connection_form::DbConnectionForm::new(id, None, window, cx))
                .into()
        }
        Route::NewConnectionIn(group) => {
            let group = Some(group.clone());
            cx.new(|cx| db_connection_form::DbConnectionForm::new(None, group, window, cx))
                .into()
        }
        Route::SshHosts => cx.new(|cx| ssh_hosts::SshHostsView::new(window, cx)).into(),
        Route::SshHostForm(id) => {
            let id = id.clone();
            cx.new(|cx| ssh_hosts::SshHostForm::new(id, window, cx))
                .into()
        }
        Route::SshKeys => cx.new(|cx| ssh_keys::SshKeysView::new(window, cx)).into(),
        Route::SshKeyForm(id) => {
            let id = id.clone();
            cx.new(|cx| ssh_keys::SshKeyForm::new(id, window, cx))
                .into()
        }
        Route::Kubeconfigs => cx
            .new(|cx| kubeconfigs::KubeconfigsView::new(window, cx))
            .into(),
        Route::KubeconfigForm(id) => {
            let id = id.clone();
            cx.new(|cx| kubeconfigs::KubeconfigForm::new(id, window, cx))
                .into()
        }
        Route::Mcp => cx.new(|cx| mcp::McpView::new(window, cx)).into(),
        Route::Settings => cx.new(|cx| settings::SettingsView::new(window, cx)).into(),
    }
}

/// The drawer title and width for a form route.
pub fn form_frame(route: &Route) -> (SharedString, Pixels) {
    let new_or_edit = |id: &Option<String>, new: &'static str, edit: &'static str| {
        if id.is_some() {
            edit
        } else {
            new
        }
    };
    // The connection form puts fields side by side and needs the room.
    let (title, width) = match route {
        Route::ConnectionForm(id) => (new_or_edit(id, "New connection", "Edit connection"), 640.),
        Route::NewConnectionIn(_) => ("New connection", 640.),
        Route::SshHostForm(id) => (new_or_edit(id, "New SSH host", "Edit SSH host"), 560.),
        Route::SshKeyForm(id) => (new_or_edit(id, "New SSH key", "Edit SSH key"), 560.),
        Route::KubeconfigForm(id) => (new_or_edit(id, "New kubeconfig", "Edit kubeconfig"), 560.),
        _ => ("", 560.),
    };
    (t(title), px(width))
}

/// Starts the update check that lives for the whole window.
pub fn start_update_check<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    // Tests and offline setups can skip the request to GitHub. Version
    // 0.0.0 is a local build (releases get theirs from the tag), and every
    // release would look newer to it.
    let local_build = env!("CARGO_PKG_VERSION") == "0.0.0";
    if !local_build && std::env::var_os("NOVERDBEE_SKIP_UPDATE_CHECK").is_none() {
        updater::check(window, cx).detach();
    }
}
