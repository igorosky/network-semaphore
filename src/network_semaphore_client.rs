use std::{io::{Read, Write}, net::{SocketAddr, TcpStream}, time::Duration};

pub struct NetworkSemaphoreClient {
  connection: TcpStream,
}

impl NetworkSemaphoreClient {
  pub fn connect(addr: SocketAddr, timeout: Duration) -> std::io::Result<Self> {
    TcpStream::connect_timeout(&addr, timeout)
      .map(|v| Self { connection: v })
  }

  pub fn acquire(&mut self) -> std::io::Result<bool> {
    let mut buf = [0; 1];
    self.connection.write_all(&[0])
      .and_then(|_| self.connection.read_exact(&mut buf))
      .map(|_| buf[0] == 0)
  }

  pub fn release(&mut self) -> std::io::Result<bool> {
    let mut buf = [0; 1];
    self.connection.write_all(&[1])
      .and_then(|_| self.connection.read_exact(&mut buf))
      .map(|_| buf[0] == 0)
  }

  pub fn is_acquired(&mut self) -> std::io::Result<bool> {
    let mut buf = [0; 1];
    self.connection.write_all(&[2])
      .and_then(|_| self.connection.read_exact(&mut buf))
      .map(|_| buf[0] == 0)
  }
}
