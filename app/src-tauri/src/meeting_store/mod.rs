mod migrations;
mod repository;
mod search;
mod types;

#[cfg(test)]
pub(crate) use repository::checked_opens;
pub use repository::{InitializationOutcome, MeetingRepository};
pub use types::*;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::MutexExt;

#[derive(Clone, Default)]
pub struct MeetingStore {
    inner: Arc<Mutex<MeetingStoreInner>>,
    initialization: Arc<Mutex<()>>,
}

#[derive(Default)]
struct MeetingStoreInner {
    repository: Option<MeetingRepository>,
    status: MeetingStoreStatus,
}

impl MeetingStore {
    pub fn initialize(&self, root: PathBuf) -> MeetingStoreStatus {
        let _initialization = self.initialization.lock_or_recover();
        let result = MeetingRepository::initialize(root).map(|(repository, outcome)| {
            let status = repository.status().unwrap_or_default();
            (repository, outcome, status)
        });
        let mut inner = self.inner.lock_or_recover();
        match result {
            Ok((repository, outcome, mut status)) => {
                status.availability = match outcome {
                    InitializationOutcome::Opened => MeetingStoreAvailability::Available,
                    InitializationOutcome::Recovered | InitializationOutcome::Reinitialized => {
                        MeetingStoreAvailability::Recovered
                    }
                };
                inner.repository = Some(repository);
                inner.status = status.clone();
                status
            }
            Err(_) => {
                inner.repository = None;
                inner.status = MeetingStoreStatus::default();
                inner.status.clone()
            }
        }
    }

    pub fn repository(&self) -> Result<MeetingRepository, String> {
        self.inner
            .lock_or_recover()
            .repository
            .clone()
            .ok_or_else(|| "The local meeting transcript store is unavailable.".to_string())
    }

    pub fn status(&self) -> MeetingStoreStatus {
        let inner = self.inner.lock_or_recover();
        match inner
            .repository
            .as_ref()
            .and_then(|repository| repository.status().ok())
        {
            Some(mut status) => {
                status.availability = inner.status.availability;
                status
            }
            None => inner.status.clone(),
        }
    }
}
