#![forbid(unsafe_code)]

//! Optional native development consumer. No CLI ownership leaks into the library.
mod files;
mod storage;

use personal_rns::{
    identity::PrivateIdentityMaterial,
    tcp::{TcpClientInterface, TcpServer},
};
use prns_groups_reticulum::{destination_config, destination_for, Ingress, Network, SyncBudget};
use prns_groups_runner_native::{Clock, RunnerControl, RunnerOptions, SendText, SystemClock};
use prns_host::{
    DestinationConfig, HostConfig, HostRole, IdentityConfig, IdentityHash, IdentitySecret,
    InterfaceKind, PersistenceConfig, PrnsLimits,
};
use prns_host_native::{
    owner::OwnedSession, ApplicationEventDispatch, NativeEmbedding, NativePreparedAttachment,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
use zeroize::Zeroizing;

const MAX_COMMAND_BYTES: usize = 16_384;
const LIMITS: prns_groups_reticulum::Limits = prns_groups_reticulum::Limits {
    max_frame_bytes: 65_536,
    max_page_items: 64,
    max_peers: 16,
    inbound_jobs: 8,
    exchange_timeout: Duration::from_secs(10),
};
#[derive(Debug)]
enum Error {
    Usage,
    Io(std::io::Error),
    Identity(personal_rns::IdentitySecretFileError),
    IdentityLength,
    PrivatePath(PathBuf),
    FixtureTooLarge,
    Fixture(prns_groups_runner_native::fixture::FixtureError),
    Database(redb::DatabaseError),
    StorageWorker,
    Runner(prns_groups_runner_native::Error),
    Network(prns_groups_reticulum::NetworkError),
    Host(prns_host_native::owner::SessionError),
    Worker(tokio::task::JoinError),
    Json(serde_json::Error),
    CommandTooLarge,
    RequestId,
    TimedOut,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage => f.write_str("usage: dev-group-peer init ROOT | run ROOT (--listen HOST:PORT | --connect HOST:PORT) [--poll-ms 100..60000]; ROOT must retain identity, fixture and database together"),
            Self::Io(error) => write!(f, "peer file or TCP interface: {error}"),
            Self::Identity(error) => write!(f, "identity file: {error:?}"),
            Self::IdentityLength => f.write_str("identity file must contain exactly 64 bytes; run init first"),
            Self::PrivatePath(path) => write!(f, "{} must be a non-symlink private directory (0700) or regular private file (0600); existing permissions are never changed automatically", path.display()),
            Self::FixtureTooLarge => f.write_str("fixture exceeds the 4 MiB limit"),
            Self::Fixture(error) => write!(f, "fixture: {error}"),
            Self::Database(error) => write!(f, "database: {error}"),
            Self::StorageWorker => f.write_str("storage worker unavailable"),
            Self::Runner(error) => error.fmt(f),
            Self::Network(error) => write!(f, "group transport: {error}"),
            Self::Host(error) => write!(f, "Reticulum host: {error:?}"),
            Self::Worker(error) => write!(f, "synchronization worker: {error}"),
            Self::Json(error) => write!(f, "command JSON: {error}"),
            Self::CommandTooLarge => f.write_str("command exceeds 16384 bytes"),
            Self::RequestId => f.write_str("request_id must be 64 hexadecimal characters"),
            Self::TimedOut => f.write_str("command timed out; retry a send with the same request_id and text"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    RequestId,
    Send { request_id: String, text: String },
    History { before: Option<u64>, limit: usize },
    Status,
    Stop,
}
enum Connection {
    Listen(String),
    Connect(String),
}
enum PreparedConnection {
    Server(Box<TcpServer>),
    Client(String),
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn execute() -> Result<(), Error> {
    let mut arguments = std::env::args().skip(1);
    let operation = arguments.next().ok_or(Error::Usage)?;
    let root = PathBuf::from(arguments.next().ok_or(Error::Usage)?);
    if operation == "init" {
        if arguments.next().is_some() {
            return Err(Error::Usage);
        }
        files::create_private_directory(&root)?;
        let identity_path = root.join("identity");
        files::regular_file_if_present(&identity_path)?;
        if identity_path.exists() {
            files::private_file(&identity_path)?;
        }
        let secret = personal_rns::load_or_create_identity_secret(&root.join("identity"))
            .map_err(Error::Identity)?;
        fs::File::open(&root)?.sync_all()?;
        return output(&metadata(&secret)?);
    }
    if operation != "run" {
        return Err(Error::Usage);
    }
    let connection = match arguments.next().as_deref() {
        Some("--listen") => Connection::Listen(arguments.next().ok_or(Error::Usage)?),
        Some("--connect") => Connection::Connect(arguments.next().ok_or(Error::Usage)?),
        _ => return Err(Error::Usage),
    };
    let poll_ms = match arguments.next().as_deref() {
        None => 15_000,
        Some("--poll-ms") => arguments
            .next()
            .ok_or(Error::Usage)?
            .parse::<u64>()
            .map_err(|_| Error::Usage)?,
        _ => return Err(Error::Usage),
    };
    if !(100..=60_000).contains(&poll_ms) || arguments.next().is_some() {
        return Err(Error::Usage);
    }
    files::private_directory(&root)?;
    files::private_file(&root.join("identity"))?;
    files::create_private_directory(&root.join("network"))?;
    let secret = read_identity(&root)?;
    let identity = identity_hash(&secret);
    let mut store = storage::Storage::open(&root, identity)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (owner, sync, stop, listen) = runtime.block_on(async {
        let config = store.client.network_config().await.map_err(Error::Runner)?;
        let destination = destination_for(identity).map_err(Error::Network)?;
        let (ingress, requests) = Ingress::new(destination, LIMITS).map_err(Error::Network)?;
        let (connection, listen) = match connection {
            Connection::Listen(address) => {
                let server = TcpServer::bind(address).await?;
                let listen = server.local_addr()?.to_string();
                (PreparedConnection::Server(Box::new(server)), Some(listen))
            }
            Connection::Connect(address) => (PreparedConnection::Client(address), None),
        };
        let embedding = NativeEmbedding {
            application_events: ApplicationEventDispatch::NativeCallback,
            on_event: Some(Box::new(move |event| {
                ingress.on_event_ref(&event);
            })),
            prepare_interfaces: Some(Box::new(move |client| {
                let attachment = match connection {
                    PreparedConnection::Server(server) => NativePreparedAttachment::Supervisor {
                        attachment: client.protocols().supervise(*server),
                        kind: InterfaceKind::TcpServer,
                    },
                    PreparedConnection::Client(address) => NativePreparedAttachment::Interface {
                        attachment: client.protocols().attach(TcpClientInterface::new(address)),
                        kind: InterfaceKind::TcpClient,
                    },
                };
                Ok(vec![attachment])
            })),
            ..NativeEmbedding::default()
        };
        let owner = OwnedSession::open_with_embedding(
            HostConfig {
                identity: IdentityConfig::Existing(IdentitySecret::new(*secret)),
                persistence: PersistenceConfig::Directory {
                    path: root.join("network").to_string_lossy().into_owned(),
                },
                role: HostRole::Transport,
                destinations: vec![DestinationConfig::Single(
                    destination_config(LIMITS).map_err(Error::Network)?,
                )],
                required_capabilities: Vec::new(),
                limits: PrnsLimits::balanced(),
            },
            embedding,
        )
        .await
        .map_err(Error::Host)?;
        let network = match Network::new(owner.client(), destination, config.peers, LIMITS) {
            Ok(network) => Arc::new(network),
            Err(error) => {
                let _ = owner.stop().await;
                return Err(Error::Network(error));
            }
        };
        let (stop, control) = watch::channel(RunnerControl::Running);
        let client = store.client.clone();
        let sync = tokio::spawn(prns_groups_runner_native::run(
            client,
            network,
            requests,
            RunnerOptions {
                poll_interval: Duration::from_millis(poll_ms),
                retry_initial: Duration::from_millis(250),
                retry_max: Duration::from_secs(5),
                budget: SyncBudget {
                    max_requests: 32,
                    max_envelopes: 8,
                    max_bytes: 262_144,
                },
            },
            Arc::new(SystemClock),
            control,
        ));
        Ok::<_, Error>((owner, sync, stop, listen))
    })?;
    drop(secret);
    let mut sync = Some(sync);
    let result = (|| {
        let mut ready = metadata_identity(identity)?;
        ready["event"] = json!("ready");
        ready["listen"] = json!(listen);
        output(&ready)?;
        // A synchronous stdin loop avoids Tokio's uncancellable stdin helper
        // holding the runtime open after a stop command. The runner has its own task.
        let mut input = std::io::stdin().lock();
        let mut sync_failure = None;
        loop {
            let line = match read_command(&mut input) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(Error::CommandTooLarge) => {
                    output_error(&Error::CommandTooLarge)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let command: Command = match serde_json::from_slice(&line) {
                Ok(command) => command,
                Err(error) => {
                    output_error(&Error::Json(error))?;
                    continue;
                }
            };
            if matches!(command, Command::Stop) {
                break;
            }
            if sync
                .as_ref()
                .is_some_and(tokio::task::JoinHandle::is_finished)
            {
                let Some(completed) = sync.take() else {
                    continue;
                };
                sync_failure = Some(match runtime.block_on(completed) {
                    Ok(Err(error)) => error.to_string(),
                    Ok(Ok(())) => "synchronization stopped unexpectedly".into(),
                    Err(error) => error.to_string(),
                });
            }
            let response = runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    if matches!(command, Command::RequestId) {
                        let id = prns_groups_runner_native::create_request_id().map_err(Error::Runner)?;
                        return Ok(json!({"event":"request_id", "request_id":hex(&id)}));
                    }
                    let conversation = store.client.conversations().await
                        .map_err(Error::Runner)?.into_iter().next()
                        .ok_or(Error::Runner(prns_groups_runner_native::Error::NotProvisioned))?;
                    match command {
                        Command::Status => {
                            owner.client().snapshot().await.map_err(Error::Host)?;
                            Ok(json!({
                                "event": "status",
                                "identity": hex(identity.as_bytes()),
                                "group": hex(&conversation.group.0),
                                "title": conversation.title,
                                "revision": conversation.revision.to_string(),
                                "members": conversation.member_count,
                                "synchronization": if sync_failure.is_some() { "unavailable" } else { "running" },
                                "detail": sync_failure,
                            }))
                        }
                        Command::Send { request_id, text } => {
                            let id = parse_request_id(&request_id)?;
                            let record = store.client.send_text(SendText {
                                group: conversation.group,
                                client_request_id: id,
                                text,
                                clock: SystemClock.now(),
                            }).await.map_err(Error::Runner)?;
                            Ok(json!({
                                "event": "accepted",
                                "request_id": hex(&id),
                                "record_id": hex(&record.0),
                            }))
                        }
                        Command::History { before, limit } => {
                            let page = store.client.messages(conversation.group, before, limit, SystemClock.now())
                                .await.map_err(Error::Runner)?;
                            let messages: Vec<_> = page.messages.into_iter().map(|message| json!({
                                "record_id": hex(&message.id.0),
                                "request_id": message.client_request_id.map(|id| hex(&id)),
                                "sequence": message.sequence.to_string(),
                                "author": hex(message.author.as_bytes()),
                                "text": message.text,
                                "outgoing": message.outgoing,
                                "received_count": message.received_count,
                                "recipient_count": message.recipient_count,
                                "sent_at_millis": message.sent_at_millis.to_string(),
                            })).collect();
                            Ok(json!({
                                "event": "history",
                                "revision": page.revision.to_string(),
                                "next_before_sequence": page.next_before_sequence.map(|sequence| sequence.to_string()),
                                "messages": messages,
                            }))
                        }
                        Command::Stop | Command::RequestId => unreachable!("handled before admission"),
                    }
                }).await.map_err(|_| Error::TimedOut)?
            });
            match response {
                Ok(response) => output(&response)?,
                Err(error) => output_error(&error)?,
            }
        }
        Ok(())
    })();
    // The runner is joined before its host or storage owner is destroyed.
    let shutdown = runtime.block_on(async {
        stop.send_replace(RunnerControl::Stop);
        let joined = match sync.take() {
            Some(sync) => sync.await.map_err(Error::Worker).map(|_| ()),
            None => Ok(()),
        };
        let stopped = owner.stop().await.map_err(Error::Host);
        joined.and(stopped)
    });
    let storage = store.stop();
    result.and(shutdown).and(storage)?;
    output(&json!({"event":"stopped"}))
}

fn read_identity(root: &Path) -> Result<Zeroizing<[u8; 64]>, Error> {
    let mut file = fs::File::open(root.join("identity"))?;
    if file.metadata()?.len() != 64 {
        return Err(Error::IdentityLength);
    }
    let mut secret = Zeroizing::new([0; 64]);
    file.read_exact(&mut *secret)?;
    Ok(secret)
}
fn identity_hash(secret: &[u8; 64]) -> IdentityHash {
    IdentityHash::new(
        *PrivateIdentityMaterial::from_bytes(*secret)
            .identity_hash()
            .as_bytes(),
    )
}
fn metadata(secret: &[u8; 64]) -> Result<Value, Error> {
    metadata_identity(identity_hash(secret))
}
fn metadata_identity(identity: IdentityHash) -> Result<Value, Error> {
    let destination = destination_for(identity).map_err(Error::Network)?;
    Ok(
        json!({"event":"identity", "identity":hex(identity.as_bytes()), "destination":hex(destination.as_bytes())}),
    )
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn parse_request_id(text: &str) -> Result<[u8; 32], Error> {
    if text.len() != 64 {
        return Err(Error::RequestId);
    }
    let mut bytes = [0; 32];
    for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        bytes[index] =
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| Error::RequestId)?, 16)
                .map_err(|_| Error::RequestId)?;
    }
    Ok(bytes)
}
fn read_command(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, Error> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return if oversized {
                Err(Error::CommandTooLarge)
            } else if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(chunk.len(), |end| end + 1);
        if line.len() + take <= MAX_COMMAND_BYTES && !oversized {
            line.extend_from_slice(&chunk[..take]);
        } else {
            oversized = true;
        }
        input.consume(take);
        if end.is_some() {
            return if oversized {
                Err(Error::CommandTooLarge)
            } else {
                Ok(Some(line))
            };
        }
    }
}
fn output(value: &Value) -> Result<(), Error> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
fn output_error(error: &Error) -> Result<(), Error> {
    let code = match error {
        Error::Runner(error) if error.is_retry_expired() => "retry_expired",
        Error::Runner(error) if error.is_storage_full() => "storage_full",
        Error::Runner(prns_groups_runner_native::Error::IdempotencyConflict) => {
            "idempotency_conflict"
        }
        Error::Runner(prns_groups_runner_native::Error::Busy) => "busy",
        Error::TimedOut => "timeout",
        Error::Json(_)
        | Error::CommandTooLarge
        | Error::RequestId
        | Error::Runner(prns_groups_runner_native::Error::InvalidInput) => "invalid_input",
        _ => "unavailable",
    };
    output(&json!({"event":"error", "code":code, "detail":error.to_string()}))
}
