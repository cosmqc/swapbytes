use std::error::Error;
use libp2p::{
    mdns,
    gossipsub,
    swarm::{ Swarm, SwarmEvent, NetworkBehaviour },
    kad::{ self, store::MemoryStore, QueryResult },
    identity::Keypair,
};

use crate::utils::{ PeerInfo, ChatState };
use crate::events::kad::QueryId;

#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
}

#[derive(NetworkBehaviour)]
pub struct SwapBytesBehaviour {
    pub chat: ChatBehaviour,
    pub kademlia: kad::Behaviour<MemoryStore>
}

pub fn get_swapbytes_behaviour(key: &Keypair) -> Result<SwapBytesBehaviour, Box<dyn Error>>{
    let chat_behaviour = ChatBehaviour {
        mdns: mdns::tokio::Behaviour::new(
            mdns::Config::default(), 
            key.public().to_peer_id()
        )?,
        gossipsub: gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()), 
            gossipsub::Config::default(),
        )?
    };

    Ok(SwapBytesBehaviour {
        chat: chat_behaviour,
        kademlia: kad::Behaviour::new(
            key.public().to_peer_id(),
            MemoryStore::new(key.public().to_peer_id()),
        )
    })
}

pub fn handle_event(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    event: SwarmEvent<SwapBytesBehaviourEvent>,
    chat_state: &mut ChatState
) {
    match event {
        // Your node has connected to the network
        SwarmEvent::NewListenAddr { address, .. } => println!("Your node is listening on {address}"),
        
        // Gossipsub and MDNS (peer discovery and chat)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Chat(event)) => { 
            handle_chat_event(swarm, event, chat_state);
        }

        // Kad events (nickname updates)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Kademlia(kad::Event::OutboundQueryProgressed { 
            id,
            result,
            ..
        })) => handle_kad_event(id, swarm, result, chat_state),

        // Default, do nothing
        _ => {}
    }
}

fn handle_chat_event(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    event: ChatBehaviourEvent,
    chat_state: &mut ChatState
) {
    match event {
        // When a new peer is discovered
        ChatBehaviourEvent::Mdns(mdns::Event::Discovered(list)) => {
            for (peer_id, multiaddr) in list {
                println!("mDNS discovered a new peer: {peer_id}, listening on {multiaddr}");
                swarm.behaviour_mut().chat.gossipsub.add_explicit_peer(&peer_id);
                swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
            }
        },

        // When a peer has left the network
        ChatBehaviourEvent::Mdns(mdns::Event::Expired(list)) => {
            for (peer_id, multiaddr) in list {
                println!("mDNS peer has expired: {peer_id}, listening on {multiaddr}");
                swarm.behaviour_mut().chat.gossipsub.remove_explicit_peer(&peer_id);
            }
        },

        // Message received
        ChatBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source: peer_id,
            message_id: _id,
            message,
        }) => {
            let key = kad::RecordKey::new(&peer_id.to_string());
            let query_id = swarm.behaviour_mut().kademlia.get_record(key);

            // add message to queue, means we can show it when we have all data
            chat_state.pending_messages.insert(query_id, (peer_id.clone(), message.data.clone()));
        },

        _ => {}
    }
}

fn handle_kad_event(id: QueryId, _swarm: &mut Swarm<SwapBytesBehaviour>, result: QueryResult, chat_state: &mut ChatState) {
    match result {
        kad::QueryResult::GetRecord(Ok(
            kad::GetRecordOk::FoundRecord(peer_record)
        )) => {
            if let Some((peer_id, msg)) = chat_state.pending_messages.remove(&id) {
                match serde_cbor::from_slice::<PeerInfo>(&peer_record.record.value) {
                    Ok(peer) => {
                        println!("{}: {}",
                            peer.nickname,
                            String::from_utf8_lossy(&msg)
                        );
                    }
                    Err(_) => {
                        println!("Peer {peer_id}: {}", String::from_utf8_lossy(&msg));
                    }
                }
            }
        }

        _ => {}
    }
}