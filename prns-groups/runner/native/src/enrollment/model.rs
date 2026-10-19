use super::*;

pub const MAX_FRAME_BYTES: usize = 131_072;
pub const MAX_GROUPS: usize = 4;
pub const MAX_INVITATIONS: usize = 16;
pub const INVITATION_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;
pub(super) const STATE_KEY: &[u8] = b"enrollment/state/v1";
pub(super) const MAX_CATALOG_BYTES: usize = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationState {
    Pending,
    JoinRequested,
    Joined,
    Declined,
    Expired,
}
#[derive(Clone, Debug)]
pub struct Invitation {
    pub id: [u8; 32],
    pub group: GroupId,
    pub title: String,
    pub inviter: IdentityHash,
    pub recipient: IdentityHash,
    pub expires_at_secs: u64,
    pub outgoing: bool,
    pub state: InvitationState,
    /// The accepted join is retrying while the owner finishes its current
    /// authored deliveries. This does not assert that every member is drained.
    pub waiting_for_delivery: bool,
}
#[derive(Clone, Debug)]
pub struct Work {
    pub id: [u8; 32],
    pub recipient: IdentityHash,
    pub frame: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Member {
    pub rns: [u8; 16],
    pub endpoint: [u8; 32],
    pub incarnation: u64,
    pub key: [u8; 32],
}
impl Member {
    pub fn binding(&self) -> MemberBinding {
        MemberBinding {
            endpoint: self.endpoint,
            incarnation: self.incarnation,
            signing_key: self.key,
        }
    }
    pub fn endpoint(&self) -> Result<Endpoint, EnrollmentError> {
        Ok(Endpoint {
            id: EndpointId(self.endpoint),
            incarnation: EndpointIncarnation::new(self.incarnation)
                .map_err(|_| EnrollmentError::Invalid)?,
        })
    }
    pub fn new(rns: IdentityHash, binding: MemberBinding) -> Self {
        Self {
            rns: rns.into_bytes(),
            endpoint: binding.endpoint,
            incarnation: binding.incarnation,
            key: binding.signing_key,
        }
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub version: u8,
    pub group: [u8; 32],
    pub owner: [u8; 16],
    pub title: String,
    pub parent: [u8; 32],
    pub revision: u64,
    pub epoch: u64,
    pub commit_digest: [u8; 32],
    pub members: Vec<Member>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Signed {
    pub manifest: Manifest,
    #[serde(with = "wire_bytes")]
    pub signature: Vec<u8>,
}
impl Signed {
    pub fn head(&self) -> Result<[u8; 32], EnrollmentError> {
        Ok(hash(&encode(self)?))
    }
    pub fn verify(
        &self,
        group: [u8; 32],
        owner: &Member,
        title: &str,
    ) -> Result<(), EnrollmentError> {
        let m = &self.manifest;
        if m.version != 1
            || m.group != group
            || m.owner != owner.rns
            || m.title != title
            || m.members.is_empty()
            || m.members.len() > 8
            || m.revision != m.epoch
            || Some(m.members.len() as u64) != m.revision.checked_add(1)
            || !m.members.contains(owner)
            || self.signature.len() != 64
            || m.members.windows(2).any(|p| p[0].endpoint >= p[1].endpoint)
            || m.members.iter().enumerate().any(|(i, x)| {
                x.incarnation == 0
                    || m.members[..i]
                        .iter()
                        .any(|y| x.rns == y.rns || x.key == y.key)
            })
        {
            return Err(EnrollmentError::Authentication);
        }
        owner
            .binding()
            .verify_enrollment(&encode(m)?, &self.signature)
            .map_err(|_| EnrollmentError::Authentication)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Control {
    pub signed: Signed,
    #[serde(with = "wire_bytes")]
    pub commit: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Added {
    pub invitation: [u8; 32],
    pub request_hash: [u8; 32],
    pub signed: Signed,
    #[serde(with = "wire_bytes")]
    pub welcome: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub current: Option<Signed>,
    pub required_tip: Option<Signed>,
    pub package: Option<Vec<u8>>,
    pub controls: Vec<Control>,
    pub added: Vec<Added>,
}
impl Snapshot {
    pub fn ready(&self) -> Result<bool, EnrollmentError> {
        let Some(current) = &self.current else {
            return Ok(false);
        };
        let Some(tip) = &self.required_tip else {
            return Ok(true);
        };
        if current.manifest.epoch < tip.manifest.epoch {
            return Ok(false);
        }
        if current.manifest.epoch == tip.manifest.epoch && current.head()? != tip.head()? {
            return Err(EnrollmentError::Authentication);
        }
        Ok(true)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub group: [u8; 32],
    pub title: String,
    pub owner: Member,
    pub local: Member,
    pub secret: Vec<u8>,
    pub namespace: [u8; 16],
    pub incarnation: [u8; 24],
    pub created: u64,
}
impl Drop for Entry {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.secret.zeroize();
    }
}
impl Entry {
    pub fn identity(&self) -> Result<LocalIdentity, EnrollmentError> {
        LocalIdentity::from_parts(self.local.binding(), self.secret.clone().into())
            .map_err(|_| EnrollmentError::Corrupt)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Offer {
    pub id: [u8; 32],
    pub group: [u8; 32],
    pub title: String,
    pub owner: Member,
    pub recipient: [u8; 16],
    pub created: u64,
    pub expires: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Invite {
    pub offer: Offer,
    pub state: InvitationState,
    pub acknowledged: bool,
    #[serde(default)]
    pub terminal_at: Option<u64>,
    /// Bind a deferred join to its exact member and KeyPackage across restarts.
    #[serde(default)]
    pub deferred_join: Option<[u8; 32]>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub id: [u8; 32],
    pub fingerprint: [u8; 32],
    pub result: [u8; 32],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub version: u8,
    pub local: [u8; 16],
    pub entries: Vec<Entry>,
    pub invites: Vec<Invite>,
    pub requests: Vec<Request>,
    #[serde(default)]
    pub request_retired_before: u64,
    #[serde(default)]
    pub legacy_upgrade_at: Option<u64>,
    #[serde(default)]
    pub invitation_time_floor: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Frame {
    Offer(Offer),
    Join {
        invitation: [u8; 32],
        member: Member,
        #[serde(with = "wire_bytes")]
        package: Vec<u8>,
    },
    Joined {
        invitation: [u8; 32],
        head: [u8; 32],
    },
    Decline {
        invitation: [u8; 32],
    },
    Sync {
        group: [u8; 32],
        head: [u8; 32],
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Reply {
    Ack,
    Expired,
    AwaitingDelivery,
    Welcome { added: Added, tip: Box<Signed> },
    Control(Option<Control>),
}

pub(super) fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, EnrollmentError> {
    serde_json::to_vec(value).map_err(|_| EnrollmentError::Corrupt)
}
pub(super) fn frame<T: Serialize>(value: &T) -> Result<Vec<u8>, EnrollmentError> {
    let bytes = encode(value)?;
    if bytes.len() > MAX_FRAME_BYTES {
        Err(EnrollmentError::Capacity)
    } else {
        Ok(bytes)
    }
}
pub(super) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, EnrollmentError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(EnrollmentError::Capacity);
    }
    serde_json::from_slice(bytes).map_err(|_| EnrollmentError::Invalid)
}
pub(super) fn random<const N: usize>() -> Result<[u8; N], EnrollmentError> {
    let mut bytes = [0; N];
    getrandom::getrandom(&mut bytes).map_err(|_| EnrollmentError::Entropy)?;
    Ok(bytes)
}
pub(super) fn work_id(domain: &[u8], id: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"prns enrollment work v1");
    h.update(domain);
    h.update(id);
    h.finalize().into()
}
pub(super) fn require_time(offer: &Offer, now: u64) -> Result<(), EnrollmentError> {
    prns_groups::enrollment::validate_invitation_time(
        Timestamp(offer.created),
        Timestamp(offer.expires),
        Timestamp(now),
        Timestamp(now),
        INVITATION_LIFETIME_SECS,
    )
    .map_err(|_| EnrollmentError::Expired)
}

// Only public signatures and opaque MLS messages use this codec. Keep 64 KiB
// MLS payloads within the 128 KiB transport frame cap without numeric JSON bloat.
mod wire_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};
    const MAX_BYTES: usize = 65_536;
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        if bytes.len() > MAX_BYTES {
            return Err(serde::ser::Error::custom(
                "enrollment byte field exceeds bound",
            ));
        }
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom(
                "enrollment byte field exceeds bound",
            ));
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| serde::de::Error::custom("invalid enrollment byte field"))?;
        if bytes.len() > MAX_BYTES {
            return Err(serde::de::Error::custom(
                "enrollment byte field exceeds bound",
            ));
        }
        Ok(bytes)
    }
}
