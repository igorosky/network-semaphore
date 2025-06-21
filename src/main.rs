mod network_semaphore_server;
mod network_semaphore_client;
mod storage;
mod packet;

use std::net::{IpAddr, SocketAddr};

use clap::Parser;
use uuid::Uuid;

use network_semaphore_server::NetworkSemaphoreServer;

use crate::network_semaphore_client::NetworkSemaphoreClient;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// Clear all files
  #[arg(short, long, default_value_t = false)]
  clear: bool,
  /// Run as server
  #[arg(short, long, default_value_t = false)]
  server: bool,
  /// Address for listening
  #[arg(short, long)]
  address: Option<IpAddr>,
  /// Port for listening
  #[arg(short, long, default_value_t = 9000)]
  port: u16,
  /// Listening timeout for the first holder in seconds
  #[arg(short('t'), long, default_value_t = 600)]
  initial_timeout: u64,
  /// Shutdown delay in seconds
  #[arg(short('d'), long, default_value_t = 300)]
  shutdown_delay: u64,
  /// Acquire the semaphore
  #[arg(short('q'), long, default_value_t = false)]
  acquire: bool,
  /// Release the semaphore
  #[arg(short, long, default_value_t = false)]
  release: bool,
  /// Check if the semaphore is acquired
  #[arg(short, long, default_value_t = false)]
  is_acquired: bool,
  /// Uuid of the semaphore holder (required for release and is_acquired)
  #[arg(short, long)]
  uuid: Option<Uuid>,
  /// List of holders
  #[arg(short, long, default_value_t = false)]
  list_holders: bool,
  /// Verify locks on all addresses
  #[arg(short, long, default_value_t = false)]
  verify_locks: bool,
}

async fn run_server(args: Args) -> ! {
  println!("Starting server");
  
  let address = args.address
    .expect("Address is required for server mode");
  let port = args.port;
  let initial_timeout = args.initial_timeout;
  
  println!("Listening on {}:{}", address, port);
  let mut server = NetworkSemaphoreServer::new(
    SocketAddr::new(address, port),
    std::time::Duration::from_secs(initial_timeout),
    std::time::Duration::from_secs(args.shutdown_delay)
  ).await;
  server.start().await;
  println!("Server shutdown initiated");
  system_shutdown::shutdown().expect("Failed to shutdown the OS");
  loop {
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
  }
}

async fn run_client(args: Args) {
  let mut client = NetworkSemaphoreClient::new("client.json".to_string()).await;
  if let Some(address) = args.address {
    let mut server = client.server(
      SocketAddr::new(address, args.port),
    );
  
    if args.acquire {
      match server.add_lock().await {
        Ok(acquired) => {
          println!("{}", acquired);
        },
        Err(e) => {
          eprintln!("Error acquiring semaphore: {}", e);
          std::process::exit(1);
        },
      }
    }
    if args.release {
      if let Some(uuid) = args.uuid {
        match server.remove_lock(&uuid).await {
          Ok(true) => println!("Semaphore released"),
          Ok(false) => std::process::exit(1),
          Err(e) => {
            eprintln!("Error releasing semaphore: {}", e);
            std::process::exit(1);
          },
        }
      } else {
        eprintln!("UUID is required to release the semaphore");
      }
    }
    if args.is_acquired {
      if let Some(uuid) = args.uuid {
        match server.is_lock_valid(&uuid).await {
          Ok(true) => println!("Lock is valid"),
          Ok(false) => println!("Lock is invalid"),
          Err(err) => {
            eprintln!("Error checking semaphore: {}", err);
            std::process::exit(1);
          },
        }
      } else {
        match server.is_server_locked().await {
          Ok(acquired) => println!("Semaphore is acquired: {}", acquired),
          Err(e) => {
            eprintln!("Error checking semaphore: {}", e);
            std::process::exit(1);
          },
        }
      }
    }
  }
  if args.verify_locks {
    let _ = client.verify_locks().await;
  }
  if args.list_holders {
    let locks = client.list_locks().await;
    for x in locks.iter() {
      let addr = x.key();
      println!("{}:", addr);
      for uuid in x.iter() {
        println!("  {}", *uuid);
      }
    }
  }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let args = Args::parse();
  if args.clear {
    crate::storage::clear().await.expect("Failed to clear config dir");
  }
  if args.server {
    run_server(args).await;
  } else {
    run_client(args).await;
  }
}
