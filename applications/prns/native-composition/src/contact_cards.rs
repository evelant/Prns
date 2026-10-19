//! Offline card operations on the existing native identity and database owners.
use std::convert::Infallible;
use std::path::Path;

use prns_contact_card::{CardFields, ContactCardSigner, MAX_CARD_BYTES, MAX_URI_BYTES};
use prns_core::identity::vault::{FileVault, FileVaultError, IdentityLabel, IdentityVault};
use prns_core::identity::PrivateIdentityMaterial;
use redb::Database;

use crate::contract::*;
use crate::development_store::DevelopmentStoreFailure;
use crate::directory::DirectoryResponse;

pub(crate) enum Request {
    Export(ExportContactCardInput),
    Import(ImportContactCardInput),
}

pub(crate) enum Reply {
    Export(ContactCardExportOutcome),
    Import(ContactCardImportOutcome),
}

pub(crate) enum Failure {
    IdentityChanged,
    MissingIdentity,
    Busy,
    Unavailable(String),
    ResetRequired(String),
}

impl From<DevelopmentStoreFailure> for Failure {
    fn from(value: DevelopmentStoreFailure) -> Self {
        match value {
            DevelopmentStoreFailure::Unavailable(detail) => Self::Unavailable(detail),
            DevelopmentStoreFailure::ResetRequired(reason) => Self::ResetRequired(reason),
        }
    }
}

impl Failure {
    pub(crate) fn export(self) -> ContactCardExportOutcome {
        match self {
            Self::IdentityChanged => ContactCardExportOutcome::IdentityChanged,
            Self::MissingIdentity => ContactCardExportOutcome::MissingIdentity,
            Self::Busy => ContactCardExportOutcome::Busy,
            Self::Unavailable(detail) => ContactCardExportOutcome::Unavailable { detail },
            Self::ResetRequired(reason) => {
                ContactCardExportOutcome::DevelopmentResetRequired { reason }
            }
        }
    }

    pub(crate) fn import(self) -> ContactCardImportOutcome {
        match self {
            Self::IdentityChanged => ContactCardImportOutcome::IdentityChanged,
            Self::MissingIdentity => ContactCardImportOutcome::MissingIdentity,
            Self::Busy => ContactCardImportOutcome::Busy,
            Self::Unavailable(detail) => ContactCardImportOutcome::Unavailable { detail },
            Self::ResetRequired(reason) => {
                ContactCardImportOutcome::DevelopmentResetRequired { reason }
            }
        }
    }
}

impl Request {
    pub(crate) fn expected_identity(&self) -> [u8; 16] {
        match self {
            Self::Export(input) => input.expected_local_identity,
            Self::Import(input) => input.expected_local_identity,
        }
    }
}

pub(crate) fn preview(uri: &str) -> ContactCardPreviewOutcome {
    let mut wire = [0_u8; MAX_CARD_BYTES];
    let length = match prns_contact_card::decode_uri(uri, &mut wire) {
        Ok(length) => length,
        Err(_) => {
            return ContactCardPreviewOutcome::Invalid {
                detail: "This is not a supported, complete contact-card link.".into(),
            }
        }
    };
    let card = match prns_contact_card::verify(&wire[..length]) {
        Ok(card) => card,
        Err(_) => {
            return ContactCardPreviewOutcome::Invalid {
                detail: "The contact card is malformed or its signature could not be verified."
                    .into(),
            }
        }
    };
    if *card.service_name_hash() != messaging_service_hash() {
        return ContactCardPreviewOutcome::UnsupportedService;
    }
    ContactCardPreviewOutcome::Valid {
        card: ContactCard {
            uri: uri.to_owned(),
            display_name: card.display_name().to_owned(),
            identity: *card.identity_hash(),
            destination: *card.destination_hash(),
            service_name_hash: *card.service_name_hash(),
        },
    }
}

pub(crate) fn check_import(
    input: &ImportContactCardInput,
) -> Result<ContactCard, ContactCardImportOutcome> {
    if input.alias.as_ref().is_some_and(|alias| {
        alias.len() > prns_contact_card::MAX_DISPLAY_NAME_BYTES
            || alias.chars().any(char::is_control)
    }) {
        return Err(ContactCardImportOutcome::Invalid {
            detail: "Use a contact name of at most 128 UTF-8 bytes without control characters."
                .into(),
        });
    }
    match preview(&input.uri) {
        ContactCardPreviewOutcome::Valid { card } => Ok(card),
        ContactCardPreviewOutcome::Invalid { detail } => {
            Err(ContactCardImportOutcome::Invalid { detail })
        }
        ContactCardPreviewOutcome::UnsupportedService => {
            Err(ContactCardImportOutcome::UnsupportedService)
        }
    }
}

pub(crate) fn execute(database: &Database, root: &Path, request: Request) -> Reply {
    // This executes on the drained store lane. Reset cannot replace/remove the
    // identity until admitted work completes. Recheck the persisted identity,
    // not just the UI or supervisor snapshot, immediately before durable work.
    let identity = load_identity(root, request.expected_identity());
    match request {
        Request::Export(_) => Reply::Export(match identity {
            Ok(identity) => export(database, &identity).unwrap_or_else(Failure::export),
            Err(failure) => failure.export(),
        }),
        Request::Import(input) => Reply::Import(match identity {
            Ok(_) => import(database, input),
            Err(failure) => failure.import(),
        }),
    }
}

/// The immutable scoped owner supplies its already verified key. Requests still
/// pin the expected hash at execution; a stale UI cannot sign/import for another
/// identity and no operation falls back to the installation's primary key.
pub(crate) fn execute_scoped(
    database: &Database,
    identity: &PrivateIdentityMaterial,
    request: Request,
) -> Reply {
    if identity.identity_hash().as_bytes() != &request.expected_identity() {
        return match request {
            Request::Export(_) => Reply::Export(Failure::IdentityChanged.export()),
            Request::Import(_) => Reply::Import(Failure::IdentityChanged.import()),
        };
    }
    match request {
        Request::Export(_) => {
            Reply::Export(export(database, identity).unwrap_or_else(Failure::export))
        }
        Request::Import(input) => Reply::Import(import(database, input)),
    }
}

fn load_identity(root: &Path, expected: [u8; 16]) -> Result<PrivateIdentityMaterial, Failure> {
    let vault = FileVault::new(root.join("identities"));
    let label = IdentityLabel::new("primary")
        .map_err(|_| Failure::Unavailable("The primary identity label is invalid.".into()))?;
    let secret = vault
        .load(&label)
        .map_err(|error| match error {
            FileVaultError::Io(error) => {
                Failure::Unavailable(format!("Could not read the local identity: {error}"))
            }
            other => Failure::ResetRequired(format!("The local identity is malformed: {other}")),
        })?
        .ok_or(Failure::MissingIdentity)?;
    let identity = PrivateIdentityMaterial::from(secret);
    if *identity.identity_hash().as_bytes() != expected {
        return Err(Failure::IdentityChanged);
    }
    Ok(identity)
}

struct Signer<'a>(&'a PrivateIdentityMaterial);
impl ContactCardSigner for Signer<'_> {
    type Error = Infallible;
    fn sign(&self, message: &[u8]) -> Result<[u8; 64], Self::Error> {
        Ok(self.0.sign(message).0)
    }
}

fn messaging_service_hash() -> [u8; 10] {
    prns_contact_card::service_name_hash("lxmf.delivery")
}

fn export(
    database: &Database,
    identity: &PrivateIdentityMaterial,
) -> Result<ContactCardExportOutcome, Failure> {
    let name = crate::development_store::messaging_name(database, None)?;
    let mut wire = [0_u8; MAX_CARD_BYTES];
    let length = prns_contact_card::encode_signed(
        CardFields {
            public_identity: identity.public().as_bytes(),
            service_name_hash: &messaging_service_hash(),
            display_name: &name,
        },
        &Signer(identity),
        &mut wire,
    )
    .map_err(|_| Failure::Unavailable("Could not sign the contact card.".into()))?;
    let mut uri = [0_u8; MAX_URI_BYTES];
    let length = prns_contact_card::encode_uri(&wire[..length], &mut uri)
        .map_err(|_| Failure::Unavailable("Could not encode the contact card.".into()))?;
    let uri = std::str::from_utf8(&uri[..length])
        .map_err(|_| Failure::Unavailable("Could not encode the contact-card link.".into()))?;
    match preview(uri) {
        ContactCardPreviewOutcome::Valid { card } => Ok(ContactCardExportOutcome::Ready { card }),
        _ => Err(Failure::Unavailable(
            "Could not validate the signed contact card.".into(),
        )),
    }
}

fn import(database: &Database, input: ImportContactCardInput) -> ContactCardImportOutcome {
    let card = match check_import(&input) {
        Ok(card) => card,
        Err(outcome) => return outcome,
    };
    // A signed name is only an initial private alias. It is not an announce,
    // identity pin, verified person, live key cache entry or route observation.
    let alias = input
        .alias
        .or_else(|| (!card.display_name.is_empty()).then_some(card.display_name));
    match crate::directory::import_contact_card(database, card.destination, card.identity, alias) {
        Ok(DirectoryResponse::Mutation(ContactMutationOutcome::Saved { contact })) => {
            ContactCardImportOutcome::Saved { contact }
        }
        Ok(DirectoryResponse::Mutation(ContactMutationOutcome::Updated { contact })) => {
            ContactCardImportOutcome::Updated { contact }
        }
        Ok(DirectoryResponse::Mutation(ContactMutationOutcome::Existing { contact })) => {
            ContactCardImportOutcome::Existing { contact }
        }
        Ok(DirectoryResponse::Mutation(ContactMutationOutcome::IdentityConflict {
            existing,
            attempted,
        })) => ContactCardImportOutcome::IdentityConflict {
            existing,
            attempted,
        },
        Err(failure) => Failure::from(failure).import(),
        _ => {
            Failure::Unavailable("The directory returned an unexpected contact-card result.".into())
                .import()
        }
    }
}

#[cfg(test)]
mod tests;
