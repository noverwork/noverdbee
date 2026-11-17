use tracing_subscriber::EnvFilter;

use crate::app::AppHandle;

/// Opens the database and registers every backend service.
pub fn backend() -> Result<AppHandle, String> {
    std::fs::create_dir_all(crate::app::data_dir()?).map_err(|error| error.to_string())?;
    let database = crate::store::database_path()?;
    if let Err(error) = crate::store::apply_pending_restore(&database) {
        tracing::error!("{error}");
    }
    let pool = crate::store::init_pool(&database)?;

    let app = AppHandle::default();
    app.manage(pool);
    Ok(app)
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let app = backend().expect("failed to start the backend");
    crate::ui::run(app);
}
