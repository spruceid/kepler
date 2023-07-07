use kepler_lib::resource::OrbitId;
use std::{
    collections::HashMap,
    ops::{AddAssign, SubAssign},
    sync::Arc,
};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct OrbitSizes(Arc<RwLock<HashMap<OrbitId, u64>>>);

impl OrbitSizes {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
    pub async fn init_size(&self, orbit: OrbitId) {
        self.0.write().await.insert(orbit, 0);
    }
    pub async fn increment_size(&self, orbit: &OrbitId, size: u64) {
        self.0
            .write()
            .await
            .get_mut(orbit)
            .map(|s| s.add_assign(size));
    }
    pub async fn decrement_size(&self, orbit: &OrbitId, size: u64) {
        self.0
            .write()
            .await
            .get_mut(orbit)
            .map(|s| s.sub_assign(size));
    }
    pub async fn get_size(&self, orbit: &OrbitId) -> Option<u64> {
        self.0.read().await.get(orbit).copied()
    }
}

impl From<HashMap<OrbitId, u64>> for OrbitSizes {
    fn from(map: HashMap<OrbitId, u64>) -> Self {
        Self(Arc::new(RwLock::new(
            map.into_iter().map(|(k, v)| (k, v)).collect(),
        )))
    }
}
