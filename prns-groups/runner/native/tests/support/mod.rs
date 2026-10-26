#![allow(dead_code)]
use prns_groups_runner_native::{Client, Clock, Error, Job, StoreSession, Submit};
use prns_groups_service::{
    ClockInterval, ClockReading, Endpoint, EndpointId, EndpointIncarnation, Timestamp,
};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
};

#[path = "../../../../transport/reticulum/tests/support/wire.rs"]
pub mod wire;

pub const NOW: u64 = 1_792_000_000;
pub fn endpoint(peer: usize) -> Endpoint {
    Endpoint {
        id: EndpointId([(peer + 1) as u8; 32]),
        incarnation: EndpointIncarnation::new(1).unwrap(),
    }
}
pub fn clock() -> ClockReading {
    ClockReading::Trusted(ClockInterval::new(Timestamp(NOW), Timestamp(NOW)).unwrap())
}
pub struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> ClockReading {
        clock()
    }
}

enum Work {
    Job(Box<Job>),
    Stop,
}
struct Lane {
    sender: mpsc::SyncSender<Work>,
    busy_admissions: Arc<std::sync::atomic::AtomicUsize>,
}
impl Submit for Lane {
    fn submit(&self, job: Job) -> Result<(), Error> {
        if {
            let mut previous = self
                .busy_admissions
                .load(std::sync::atomic::Ordering::SeqCst);
            loop {
                let Some(next) = previous.checked_sub(1) else {
                    break Err(previous);
                };
                match self.busy_admissions.compare_exchange_weak(
                    previous,
                    next,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                ) {
                    Ok(previous) => break Ok(previous),
                    Err(current) => previous = current,
                }
            }
        }
        .is_ok()
        {
            return Err(Error::Busy);
        }
        self.sender
            .try_send(Work::Job(Box::new(job)))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Error::Busy,
                mpsc::TrySendError::Disconnected(_) => Error::Unavailable,
            })
    }
}
pub struct Worker {
    pub client: Client,
    pub busy_admissions: Arc<std::sync::atomic::AtomicUsize>,
    sender: mpsc::SyncSender<Work>,
    thread: thread::JoinHandle<()>,
}
impl Worker {
    pub fn open(path: PathBuf, fixture: Vec<u8>, identity: prns_host::IdentityHash) -> Self {
        let (sender, receiver) = mpsc::sync_channel(8);
        let busy_admissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let client = Client::new(Arc::new(Lane {
            sender: sender.clone(),
            busy_admissions: busy_admissions.clone(),
        }));
        let (ready, readiness) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let db = redb::Database::create(path).unwrap();
            let cfg = prns_groups_runner_native::fixture::load_and_install(&db, &fixture, identity)
                .unwrap();
            let mut session = StoreSession::open(&db, cfg).unwrap();
            ready.send(()).unwrap();
            while let Ok(work) = receiver.recv() {
                match work {
                    Work::Job(job) => job.execute(&mut session),
                    Work::Stop => break,
                }
            }
        });
        readiness.recv().unwrap();
        Self {
            client,
            busy_admissions,
            sender,
            thread,
        }
    }
    pub fn stop(self) {
        self.sender.send(Work::Stop).unwrap();
        self.thread.join().unwrap();
    }
}
