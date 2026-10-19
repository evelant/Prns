use super::{files, Error};
use prns_groups_runner_native::{fixture, Client, Job, StoreSession, Submit};
use std::{
    io::Read,
    path::Path,
    sync::{mpsc, Arc, Mutex, Weak},
    thread,
};
use zeroize::Zeroizing;

enum Work {
    Job(Box<Job>),
    Stop,
}
struct Lane(Mutex<Option<mpsc::SyncSender<Work>>>);
struct BorrowedLane(Weak<Lane>);
impl Submit for BorrowedLane {
    fn submit(&self, job: Job) -> Result<(), prns_groups_runner_native::Error> {
        use prns_groups_runner_native::Error::{Busy, Unavailable};
        let lane = self.0.upgrade().ok_or(Unavailable)?;
        let sender = lane.0.lock().map_err(|_| Unavailable)?;
        sender
            .as_ref()
            .ok_or(Unavailable)?
            .try_send(Work::Job(Box::new(job)))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Busy,
                mpsc::TrySendError::Disconnected(_) => Unavailable,
            })
    }
}

pub(super) struct Storage {
    pub client: Client,
    lane: Arc<Lane>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Storage {
    pub fn open(root: &Path, identity: prns_host::IdentityHash) -> Result<Self, Error> {
        files::private_file(&root.join("group-fixture.json"))?;
        files::regular_file_if_present(&root.join("application.redb"))?;
        let mut fixture = Zeroizing::new(Vec::new());
        std::fs::File::open(root.join("group-fixture.json"))?
            .take(fixture::MAX_FIXTURE_BYTES as u64 + 1)
            .read_to_end(&mut fixture)?;
        if fixture.len() > fixture::MAX_FIXTURE_BYTES {
            return Err(Error::FixtureTooLarge);
        }
        let database_path = root.join("application.redb");
        let (sender, receiver) = mpsc::sync_channel(8);
        let lane = Arc::new(Lane(Mutex::new(Some(sender))));
        let client = Client::new(Arc::new(BorrowedLane(Arc::downgrade(&lane))));
        let (ready, readiness) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("group-peer-storage".into())
            .spawn(move || {
                let database = match redb::Database::create(database_path) {
                    Ok(database) => database,
                    Err(error) => {
                        let _ = ready.send(Err(Error::Database(error)));
                        return;
                    }
                };
                let session = fixture::load_and_install(&database, &fixture, identity)
                    .map_err(Error::Fixture)
                    .and_then(|provisioned| {
                        StoreSession::open(&database, provisioned).map_err(Error::Runner)
                    });
                drop(fixture);
                let mut session = match session {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_err() {
                    return;
                }
                while let Ok(work) = receiver.recv() {
                    match work {
                        Work::Job(job) => job.execute(&mut session),
                        Work::Stop => break,
                    }
                }
            })?;
        let storage = Self {
            client,
            lane,
            worker: Some(worker),
        };
        readiness.recv().map_err(|_| Error::StorageWorker)??;
        Ok(storage)
    }
    pub fn stop(&mut self) -> Result<(), Error> {
        // Fence new admission before putting Stop after every accepted operation.
        let sender = self.lane.0.lock().map_err(|_| Error::StorageWorker)?.take();
        if let Some(sender) = sender {
            let _ = sender.send(Work::Stop);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| Error::StorageWorker)?;
        }
        Ok(())
    }
}
impl Drop for Storage {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
