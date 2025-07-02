use bitcode::{Decode, Encode};

use crate::{player_cosmetics::PlayerPngDesc, GameSettings};

use super::{omni::OmniPeerId, world::WorldNetMessage};

use crate::net::world::world_model::ChunkCoord;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    Peer(OmniPeerId),
    Host,
    Broadcast,
}

pub(crate) struct MessageRequest<T> {
    pub(crate) reliability: tangled::Reliability,
    pub(crate) dst: Destination,
    pub(crate) msg: T,
}

#[derive(Debug, Decode, Encode)]
pub(crate) enum NetMsg {
    Welcome,
    Disconnect { id: OmniPeerId },
    RequestMods,
    Mods { mods: Vec<String> },
    EndRun,
    Kick,
    PeerDisconnected { id: OmniPeerId },
    StartGame { settings: GameSettings },
    ModRaw { data: Vec<u8> },
    ModCompressed { data: Vec<u8> },
    WorldMessage(WorldNetMessage),
    PlayerColor(PlayerPngDesc, bool, Option<OmniPeerId>, String),
    ChecksumRequest {
        coords: Vec<ChunkCoord>,
    },
    ChecksumResponse {
        checks: Vec<(ChunkCoord, u64)>,
    },
}

impl From<MessageRequest<WorldNetMessage>> for MessageRequest<NetMsg> {
    fn from(value: MessageRequest<WorldNetMessage>) -> Self {
        Self {
            msg: NetMsg::WorldMessage(value.msg),
            reliability: value.reliability,
            dst: value.dst,
        }
    }
}
