use std::{io::{Error, ErrorKind}, net::{IpAddr, SocketAddr}, sync::Arc};
use bytes::Bytes;
use dashmap::{DashMap, DashSet};
use tokio::net::{tcp::{OwnedReadHalf, OwnedWriteHalf}, TcpStream};
use uuid::Uuid;
use tokio_stream::StreamExt;
use futures::sink::SinkExt;
use tokio_util::{bytes, codec::{FramedRead, FramedWrite}};
use crate::packet::{RequestPacket, ResponsePacket};
use crate::storage::{get_data, save_data};

#[derive(Debug, Clone)]
pub struct NetworkSemaphoreClient {
  locks: Arc<DashMap<SocketAddr, (Uuid, DashSet<Uuid>)>>,
  file: Arc<str>,
}

impl NetworkSemaphoreClient {
  pub async fn new(file: String) -> Self {
    let locks = get_data(&file)
      .await
      .unwrap_or(DashMap::new());
    Self {
      locks: Arc::from(locks),
      file: file.into(),
    }
  }

  pub fn server(
    &self,
    addr: SocketAddr,
  ) -> Server {
    Server::new(addr, self.clone())
  }

  pub async fn list_locks(&self) -> DashMap<SocketAddr, DashSet<Uuid>> {
    DashMap::from_iter(self.locks.iter().map(|v|
      (*v.key(), v.value().1.clone())
    ))
  }

  pub async fn verify_locks(&mut self) -> DashMap<IpAddr, std::io::Result<DashSet<Uuid>>> {
    let results = DashMap::new();
    for addr in self.locks.clone().iter()
      .map(|v| *v.key()) {
      if !self.server(addr).verify_lock().await
        .map(|v| v.is_empty())
        .unwrap_or(true) {
        self.locks.remove(&addr);
      }
    }
    results
  }
}

pub struct Server {
  addr: SocketAddr,
  client: NetworkSemaphoreClient,
  connection: Option<NetworkSemaphoreClientConnection>,
}

impl Server {
  fn new(addr: SocketAddr, client: NetworkSemaphoreClient) -> Self {
    Self { addr, client, connection: None }
  }

  async fn get_connection(
    connection: &mut Option<NetworkSemaphoreClientConnection>,
    addr: SocketAddr
  ) -> std::io::Result<&mut NetworkSemaphoreClientConnection> {
    if connection.is_none() {
      *connection = Some(NetworkSemaphoreClientConnection::connect(addr).await?);
    }
    Ok(unsafe { connection.as_mut().unwrap_unchecked() })
  }

  pub async fn verify_lock(&mut self) -> std::io::Result<DashSet<Uuid>> {
    if let Some((uuid, uuids)) = self.client.locks.get(&self.addr)
      .map(|v| v.value().clone()) {
      Self::get_connection(&mut self.connection, self.addr).await?.is_acquired(uuid).await?;
      Ok(uuids)
    } else {
      Ok(DashSet::new())
    }
  }

  pub async fn add_lock(&mut self) -> std::io::Result<Uuid> {
    let new_uuid = Uuid::new_v4();
    if !self.client.locks.contains_key(&self.addr) {
      let uuid = Self::get_connection(&mut self.connection, self.addr).await?.acquire().await?;
      let set = DashSet::new();
      set.insert(new_uuid);
      self.client.locks.insert(self.addr, (uuid, set));
    } else {
      unsafe { self.client.locks.get(&self.addr).unwrap_unchecked() }.value().1.insert(new_uuid);
    }
    save_data(&self.client.file, &*self.client.locks).await?;
    Ok(new_uuid)
  }

  pub async fn remove_lock(&mut self, uuid: &Uuid) -> std::io::Result<bool> {
    match self.client.locks.get(&self.addr) {
      Some(v) => {
        let (k_uuid, uuids) = v.value();
        if uuids.remove(uuid).is_some() {
          save_data(&self.client.file, &*self.client.locks).await?;
          if uuids.is_empty() {
            Self::get_connection(&mut self.connection, self.addr).await?.release(*k_uuid).await
          } else {
            Ok(false)
          }
        } else {
          Err(Error::other(format!("Value not contained {}", uuid)))
        }
      }
      None => Err(Error::other("No semaphore contained")),
    }
  }

  pub async fn is_server_locked(&mut self) -> std::io::Result<bool> {
    let ans = match self.client.locks.get(&self.addr) {
      Some(v) =>
        Self::get_connection(&mut self.connection, self.addr).await?.is_acquired(v.value().0).await,
      None => Ok(false)
    };
    if ans.as_ref().is_ok_and(|v| !v) {
      self.client.locks.remove(&self.addr);
    }
    ans
  }

  pub async fn is_lock_valid(&mut self, uuid: &Uuid) -> std::io::Result<bool> {
    if self.is_server_locked().await? {
      if let Some(v) = self.client.locks.get(&self.addr) {
        return Ok(v.value().1.contains(uuid));
      }
    }
    Ok(false)
  }
}

pub struct NetworkSemaphoreClientConnection {
  reader: FramedRead<OwnedReadHalf, tokio_util::codec::LengthDelimitedCodec>,
  writer: FramedWrite<OwnedWriteHalf, tokio_util::codec::LengthDelimitedCodec>,
}

impl NetworkSemaphoreClientConnection {
  async fn connect(addr: SocketAddr) -> tokio::io::Result<Self> {
    let connection = TcpStream::connect(&addr).await?;
    let (reader, writer) = connection.into_split();
    Ok(Self {
      reader: FramedRead::new(
        reader,
        tokio_util::codec::LengthDelimitedCodec::new(),
      ),
      writer: FramedWrite::new(
        writer,
        tokio_util::codec::LengthDelimitedCodec::new(),
      ),
    })
  }

  async fn communicate(
    &mut self,
    request: RequestPacket,
  ) -> std::io::Result<ResponsePacket> {
    let request = bincode::encode_to_vec(
      request,
      bincode::config::standard()
    ).expect("Failed to encode request");
    self.writer.send(Bytes::from(request)).await
      .map_err(|e| Error::other(format!("Failed to send request: {}", e)))?;
    self.writer.flush().await?;

    let response = self.reader.next().await
      .ok_or(Error::new(ErrorKind::UnexpectedEof, "No response received"))?
      .map_err(|e| Error::other(format!("Failed to read response: {}", e)))?;
    
    let (response, _) = bincode::decode_from_slice::<ResponsePacket, _>(
      &response,
      bincode::config::standard()
    ).map_err(|e| Error::new(ErrorKind::InvalidData, format!("Failed to decode response: {}", e)))?;
    
    Ok(response)
  }

  pub async fn acquire(&mut self) -> std::io::Result<Uuid> {
    let request = RequestPacket::Acquire;
    let response = self.communicate(request).await?;
    match response {
      ResponsePacket::Success(uuid) => Ok(uuid),
      ResponsePacket::Failure => Err(Error::other("Failed to acquire semaphore")),
    }
  }

  pub async fn release(&mut self, uuid: Uuid) -> std::io::Result<bool> {
    let request = RequestPacket::Release(uuid);
    let response = self.communicate(request).await?;
    match response {
      ResponsePacket::Success(_) => Ok(true),
      ResponsePacket::Failure => Ok(false),
    }
  }

  pub async fn is_acquired(&mut self, uuid: Uuid) -> std::io::Result<bool> {
    let request = RequestPacket::IsAcquired(uuid);
    let response = self.communicate(request).await?;
    match response {
      ResponsePacket::Success(_) => Ok(true),
      ResponsePacket::Failure => Ok(false),
    }
  }
}
