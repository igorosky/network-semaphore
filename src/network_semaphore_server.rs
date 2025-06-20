use std::{net::{IpAddr, SocketAddr}, sync::{atomic::AtomicBool, Arc}};
use dashmap::DashSet;
use tokio::{io::AsyncReadExt, net::{tcp::OwnedWriteHalf, TcpListener, TcpStream}};
use tokio::sync::Notify;
use tokio::io::AsyncWriteExt;

/**
 * Request
 * 0 - get
 * 1 - release
 * 2 - do i have
 * 
 * Response
 * 0 - successful/yes
 * 1 - failed/no
 * 2 - invalid package
 */

struct Shared {
  holders: DashSet<IpAddr>,
  finish_notify: Notify,
  finished: AtomicBool,
}

pub struct NetworkSemaphoreServer {
  address: String,
  port: u16,
  initial_timeout: u64,
  shared: Arc<Shared>,
}

impl NetworkSemaphoreServer {
  pub fn new(address: String, port: u16, initial_timeout: u64) -> Self {
    Self {
      address,
      port,
      initial_timeout,
      shared: Arc::new(Shared {
        holders: DashSet::new(),
        finish_notify: Notify::new(),
        finished: AtomicBool::new(false),
      }),
    }
  }

  async fn shutdown(shared: Arc<Shared>) {
    shared.finished.store(true, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    shared.finish_notify.notify_waiters();
  }
  
  fn initial_wait(&self) {
    if self.initial_timeout == 0 {
      return;
    }
    let initial_timeout = self.initial_timeout;
    let shared = self.shared.clone();
    tokio::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_secs(initial_timeout)).await;
      if shared.holders.len() == 0 {
        Self::shutdown(shared).await;
      }
    });
  }

  async fn handle_message(buf: &[u8], writer: &mut OwnedWriteHalf, address: SocketAddr, shared: Arc<Shared>) {
    let mut release = false;
    let send = if buf.len() != 1 {
      println!("Invalid packet from {}", address);
      writer.write_all(&[2]).await
    } else {
      match buf[0] {
        0 => {
          println!("Received acquire packet {}", address);
          writer.write_all(&[!shared.holders.insert(address.ip()) as u8]).await
        }
        1 => {
          println!("Received release packet from {}", address);
          release = shared.holders.remove(&address.ip()).is_some();
          writer.write_all(&[!release as u8]).await
        }
        2 => {
          println!("Received check packet from {}", address);
          writer.write_all(&[!shared.holders.contains(&address.ip()) as u8]).await
        }
        _ => {
          println!("Invalid packet from {}", address);
          writer.write_all(&[2]).await
        }
      }
    };
    if let Err(_) = send {
      println!("Failed to send response to {}", address);
    }
    if release && shared.holders.is_empty() {
      println!("No more holders, shutting down");
      Self::shutdown(shared).await;
    }
  }

  async fn connection_handler(stream: TcpStream, address: SocketAddr, shared: Arc<Shared>) {
    let mut buf = [0; 1];
    let (mut reader, mut writer) = stream.into_split();
    loop {
      match reader.read_exact(&mut buf).await {
        Err(err) => {
          println!("Failed while reading from {} ({})", address.ip(), err.kind());
          break;
        }
        Ok(0) => break,
        Ok(count) => Self::handle_message(&buf[..count], &mut writer, address, shared.clone()).await,
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
    let listener = TcpListener::bind(format!("{}:{}", self.address, self.port)).await
      .expect("msg: Failed to bind TCP listener");
    self.initial_wait();
    tokio::select! {
      _ = Self::connection_accepter(listener, self.shared.clone()) => (),
      _ = self.shared.finish_notify.notified() => (),
    }
  }
}
