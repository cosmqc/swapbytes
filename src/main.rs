mod events;
mod files;
mod input;
mod utils;

use files::LocalFileStore;
use futures::StreamExt;
use libp2p::{kad::Mode, noise, rendezvous, tcp, yamux, Multiaddr};
use std::{error::Error, time::Duration};
use tokio::io::{self, AsyncBufReadExt};
use tokio::time::MissedTickBehavior;
use clap::Parser;

use crate::events::get_swapbytes_behaviour;
use crate::utils::ChatState;

#[derive(Parser, Debug)]
#[clap(name = "swapbytes")]
struct Cli {
    #[arg(long)]
    port: Option<String>,

    #[arg(long)]
    rendezvous: Option<String>,
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

    // Rendezvous server schenanigans
    let rendezvous_addr = cli.rendezvous.unwrap_or("127.0.0.1".to_string());
    let rendezvous_point_address = format!("/ip4/{}/tcp/62649", rendezvous_addr)
        .parse::<Multiaddr>()
        .unwrap();

    let external_address = format!("/ip4/{}/tcp/0", rendezvous_addr)
        .parse::<Multiaddr>()
        .unwrap();
    swarm.add_external_address(external_address);
    swarm.dial(rendezvous_point_address.clone()).unwrap();

    let listen_port = cli.port.unwrap_or("0".to_string());
    let multiaddr = format!("/ip4/0.0.0.0/tcp/{listen_port}");
    swarm.listen_on(multiaddr.parse()?)?;

    // Discovery ping goes off every 30 seconds
    let mut discover_tick = tokio::time::interval(Duration::from_secs(30));
    discover_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

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
            event = swarm.select_next_some() => events::handle_event(&mut swarm, event, &mut chat_state, &mut file_store).await,

            // If discovery tick, try to discover new peers
            _ = discover_tick.tick() => {
                swarm.dial(rendezvous_point_address.clone()).unwrap();
                swarm.behaviour_mut().rendezvous.rendezvous.discover(
                    Some(rendezvous::Namespace::new("rendezvous".to_string()).unwrap()),
                    None,
                    None,
                    chat_state.rendezvous
                )
            }
        }
    }
}
