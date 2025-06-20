mod network_semaphore_server;
mod network_semaphore_client;

use clap::Parser;

use network_semaphore_server::NetworkSemaphoreServer;
use network_semaphore_client::NetworkSemaphoreClient;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// Run as server
  #[arg(short, long, default_value_t = false)]
  server: bool,
  /// Address for listening
  #[arg(short, long)]
  address: String,
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
}

async fn run_server(args: Args) {
  println!("Starting server");
  
  let address = args.address;
  let port = args.port;
  let initial_timeout = args.initial_timeout;
  
  println!("Listening on {}:{}", address, port);
  let mut server = NetworkSemaphoreServer::new(address, port, initial_timeout, args.shutdown_delay);
  server.start().await;
  println!("Server shutdown initiated");
  system_shutdown::shutdown().expect("Failed to shutdown the OS");
}

fn run_client(args: Args) {
  let mut client = NetworkSemaphoreClient::connect(
    format!("{}:{}", args.address, args.port).parse().expect("Invalid address"),
    std::time::Duration::from_secs(args.initial_timeout),
  ).expect("Failed to connect to server");

  if args.acquire {
    match client.acquire() {
      Ok(acquired) => println!("Semaphore acquired: {}", acquired),
      Err(e) => eprintln!("Error acquiring semaphore: {}", e),
    }
  }
  if args.release {
    match client.release() {
      Ok(released) => println!("Semaphore released: {}", released),
      Err(e) => eprintln!("Error releasing semaphore: {}", e),
    }
  }
  if args.is_acquired {
    match client.is_acquired() {
      Ok(acquired) => println!("Semaphore is acquired: {}", acquired),
      Err(e) => eprintln!("Error checking semaphore status: {}", e),
    }
  }  
}

fn main() {
  let args = Args::parse();
  if args.server {
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .expect("Failed to create Tokio runtime")
      .block_on(async move {
          run_server(args).await;
        }
      );
  } else {
    run_client(args);
  }
}
