use libp2p::{
    gossipsub,
    identity::Keypair,
    kad::{self, store::MemoryStore, QueryResult},
    mdns,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
};
use std::error::Error;

use crate::{events::kad::QueryId, utils};
use crate::files::FileMetadata;
use crate::utils::{ChatState, NicknameUpdate, PeerInfo};

#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
}

#[derive(NetworkBehaviour)]
pub struct SwapBytesBehaviour {
    pub chat: ChatBehaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
}

/// Setup different sets of behaviour for the app.
/// Splitting them means its easier to fliter them in the event handler
pub fn get_swapbytes_behaviour(key: &Keypair) -> Result<SwapBytesBehaviour, Box<dyn Error>> {
    let chat_behaviour = ChatBehaviour {
        mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?,
        gossipsub: gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()),
            gossipsub::Config::default(),
        )?,
    };

    Ok(SwapBytesBehaviour {
        chat: chat_behaviour,
        kademlia: kad::Behaviour::new(
            key.public().to_peer_id(),
            MemoryStore::new(key.public().to_peer_id()),
        ),
    })
}

/// High level event handler, filters by behaviour type and delegates to lower-level handlers
pub fn handle_event(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    event: SwarmEvent<SwapBytesBehaviourEvent>,
    chat_state: &mut ChatState,
) {
    match event {
        // Your node has connected to the network
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("Your node is listening on {address}")
        }

        // Gossipsub and MDNS (peer discovery and chat)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Chat(event)) => {
            handle_chat_event(swarm, event, chat_state);
        }

        // Kad events (nickname updates)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Kademlia(
            kad::Event::OutboundQueryProgressed { id, result, .. },
        )) => handle_kad_event(id, swarm, result, chat_state),

        // Default, do nothing
        // default => println!("{default:?}")
        _ => {}
    }
}

/// Low-level chat handler. Sorts MDNS events (mostly connection and peers), and catching broadcasts 
fn handle_chat_event(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    event: ChatBehaviourEvent,
    chat_state: &mut ChatState,
) {
    match event {
        // When a new peer is discovered
        ChatBehaviourEvent::Mdns(mdns::Event::Discovered(list)) => {
            for (peer_id, multiaddr) in list {
                println!("mDNS discovered a new peer: {peer_id}, listening on {multiaddr}");

                // Add peer to gossipsub
                swarm
                    .behaviour_mut()
                    .chat
                    .gossipsub
                    .add_explicit_peer(&peer_id);

                // Add peer to kademlia
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, multiaddr);

                swarm.behaviour_mut().kademlia.get_closest_peers(peer_id.clone());

                // Query for nickname
                if !chat_state.nicknames.contains_key(&peer_id.to_string()) {
                    let key = kad::RecordKey::new(&format!("peer::{}", peer_id));
                    swarm.behaviour_mut().kademlia.get_record(key);
                }
            }
        }

        // When a peer has left the network
        ChatBehaviourEvent::Mdns(mdns::Event::Expired(list)) => {
            for (peer_id, multiaddr) in list {
                println!("mDNS peer has expired: {peer_id}, listening on {multiaddr}");
                swarm
                    .behaviour_mut()
                    .chat
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
            }
        }

        // Message received
        ChatBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source: peer_id,
            message_id: _id,
            message,
        }) => {
            // Try to interpret the message as a NicknameUpdate
            if let Ok(update) = serde_cbor::from_slice::<NicknameUpdate>(&message.data) {
                chat_state
                    .nicknames
                    .insert(update.peer_id.clone(), update.nickname);
            } else {
                // If the message is empty, ignore the message
                if message.data.is_empty() {
                    return;
                }

                // Otherwise output the message
                match chat_state.nicknames.get(&peer_id.to_string()) {
                    // If we've got the nickname cached, print that immediately
                    Some(nickname) => {
                        println!("{}: {}", nickname, String::from_utf8_lossy(&message.data))
                    }
                    // Otherwise, queue it for later
                    None => {
                        let key = kad::RecordKey::new(&format!("peer::{}", peer_id));
                        let query_id = swarm.behaviour_mut().kademlia.get_record(key);

                        chat_state
                            .pending_messages
                            .insert(query_id, (peer_id.clone(), message.data));
                    }
                }
            }
        }

        // default => println!("{default:?}")
        _ => {}
    }
}

/// Kademlia handler, handles responses for DHT queries requested elsewhere.
fn handle_kad_event(
    id: QueryId,
    swarm: &mut Swarm<SwapBytesBehaviour>,
    result: QueryResult,
    chat_state: &mut ChatState,
) {
    match result {
        // Response from DHT request
        kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FoundRecord(peer_record))) => {
            // Deduplicate responses if multiple peers answer, only keep the ones with a peer id
            if peer_record.peer.is_none() || !chat_state.handled_keys.insert(id.to_string()) {
                return;
            }

            // Match on the custom response type (peer, file, file_index, etc)
            let record_key = String::from_utf8_lossy(peer_record.record.key.as_ref());
            match record_key.as_ref() {
                // Deferred messages (replies to nickname queries)
                key if key.starts_with("peer::") => {
                    if let Some((peer_id, msg)) = chat_state.pending_messages.remove(&id) {
                        match serde_cbor::from_slice::<PeerInfo>(&peer_record.record.value) {
                            Ok(peer) => {
                                // Regardless of if the message exists, update the nickname
                                chat_state.nicknames.insert(peer.peerid, peer.nickname.clone());
                                
                                if msg.is_empty() {
                                    return;
                                }

                                // If not empty, output it like a normal message
                                println!("{}: {}", peer.nickname, String::from_utf8_lossy(&msg));
                            }
                            Err(_) => {
                                println!("Peer {peer_id}: {}", String::from_utf8_lossy(&msg));
                            }
                        }
                    }
                }

                // File metadata responses
                key if key.starts_with("file::") => {
                    match serde_cbor::from_slice::<FileMetadata>(&peer_record.record.value) {
                        Ok(metadata) => {
                            println!(
                                "\t{} ({} bytes) - {}",
                                metadata.filename,
                                metadata.size,
                                metadata
                                    .description
                                    .unwrap_or_else(|| "No description".to_string())
                            )
                        }
                        Err(e) => {
                            println!("Error deserializing file metadata: {e}");
                        }
                    }
                }

                // Response from a peer saying what files they have.
                key if key.starts_with("file_index::") => {
                    // Get sender name
                    let sender = match peer_record.peer {
                        Some(peerid) => {
                            let peerid_str = peerid.to_string();
                            chat_state
                                .nicknames
                                .get(&peerid_str)
                                .cloned()
                                .unwrap_or(peerid_str)
                        }
                        None => "Someone".to_string(),
                    };

                    // Print a message, then send a request for the metadata of each file listed
                    match serde_cbor::from_slice::<Vec<String>>(&peer_record.record.value) {
                        Ok(hashes) => {
                            let file_count = hashes.len();
                            println!(
                                "{} has uploaded {} file{}:",
                                sender,
                                file_count,
                                if file_count == 1 { "" } else { "s" }
                            );

                            hashes.iter().for_each(|hash| {
                                let key = kad::RecordKey::new(&format!("file::{}", hash));
                                swarm.behaviour_mut().kademlia.get_record(key);
                            });
                        }
                        Err(e) => {
                            println!("Failed to parse file index for {key}: {e}");
                        }
                    }
                }

                // If the record type isn't defined 
                _ => println!("Unexpected record type: {}", record_key)
            }
        }

        // Once bootstrapping is complete, broadcast nickname
        kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { num_remaining, .. })) => {
            if num_remaining == 0 {
                utils::set_nickname(swarm, None, chat_state);
            }
        }

        // default => println!("{default:?}")
        _ => {}
    }
}
