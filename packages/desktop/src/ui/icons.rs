//! Icons by their Lucide file name.

use gpui_kit::component::Icon;

/// A Lucide icon by its kebab-case name, e.g. `folder-open`.
pub fn lucide(name: &str) -> Icon {
    Icon::default().path(format!("icons/{name}.svg"))
}
