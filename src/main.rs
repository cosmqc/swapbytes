mod events;
mod files;
mod input;
mod utils;

use files::LocalFileStore;
use futures::StreamExt;
use libp2p::{kad::Mode, noise, tcp, yamux, Multiaddr};
use std::{error::Error, time::Duration};
use tokio::io::{self, AsyncBufReadExt};

use crate::events::get_swapbytes_behaviour;
use crate::utils::ChatState;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(name = "swapbytes")]
struct Cli {
    #[arg(long)]
    port: Option<String>,

    #[arg(long)]
    peer: Option<Multiaddr>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    // Initialize swarm
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|key| {
            get_swapbytes_behaviour(key).expect("Failed to build SwapBytesBehaviour")
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Create an input for the user and ask them for their nickname
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let nickname = utils::prompt_for_nickname(&mut stdin, &mut swarm).await;

    // Initialize local state trackers
    let mut chat_state = ChatState::new(nickname);
    let mut file_store = LocalFileStore::new();

    // Setup GossipSub
    swarm
        .behaviour_mut()
        .chat
        .gossipsub
        .subscribe(&chat_state.current_topic.clone())?;
    swarm.behaviour_mut().kademlia.set_mode(Some(Mode::Server));

    let listen_port = cli.port.unwrap_or("0".to_string());
    let multiaddr = format!("/ip4/0.0.0.0/tcp/{listen_port}");
    swarm.listen_on(multiaddr.parse()?)?;

    println!("Enter chat messages one line at a time:");

    // Main loop
    loop {
        tokio::select! {
            // If the user sends a command/message, handle it
            Ok(Some(line)) = stdin.next_line()
                => input::handle_input_line(
                    &mut swarm,
                    line,
                    chat_state.current_topic.clone(),
                    &mut stdin,
                    &mut file_store,
                    &mut chat_state
                ).await?,

            // Catch events and handle them
            event = swarm.select_next_some() => events::handle_event(&mut swarm, event, &mut chat_state, &mut file_store).await
        }
    }
}
