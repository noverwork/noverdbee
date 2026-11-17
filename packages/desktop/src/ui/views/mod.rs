//! One module per page, plus the update check.

pub mod common;
pub mod db_connection_form;
pub mod mcp;
pub mod navigator;
pub mod result_grid;
pub mod selection;
pub mod settings;
pub mod sql_editor;
pub mod table_view;
pub mod updater;
pub mod workspace;

use gpui_kit::*;

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
        Route::Mcp => cx.new(|cx| mcp::McpView::new(window, cx)).into(),
        Route::Settings => cx.new(|cx| settings::SettingsView::new(window, cx)).into(),
    }
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
