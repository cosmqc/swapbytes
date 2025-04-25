mod input;
mod utils;
mod events;

use std::{ error::Error, time::Duration };
use libp2p::{ kad::Mode, tcp, noise, yamux, gossipsub };
use tokio::io::{self, AsyncBufReadExt};
use futures::StreamExt;

use crate::events::get_swapbytes_behaviour;
use crate::utils::ChatState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
    .with_tokio()
    .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
    .with_quic()
    .with_behaviour(|key| {
        get_swapbytes_behaviour(key).expect("Failed to build SwapBytesBehaviour")
    })?
    .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
    .build();

    // create a new topic called "chat"
    let topic = gossipsub::IdentTopic::new("chat");

    // use the gossipsub behaviour to subscribe to "chat"
    swarm.behaviour_mut().chat.gossipsub.subscribe(&topic.clone())?;
    swarm.behaviour_mut().kademlia.set_mode(Some(Mode::Server));

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    let mut chat_state = ChatState::new();

    // create non-blocking standard input
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    utils::prompt_for_nickname(&mut stdin, &mut swarm).await;
    println!("Enter chat messages one line at a time:");

    loop {
        tokio::select! {
            // If the user sends a command/message, handle it 
            Ok(Some(line)) = stdin.next_line() => input::handle_input_line(&mut swarm, line, topic.clone(), &mut stdin).await?,
            
            // Catch events and handle them
            event = swarm.select_next_some() => events::handle_event(&mut swarm, event, &mut chat_state)
        }
    }

}