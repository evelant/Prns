use super::*;
use crate::{
    ChangeToken, PublishRecord, RecordChanges, RecordDeliveryProgress, RecordPage, StoredRecord,
};
use prns_groups_service::{GroupId, Record};

pub(super) fn progress(
    service: &Service<'_>,
    id: RecordId,
    clock: ClockReading,
) -> Result<RecordDeliveryProgress, Error> {
    let summary = service.progress(id, clock)?;
    Ok(RecordDeliveryProgress {
        membership_changed: !service.in_current_era(id)? && !summary.all_received(),
        summary,
    })
}

fn project(
    service: &Service<'_>,
    record: Record,
    sequence: u64,
    clock: ClockReading,
) -> Result<StoredRecord, Error> {
    let outgoing = record.metadata.author == service.local_endpoint();
    Ok(StoredRecord {
        progress: progress(service, record.metadata.record, clock)?,
        client_request_id: outgoing.then_some(record.nonce),
        record,
        sequence,
        outgoing,
    })
}

pub(super) fn page(
    service: &Service<'_>,
    cfg: &Provisioned,
    group: GroupId,
    before: Option<u64>,
    limit: usize,
    clock: ClockReading,
) -> Result<RecordPage, Error> {
    page_filtered(service, cfg, group, before, limit, clock, |_| true)
}

fn page_filtered(
    service: &Service<'_>,
    cfg: &Provisioned,
    group: GroupId,
    before: Option<u64>,
    limit: usize,
    clock: ClockReading,
    include: fn(&Record) -> bool,
) -> Result<RecordPage, Error> {
    if group != cfg.group || limit == 0 || limit > 100 {
        return Err(Error::InvalidInput);
    }
    let changes = service.changes(ChangeCursor(0), cfg.service_limits.max_changes)?;
    let mut records = Vec::new();
    for change in changes.changes.into_iter().rev() {
        if before.is_some_and(|before| change.cursor.0 >= before) {
            continue;
        }
        let id = match change.kind {
            ChangeKind::Published(id) | ChangeKind::Received(id) => id,
            _ => continue,
        };
        let record = service.record(id)?.ok_or(Error::Corrupt)?;
        if !include(&record) {
            continue;
        }
        records.push(project(service, record, change.cursor.0, clock)?);
        if records.len() > limit {
            break;
        }
    }
    let next_before_sequence = if records.len() > limit {
        records.pop();
        records.last().map(|record| record.sequence)
    } else {
        None
    };
    Ok(RecordPage {
        records,
        revision: changes.head.0,
        next_before_sequence,
    })
}

pub(super) fn record(
    service: &Service<'_>,
    cfg: &Provisioned,
    group: GroupId,
    id: RecordId,
    clock: ClockReading,
) -> Result<Option<StoredRecord>, Error> {
    if group != cfg.group {
        return Err(Error::InvalidInput);
    }
    let Some(record) = service.record(id)? else {
        return Ok(None);
    };
    let changes = service.changes(ChangeCursor(0), cfg.service_limits.max_changes)?;
    let sequence = changes
        .changes
        .into_iter()
        .find_map(|change| match change.kind {
            ChangeKind::Published(found) | ChangeKind::Received(found) if found == id => {
                Some(change.cursor.0)
            }
            _ => None,
        })
        .ok_or(Error::Corrupt)?;
    project(service, record, sequence, clock).map(Some)
}

pub(super) fn changes(
    service: &Service<'_>,
    cfg: &Provisioned,
    group: GroupId,
    after: Option<ChangeToken>,
    limit: usize,
) -> Result<RecordChanges, Error> {
    if group != cfg.group
        || limit == 0
        || limit > 100
        || after
            .is_some_and(|cursor| cursor.group != group || cursor.incarnation != cfg.incarnation)
    {
        return Err(Error::InvalidInput);
    }
    let position = after.map_or(0, |cursor| cursor.position);
    let page = service.changes(ChangeCursor(position), limit)?;
    let mut previous = position;
    let mut history_pruned = false;
    for change in &page.changes {
        history_pruned |= previous.checked_add(1) != Some(change.cursor.0);
        previous = change.cursor.0;
    }
    history_pruned |= page.next.0 > previous;
    let token = |position| ChangeToken {
        group,
        incarnation: cfg.incarnation,
        position,
    };
    Ok(RecordChanges {
        changes: page.changes,
        next: token(page.next.0),
        head: token(page.head.0),
        history_pruned,
    })
}

pub(super) fn messages(
    service: &Service<'_>,
    cfg: &Provisioned,
    group: GroupId,
    before: Option<u64>,
    limit: usize,
    clock: ClockReading,
) -> Result<MessagePage, Error> {
    let page = page_filtered(service, cfg, group, before, limit, clock, |record| {
        record.metadata.namespace == prns_group_text::NAMESPACE
            && record.metadata.schema == prns_group_text::SCHEMA
            && prns_group_text::Text::decode(&record.body).is_ok()
    })?;
    let mut messages = Vec::with_capacity(page.records.len());
    for item in page.records {
        let record = item.record;
        let author = if item.outgoing {
            cfg.local_reticulum_identity
        } else {
            cfg.peers
                .iter()
                .chain(cfg.history_peers.iter())
                .find(|peer| peer.endpoint == record.metadata.author)
                .ok_or(Error::Corrupt)?
                .identity
        };
        messages.push(Message {
            membership_changed: item.progress.membership_changed
                || (cfg.lifecycle != crate::GroupLifecycle::Active
                    && !item.progress.summary.all_received()),
            id: record.metadata.record,
            client_request_id: item.client_request_id,
            sequence: item.sequence,
            author,
            text: prns_group_text::Text::decode(&record.body)
                .map_err(|_| Error::Corrupt)?
                .as_str()
                .to_owned(),
            outgoing: item.outgoing,
            received_count: item.progress.summary.received,
            recipient_count: item.progress.summary.original_targets,
            sent_at_millis: record
                .metadata
                .validity
                .created_at()
                .0
                .checked_mul(1000)
                .ok_or(Error::Corrupt)?,
        });
    }
    Ok(MessagePage {
        messages,
        revision: page.revision,
        next_before_sequence: page.next_before_sequence,
    })
}

pub(super) fn publish_text(
    service: &mut Service<'_>,
    cfg: &Provisioned,
    request: SendText,
) -> Result<RecordId, Error> {
    let text = prns_group_text::Text::new(&request.text).map_err(|_| Error::InvalidInput)?;
    let mut buffer = [0; prns_group_text::MAX_ENCODED_BYTES];
    let body = text
        .encode(&mut buffer)
        .map_err(|_| Error::InvalidInput)?
        .to_vec();
    // Resolve the convenience selector once. Exact text retries keep original
    // targets after membership changes and remain compatible with old records.
    let mut targets = None;
    for change in service
        .changes(ChangeCursor(0), cfg.service_limits.max_changes)?
        .changes
    {
        if let ChangeKind::Published(id) = change.kind {
            let record = service.record(id)?.ok_or(Error::Corrupt)?;
            if record.nonce == request.client_request_id {
                targets = Some(record.targets);
                break;
            }
        }
    }
    let targets = targets.unwrap_or_else(|| {
        cfg.readers
            .iter()
            .copied()
            .filter(|reader| *reader != service.local_endpoint())
            .collect()
    });
    publish(
        service,
        cfg,
        PublishRecord {
            group: request.group,
            client_request_id: request.client_request_id,
            namespace: prns_group_text::NAMESPACE,
            schema: prns_group_text::SCHEMA,
            targets,
            body,
            clock: request.clock,
        },
    )
}
