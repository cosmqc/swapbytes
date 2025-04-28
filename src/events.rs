use libp2p::request_response;
use libp2p::{
    gossipsub,
    identity::Keypair,
    kad::{self, store::MemoryStore, QueryResult},
    mdns, ping, rendezvous,
    request_response::{Message, ProtocolSupport},
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    PeerId, StreamProtocol,
};
use std::error::Error;
use tokio::time::Duration;

use crate::files::{FileMetadata, FileResponse};
use crate::utils::ChatState;
use crate::{
    events::kad::QueryId,
    files::{save_file_to_filesystem, AcknowledgeResponse, DirectMessage, LocalFileStore},
    input::ChatMessage,
    utils::{NicknameUpdate, TradeRequest},
};

#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
}

#[derive(NetworkBehaviour)]
pub struct RendezvousBehaviour {
    pub rendezvous: rendezvous::client::Behaviour,
    pub ping: ping::Behaviour,
}

#[derive(NetworkBehaviour)]
pub struct SwapBytesBehaviour {
    pub chat: ChatBehaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub file_transfer:
        request_response::cbor::Behaviour<Option<FileResponse>, Option<FileResponse>>,
    pub direct_message: request_response::cbor::Behaviour<DirectMessage, AcknowledgeResponse>,
    pub nickname_update: request_response::cbor::Behaviour<NicknameUpdate, NicknameUpdate>,
    pub trade_request: request_response::cbor::Behaviour<TradeRequest, AcknowledgeResponse>,
    pub rendezvous: RendezvousBehaviour,
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

    let rendezvous_behaviour = RendezvousBehaviour {
        rendezvous: rendezvous::client::Behaviour::new(key.clone()),
        ping: ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(1))),
    };

    Ok(SwapBytesBehaviour {
        chat: chat_behaviour,
        kademlia: kad::Behaviour::new(
            key.public().to_peer_id(),
            MemoryStore::new(key.public().to_peer_id()),
        ),
        file_transfer: request_response::cbor::Behaviour::new(
            [(
                StreamProtocol::new("/file-exchange/1"),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        ),
        direct_message: request_response::cbor::Behaviour::new(
            [(
                StreamProtocol::new("/direct-message/1"),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        ),
        nickname_update: request_response::cbor::Behaviour::new(
            [(
                StreamProtocol::new("/nickname-update/1"),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        ),
        trade_request: request_response::cbor::Behaviour::new(
            [(
                StreamProtocol::new("/trade-request/1"),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        ),
        rendezvous: rendezvous_behaviour,
    })
}

/// High level event handler, filters by behaviour type and delegates to lower-level handlers
pub async fn handle_event(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    event: SwarmEvent<SwapBytesBehaviourEvent>,
    chat_state: &mut ChatState,
    file_store: &mut LocalFileStore,
) {
    match event {
        // Gossipsub and MDNS (peer discovery and chat)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Chat(event)) => {
            handle_chat_event(swarm, event, chat_state)
        }

        // Kad events (any data thats supposed to be public, file metadata at the moment)
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::Kademlia(
            kad::Event::OutboundQueryProgressed { id, result, .. },
        )) => handle_kad_event(id, swarm, result, chat_state),

        // File sharing with request/response pattern
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::FileTransfer(
            request_response::Event::Message { peer, message, .. },
        )) => handle_file_transfer_event(peer, message, swarm, chat_state, file_store).await,

        // Direct messages with request/response pattern
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::DirectMessage(
            request_response::Event::Message { message, .. },
        )) => handle_direct_message_event(message, swarm).await,

        // Nickname updates with request/response pattern
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::NicknameUpdate(
            request_response::Event::Message { peer, message, .. },
        )) => handle_nickname_event(peer, message, swarm, chat_state).await,

        // Async Trade requests with request/response pattern
        SwarmEvent::Behaviour(SwapBytesBehaviourEvent::TradeRequest(
            request_response::Event::Message { peer, message, .. },
        )) => handle_trade_request_event(peer, message, swarm, chat_state, file_store).await,

        SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == chat_state.rendezvous => {
            if let Err(error) = swarm.behaviour_mut().rendezvous.rendezvous.register(
                rendezvous::Namespace::from_static("rendezvous"),
                chat_state.rendezvous,
                None,
            ) {
                println!("Failed to register: {error}");
                return;
            }
            println!("Connection established with rendezvous point {}", peer_id);

            swarm.behaviour_mut().rendezvous.rendezvous.discover(
                Some(rendezvous::Namespace::new("rendezvous".to_string()).unwrap()),
                None,
                None,
                chat_state.rendezvous,
            )
        }

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

                swarm
                    .behaviour_mut()
                    .kademlia
                    .get_closest_peers(peer_id.clone());
            }
        }

        // When a peer has left the network
        ChatBehaviourEvent::Mdns(mdns::Event::Expired(list)) => {
            for (peer_id, _) in list {
                let peer_id_str = peer_id.to_string();
                let nickname = chat_state.nicknames.get(&peer_id_str);
                println!("{} has left the network", nickname);

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
            // Try to interpret the message as a ChatMessage
            if let Ok(chat) = serde_cbor::from_slice::<ChatMessage>(&message.data) {
                chat_state
                    .nicknames
                    .insert(peer_id.to_string(), chat.nickname.clone());
                println!("{}: {}", chat.nickname, chat.message);
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
            // Match on the custom response type (file, file_index, etc)
            let record_key = String::from_utf8_lossy(peer_record.record.key.as_ref());
            match record_key.as_ref() {
                // File metadata responses
                key if key.starts_with("file::") => {
                    // Deduplicate
                    if !chat_state.pending_keys.remove(&id) {
                        return;
                    }
                    match serde_cbor::from_slice::<FileMetadata>(&peer_record.record.value) {
                        Ok(metadata) => {
                            println!(
                                "\t{} - {} ({} bytes) - {}",
                                metadata.hash,
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
                    // Deduplicate
                    if peer_record.peer.is_none() || !chat_state.pending_keys.remove(&id) {
                        return;
                    }

                    match serde_cbor::from_slice::<Vec<String>>(&peer_record.record.value) {
                        Ok(hashes) => {
                            let file_count = hashes.len();
                            let peerid_str = peer_record
                                .peer
                                .map_or("Someone".to_string(), |peer_id| peer_id.to_string());
                            println!(
                                "{} has uploaded {} file{}:",
                                chat_state.nicknames.get(&peerid_str),
                                file_count,
                                if file_count == 1 { "" } else { "s" }
                            );

                            // For each file listed, request the metadata of it
                            hashes.iter().for_each(|hash| {
                                let key = kad::RecordKey::new(&format!("file::{}", hash));
                                let queryid = swarm.behaviour_mut().kademlia.get_record(key);
                                chat_state.pending_keys.insert(queryid);
                            });
                        }
                        Err(e) => {
                            println!("Failed to parse file index for {key}: {e}");
                        }
                    }
                }

                // If the record type isn't defined
                _ => println!("Unexpected record type: {}", record_key),
            }
        }

        // Once bootstrapping is complete, fetch nicknames from peers
        kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { num_remaining, .. })) => {
            if num_remaining == 0 {
                let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
                // For each peer, request their nickname
                for peer in peers {
                    swarm
                        .behaviour_mut()
                        .nickname_update
                        .send_request(&peer, NicknameUpdate(chat_state.nickname.clone()));
                }
            }
        }

        // default => println!("{default:?}")
        _ => {}
    }
}

async fn handle_direct_message_event(
    message: Message<DirectMessage, AcknowledgeResponse>,
    swarm: &mut Swarm<SwapBytesBehaviour>,
) {
    match message {
        Message::Request {
            request, channel, ..
        } => {
            // Output DM to user
            println!("*DM* {}: {}", request.sender_nickname, request.message);

            // Send response so request is fulfilled
            if let Err(_) = swarm
                .behaviour_mut()
                .direct_message
                .send_response(channel, AcknowledgeResponse(true))
            {
                eprintln!("Failed to send response.")
            };
        }

        // Ignore response messages
        Message::Response { .. } => {}
    }
}

/// Handles NicknameUpdate requests/responses.
async fn handle_nickname_event(
    peer_id: PeerId,
    message: Message<NicknameUpdate, NicknameUpdate>,
    swarm: &mut Swarm<SwapBytesBehaviour>,
    chat_state: &mut ChatState,
) {
    match message {
        // Someone's updating us with their nickname, and asking for ours.
        Message::Request {
            request, channel, ..
        } => {
            chat_state.nicknames.insert(peer_id.to_string(), request.0);
            if let Err(_) = swarm
                .behaviour_mut()
                .nickname_update
                .send_response(channel, NicknameUpdate(chat_state.nickname.clone()))
            {
                eprintln!("Failed to send nickname acknowledgement")
            }
        }

        // A response to our nickname request, save it in the app state
        Message::Response { response, .. } => {
            chat_state.nicknames.insert(peer_id.to_string(), response.0);
        }
    }
}

/// Handles trade requests/responses. Handling is async so users aren't blocked during a request
async fn handle_trade_request_event(
    peer_id: PeerId,
    message: Message<TradeRequest, AcknowledgeResponse>,
    swarm: &mut Swarm<SwapBytesBehaviour>,
    chat_state: &mut ChatState,
    file_store: &mut LocalFileStore,
) {
    match message {
        // Someone's asking to trade files with us, and asking for ours.
        Message::Request {
            request, channel, ..
        } => {
            let requested_file_exists = file_store.contains_file(&request.requested_file);
            if requested_file_exists {
                chat_state
                    .incoming_trades
                    .insert(peer_id.to_string(), request.clone());

                let requested_file = file_store.get_metadata(&request.requested_file).unwrap();
                println!(
                    "{} would like to trade '{}' for their '{}'{}. Type '/trade_accept {}' to confirm trade.",
                    request.nickname,
                    requested_file.filename,
                    request.offered_file.filename,
                    request.offered_file.description
                        .as_ref()
                        .map(|desc| format!(" ({})", desc))
                        .unwrap_or_default(),
                    request.nickname
                );
            }
            if let Err(_) = swarm
                .behaviour_mut()
                .trade_request
                .send_response(channel, AcknowledgeResponse(requested_file_exists))
            {
                eprintln!("Failed to send nickname acknowledgement")
            }
        }

        // A acknowledgement response to our trade request, represents whether the requested file exists
        Message::Response { response, .. } => {
            match response {
                // Other user acknowledged the trade request, they have the file but are deciding
                AcknowledgeResponse(true) => {}
                // Other user doesn't have the file, tell user and forget about it
                AcknowledgeResponse(false) => {
                    chat_state.outgoing_trades.remove(&peer_id.to_string());
                    eprintln!(
                        "{} does not have the requested file",
                        chat_state.nicknames.get(&peer_id.to_string())
                    )
                }
            };
        }
    }
}

async fn handle_file_transfer_event(
    peer_id: PeerId,
    message: Message<Option<FileResponse>, Option<FileResponse>>,
    swarm: &mut Swarm<SwapBytesBehaviour>,
    chat_state: &mut ChatState,
    file_store: &mut LocalFileStore,
) {
    match message {
        // Someone has accepted our trade request and sent their file.
        Message::Request {
            request, channel, ..
        } => {
            match request {
                Some(file_response) => {
                    // Fetch the related trade request, otherwise drop the request
                    let Some(trade_details) =
                        chat_state.outgoing_trades.remove(&peer_id.to_string())
                    else {
                        if let Err(_) = swarm
                            .behaviour_mut()
                            .file_transfer
                            .send_response(channel, None)
                        {
                            eprintln!("Failed to send file response");
                        }
                        return;
                    };

                    // Save the file sent by the other party
                    if let Err(e) = save_file_to_filesystem(
                        file_response.file,
                        &file_response.metadata.filename,
                    )
                    .await
                    {
                        eprintln!("Failed to save file: {}", e);
                    }

                    // Construct response and send it
                    let file_bytes = file_store
                        .get_file(&trade_details.offered_file.hash)
                        .unwrap_or_default();
                    let response = FileResponse {
                        file: file_bytes,
                        metadata: trade_details.offered_file,
                    };
                    if let Err(_) = swarm
                        .behaviour_mut()
                        .file_transfer
                        .send_response(channel, Some(response))
                    {
                        eprintln!("Failed to send file response");
                    }

                    println!("Trade successful!")
                }

                // Trade request declined, remove trade request from state
                None => {
                    let peer_id_str = peer_id.to_string();
                    chat_state.outgoing_trades.remove(&peer_id_str);

                    let nickname = chat_state.nicknames.get(&peer_id_str);
                    println!("Your trade request with {} was declined", nickname)
                }
            }
        }

        // Someone has sent their file, so we send our file back
        Message::Response { response, .. } => {
            match response {
                Some(file) => {
                    // The trade is complete, remove its reference from the state
                    chat_state.incoming_trades.remove(&peer_id.to_string());

                    // Save the file sent by the other party
                    if let Err(e) =
                        save_file_to_filesystem(file.file, &file.metadata.filename).await
                    {
                        eprintln!("Failed to save file: {}", e);
                        return;
                    }

                    println!("Trade successful!")
                }

                None => eprintln!("File transfer failed."),
            }
        }
    }
}
