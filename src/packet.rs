use bincode::{Decode, Encode};
use uuid::Uuid;

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequestPacket {
  Acquire,
  Release(#[bincode(with_serde)] Uuid),
  IsAcquired(#[bincode(with_serde)] Uuid),
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResponsePacket {
  Success(#[bincode(with_serde)] Uuid),
  Failure,
}
