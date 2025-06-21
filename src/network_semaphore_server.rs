use std::{net::SocketAddr, sync::{atomic::AtomicBool, Arc}};
use bytes::Bytes;
use dashmap::DashSet;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_stream::StreamExt;
use futures::sink::SinkExt;
use tokio_util::bytes;
use uuid::Uuid;
use crate::storage::{get_data, save_data};

use crate::packet::{RequestPacket, ResponsePacket};

const HOLDERS_FILE: &str = "holders.json";

struct Shared {
  holders: DashSet<Uuid>,
  finish_notify: Notify,
  finished: AtomicBool,
  cancel: Notify,
  shutdown_delay: std::time::Duration,
}

pub struct NetworkSemaphoreServer {
  address: SocketAddr,
  initial_timeout: std::time::Duration,
  shared: Arc<Shared>,
}

impl NetworkSemaphoreServer {
  pub async fn new(
    address: SocketAddr,
    initial_timeout: std::time::Duration,
    shutdown_delay: std::time::Duration,
  ) -> Self {
    Self {
      address,
      initial_timeout,
      shared: Arc::new(Shared {
        holders: get_data(HOLDERS_FILE).await
          .unwrap_or_else(|| {
            println!("Failed to read data from file");
            DashSet::new()
          }),
        finish_notify: Notify::new(),
        finished: AtomicBool::new(false),
        cancel: Notify::new(),
        shutdown_delay,
      }),
    }
  }

  async fn shutdown(shared: Arc<Shared>) {
    shared.finished.store(true, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    shared.finish_notify.notify_waiters();
  }
  
  async fn delayed_shutdown(delay: std::time::Duration, shared: Arc<Shared>) {
    if delay.is_zero() {
      Self::shutdown(shared).await;
    } else {
      tokio::spawn(async move {
        println!("Waiting for {:?} seconds before shutdown", delay);
        tokio::select! {
          _ = shared.cancel.notified() => {
            println!("Shutdown cancelled");
          }
          _ = tokio::time::sleep(delay) => {
            if shared.holders.is_empty() {
              Self::shutdown(shared).await;
            } else {
              println!("Shutdown cancelled, holders still present");
            }
          },
        }
      });
    }
  }

  async fn handle_message(
    req: RequestPacket,
    address: SocketAddr,
    shared: Arc<Shared>,
  ) -> Result<ResponsePacket, String> {
    match req {
      RequestPacket::Acquire => {
        shared.cancel.notify_waiters();
        let uuid = {
          let mut uuid = Uuid::new_v4();
          while !shared.holders.insert(uuid) {
            uuid = Uuid::new_v4();
          }
          uuid
        };
        println!("Acquired UUID: {}, by {}", uuid, address.ip());
        save_data(HOLDERS_FILE, &shared.holders).await
          .map_err(|e| format!("Failed to save data: {}", e))?;
        Ok(ResponsePacket::Success(uuid))
      }
      RequestPacket::Release(uuid) => {
        if shared.holders.remove(&uuid).is_some() {
          println!("Released UUID: {}, by {}", uuid, address.ip());
          save_data(HOLDERS_FILE, &shared.holders).await
            .map_err(|e| format!("Failed to save data: {}", e))?;
          if shared.holders.is_empty() {
            Self::delayed_shutdown(shared.shutdown_delay, shared).await;
          }
          Ok(ResponsePacket::Success(uuid))
        } else {
          println!("Failed to release UUID: {} by: {}", uuid, address.ip());
          Ok(ResponsePacket::Failure)
        }
      }
      RequestPacket::IsAcquired(uuid) => {
        if shared.holders.contains(&uuid) {
          println!("UUID is acquired: {}, by {}", uuid, address.ip());
          Ok(ResponsePacket::Success(uuid))
        } else {
          println!("UUID is not acquired: {}, by {}", uuid, address.ip());
          Ok(ResponsePacket::Failure)
        }
      }
    }
    
  }

  async fn connection_handler(stream: TcpStream, address: SocketAddr, shared: Arc<Shared>) {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio_util::codec::FramedRead::new(
      reader,
      tokio_util::codec::LengthDelimitedCodec::new()
    );
    let mut writer = tokio_util::codec::FramedWrite::new(
      writer,
      tokio_util::codec::LengthDelimitedCodec::new()
    );
    while let Some(Ok(packet)) = reader.next().await {
      let packet = bincode::decode_from_slice::<RequestPacket, _>(
        &packet,
        bincode::config::standard()
      );
      match packet {
        Ok((req, _)) => {
          println!("Received packet from {}: {:?}", address, req);
          match Self::handle_message(req, address, shared.clone()).await {
            Ok(response) => {
              let encoded_response = bincode::encode_to_vec(
                response,
                bincode::config::standard()
              )
                .expect("Failed to encode response");
              if let Err(err) = writer.send(Bytes::from(encoded_response)).await {
                println!("Failed to send response to {}: {}", address, err);
              }
              if let Err(err) = writer.flush().await {
                println!("Failed to send response to {}: {}", address, err);
              }
            }
            Err(err) => println!("Error handling message from {}: {}", address, err),
          }
        }
        Err(e) => {
          println!("Failed to decode packet from {}: {}", address, e);
          continue;
        }
      }
    }
  }

  async fn connection_accepter(listener: TcpListener, shared: Arc<Shared>) {
    loop {
      let connection = listener.accept().await;
      match connection {
        Ok((stream, address)) => {
          if shared.finished.load(std::sync::atomic::Ordering::Relaxed) {
            break;
          }
          let shared = shared.clone();
          tokio::spawn(async move {
            println!("Accepted connection from {}", address.ip());
            tokio::select! {
              _ = Self::connection_handler(stream, address, shared.clone()) => (),
              _ = shared.finish_notify.notified() => (),
            };
          });
        }
        Err(_) => println!("Failed to accept connection"),
      }
    }
  }
  
  pub async fn start(&mut self) {
    let listener = TcpListener::bind(self.address).await
      .expect("msg: Failed to bind TCP listener");
    if !self.initial_timeout.is_zero() {
      Self::delayed_shutdown(self.initial_timeout, self.shared.clone()).await;
    }
    tokio::select! {
      _ = Self::connection_accepter(listener, self.shared.clone()) => (),
      _ = self.shared.finish_notify.notified() => (),
    }
  }
}
