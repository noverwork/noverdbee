//! What the backend needs from the application around it: shared state looked
//! up by type and the Tokio runtime backend work runs on.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

const APP_IDENTIFIER: &str = "com.noverwork.noverdbee";

/// The runtime every backend future runs on. The UI has its own executor, so
/// it spawns backend work here and awaits the join handle.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start the Tokio runtime")
    })
}

/// Where the database lives. `NOVERDBEE_DATA_DIR` overrides it, for tests and
/// for running a second copy against scratch data.
pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("NOVERDBEE_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    dirs::data_dir()
        .map(|dir| dir.join(APP_IDENTIFIER))
        .ok_or_else(|| "no application data folder available".to_string())
}

#[derive(Default)]
struct Inner {
    states: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

#[derive(Clone, Default)]
pub struct AppHandle {
    inner: Arc<Inner>,
}

impl AppHandle {
    pub fn manage<T: Send + Sync + 'static>(&self, state: T) {
        self.inner
            .states
            .write()
            .expect("state lock")
            .insert(TypeId::of::<T>(), Arc::new(state));
    }

    /// Panics when `T` was never managed, which is a startup bug.
    pub fn state<T: Send + Sync + 'static>(&self) -> State<'_, T> {
        let state = self
            .inner
            .states
            .read()
            .expect("state lock")
            .get(&TypeId::of::<T>())
            .cloned()
            .unwrap_or_else(|| panic!("state {} is not managed", std::any::type_name::<T>()));
        State {
            value: state
                .downcast()
                .expect("state is stored under its own type"),
            lifetime: PhantomData,
        }
    }
}

pub struct State<'a, T> {
    value: Arc<T>,
    lifetime: PhantomData<&'a ()>,
}

impl<T> State<'_, T> {
    pub fn inner(&self) -> &T {
        &self.value
    }
}

impl<T> Deref for State<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_state_is_found_by_type() {
        let app = AppHandle::default();
        app.manage(7_u32);
        assert_eq!(*app.state::<u32>(), 7);
    }
}
