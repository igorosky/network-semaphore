use std::{net::{IpAddr, SocketAddr}, sync::{atomic::AtomicBool, Arc}};
use dashmap::DashSet;
use tokio::{io::AsyncReadExt, net::{tcp::OwnedWriteHalf, TcpListener, TcpStream}};
use tokio::sync::Notify;
use tokio::io::AsyncWriteExt;

/*
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
  cancel: Notify,
  shutdown_delay: u64,
}

pub struct NetworkSemaphoreServer {
  address: String,
  port: u16,
  initial_timeout: u64,
  shared: Arc<Shared>,
}

impl NetworkSemaphoreServer {
  pub fn new(address: String, port: u16, initial_timeout: u64, shutdown_delay: u64) -> Self {
    Self {
      address,
      port,
      initial_timeout,
      shared: Arc::new(Shared {
        holders: DashSet::new(),
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
  
  async fn delayed_shutdown(delay: u64, shared: Arc<Shared>) {
    if delay == 0 {
      Self::shutdown(shared).await;
    } else {
      tokio::spawn(async move {
        println!("Waiting for {} seconds before shutdown", delay);
        tokio::select! {
          _ = shared.cancel.notified() => {
            println!("Shutdown cancelled");
          }
          _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {
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

  async fn handle_message(buf: &[u8], writer: &mut OwnedWriteHalf, address: SocketAddr, shared: Arc<Shared>) {
    let mut acquire = false;
    let mut release = false;
    let send = if buf.len() != 1 {
      println!("Invalid packet from {}", address);
      writer.write_all(&[2]).await
    } else {
      match buf[0] {
        0 => {
          println!("Received acquire packet {}", address);
          acquire = shared.holders.insert(address.ip());
          writer.write_all(&[!acquire as u8]).await
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
    if send.is_err() {
      println!("Failed to send response to {}", address);
      return;
    }
    if acquire {
      shared.cancel.notify_waiters();
    }
    if release && shared.holders.is_empty() {
      println!("No more holders, shutting down");
      Self::delayed_shutdown(shared.shutdown_delay, shared).await;
    }
  }

  async fn connection_handler(stream: TcpStream, address: SocketAddr, shared: Arc<Shared>) {
    let mut buf = [0; 1];
    let (mut reader, mut writer) = stream.into_split();
    loop {
      match reader.read_exact(&mut buf).await {
        Err(err) if err.kind() == tokio::io::ErrorKind::UnexpectedEof => {
          println!("Connection closed by {}", address.ip());
          break;
        }
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
    let listener = TcpListener::bind(format!("{}:{}", self.address, self.port)).await
      .expect("msg: Failed to bind TCP listener");
    if self.initial_timeout != 0 {
      Self::delayed_shutdown(self.initial_timeout, self.shared.clone()).await;
    }
    tokio::select! {
      _ = Self::connection_accepter(listener, self.shared.clone()) => (),
      _ = self.shared.finish_notify.notified() => (),
    }
  }
}
