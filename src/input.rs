use libp2p::gossipsub::IdentTopic;
use libp2p::kad;
use libp2p::swarm::Swarm;
use libp2p::PeerId;
use regex::Regex;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use tokio::io::{BufReader, Lines, Stdin};
use serde::{Deserialize, Serialize};

use crate::files::{DirectMessage, FileResponse, LocalFileStore};
use crate::events::SwapBytesBehaviour;
use crate::utils::{self, prompt_for_nickname, ChatState, TradeRequest};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub message: String,
    pub nickname: String
}

pub async fn handle_input_line(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    line: String,
    current_topic: IdentTopic,
    stdin: &mut Lines<BufReader<Stdin>>,
    file_store: &mut LocalFileStore,
    chat_state: &mut ChatState,
) -> Result<(), Box<dyn Error>> {
    if line.trim().is_empty() {
        return Ok(());
    }

    // All inputs without the command prefix should just get sent as messages
    if line.chars().nth(0).unwrap() != '/' {
        let message = ChatMessage {
            message: line,
            nickname: chat_state.nickname.clone()
        };

        let message_bytes = serde_cbor::to_vec(&message)?;

        if let Err(e) = swarm
        .behaviour_mut()
        .chat
        .gossipsub
        .publish(current_topic, message_bytes) {
            eprintln!("Failed to send message: {}", e);
        }

        return Ok(());
    }

    // Strip the forward slash, handle the case where only a forward slash is given
    let args = split_string(&line[1..]);
    let Some(cmd) = args.get(0) else {
        println!("No command given");
        return Ok(());
    };

    // Match on the command given
    match cmd.to_lowercase().as_str() {
        "help" => {
            println!("So you want help? *mr krabs playing the worlds smallest violin*");
            Ok(())
        }

        "nick" => {
            // Wrong number of arguments
            if args.len() > 3 {
                println!("Invalid number of arguments. Correct usage: /nick <nickname>. Use quotes if your nickname has multiple words.");
                return Ok(());
            }

            // If only the command provided, enter prompt loop
            let Some(nickname) = args.get(1) else {
                let nickname = prompt_for_nickname(stdin, swarm).await;
                chat_state.nickname = nickname;
                return Ok(());
            };

            // If given nickname is empty, enter prompt loop
            if nickname.trim().is_empty() {
                println!("Nickname cannot be empty. Please enter a valid nickname.");
                let nickname = prompt_for_nickname(stdin, swarm).await;
                chat_state.nickname = nickname;
                return Ok(());
            }

            let nickname = utils::process_nickname(swarm, nickname);
            println!("Nickname set to '{}'", nickname);
            chat_state.nickname = nickname;

            Ok(())
        }

        "upload" => {
            if !(2..=3).contains(&args.len()) {
                println!("Usage: /upload <path_to_file> <description?>");
                return Ok(());
            }

            // Validate file exists
            let file_path = Path::new(&args[1]);
            if !file_path.exists() {
                println!("File not found: {}", file_path.display());
                return Ok(());
            }

            // Read the file bytes
            let file_bytes = match fs::read(file_path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    println!("Failed to read file: {e}");
                    return Ok(());
                }
            };

            // Extract the filename from the path, validate its not some scuffed encoding
            let filename = match file_path.file_name().and_then(|f| f.to_str()) {
                Some(name) => name,
                None => {
                    println!("Invalid filename.");
                    return Ok(());
                }
            };
            
            // Get description if it exists
            let description = args.get(2).cloned();

            // Share file metadata to peers
            let peer_id = swarm.local_peer_id().clone();
            let hash = file_store.add_file(
                file_bytes, 
                filename, 
                &peer_id,
                description
            );
            if let Some(metadata) = file_store.get_metadata(&hash) {
                if let Ok(serialized) = serde_cbor::to_vec(metadata) {
                    let record = kad::Record {
                        key: kad::RecordKey::new(&format!("file::{}", hash)),
                        value: serialized,
                        publisher: Some(peer_id),
                        expires: None,
                    };
            
                    if let Err(e) = swarm
                        .behaviour_mut()
                        .kademlia
                        .put_record(record, kad::Quorum::One)
                    {
                        println!("Error publishing metadata: {e}");
                    } else {
                        println!("Uploaded and shared metadata for file {} with hash {}", filename, hash);
                    }
                } else {
                    println!("Error serializing metadata");
                }
            }

            // Update a set of what files we have on the DHT, makes it easier to query everyone's files.
            let file_hashes = file_store.all_hashes();
            let index_key = format!("file_index::{}", swarm.local_peer_id());
            let record = kad::Record {
                key: kad::RecordKey::new(&index_key),
                value: serde_cbor::to_vec(&file_hashes)?,
                publisher: Some(peer_id),
                expires: None,
            };

            swarm
                .behaviour_mut()
                .kademlia
                .put_record(record, kad::Quorum::One)
                .expect("Failed to update file list");

            Ok(())
        }

        // "get_file_metadata" => {
        //     if args.len() < 2 || args[1].len() == 0 {
        //         println!("Usage: /get_file_metadata <file_hash>");
        //         return Ok(());
        //     }

        //     let file_hash = args[1].clone();

        //     // Query the DHT for the file metadata
        //     let key = kad::RecordKey::new(&format!("file::{}", file_hash));
        //     swarm.behaviour_mut().kademlia.get_record(key.clone());

        //     Ok(())
        // }

        // "list_files" => {
        //     let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
        //     for peer_id in peers {
        //         let key = kad::RecordKey::new(&format!("file_index::{}", peer_id));
        //         let queryid = swarm.behaviour_mut().kademlia.get_record(key);
        //         chat_state.pending_keys.insert(queryid);
        //     }
        //     Ok(())
        // }

        "dm" => {
            if args.len() != 3 {
                println!("Usage: /dm <nickname> <message>");
                return Ok(());
            }
            let Some(nickname) = args.get(1) else {
                eprintln!("Failed to parse nickname");
                return Ok(());
            };
            let Some(peer_id_str) = chat_state.nicknames.get_key_from_value(nickname) else {
                eprintln!("Nickname not found");
                return Ok(());
            };
            let Ok(peerid) = PeerId::from_str(&peer_id_str) else {
                eprintln!("Failed to parse retrieved nickname");
                return Ok(());
            };
            let Some(message) = args.get(2) else {
                eprintln!("Failed to parse message");
                return Ok(());
            };

            swarm
                .behaviour_mut()
                .direct_message
                .send_request(
                    &peerid, 
                    DirectMessage {
                        message: message.clone(),
                        sender_nickname: chat_state.nickname.clone()
                    }
                );
            
            Ok(())
        }

        "trade" => {
            if args.len() != 4 {
                println!("Usage: /trade <nickname> <offered_file_hash> <requested_file_hash>");
                return Ok(());
            }
            // Process nickname
            let Some(nickname) = args.get(1) else {
                eprintln!("Failed to parse nickname");
                return Ok(());
            };
            let Some(peer_id_str) = chat_state.nicknames.get_key_from_value(nickname) else {
                eprintln!("Nickname not found");
                return Ok(());
            };
            let Ok(peerid) = PeerId::from_str(&peer_id_str) else {
                eprintln!("Failed to parse retrieved nickname");
                return Ok(());
            };

            // Process offered file hash
            let Some(offered_hash) = args.get(2) else {
                eprintln!("Failed to parse file hash of the offered file");
                return Ok(());
            };

            // Make sure the file the user is offering exists
            let Some(offered_file) = file_store.get_metadata(offered_hash) else {
                eprintln!("Offered file does not exist");
                return Ok(());
            };

            // Process requested file hash
            let Some(requested_hash) = args.get(3) else {
                eprintln!("Failed to parse file hash of the requested file");
                return Ok(());
            };

            // Create the request and send it
            let trade = TradeRequest {
                offered_file: offered_file.clone(),
                requested_file: requested_hash.clone(),
                nickname: chat_state.nickname.clone()
            };

            chat_state.outgoing_trades.insert(peerid.to_string(), trade.clone());
            swarm.behaviour_mut()
                .trade_request
                .send_request(&peerid,trade);

            println!("Trade request sent to {}, transfer will happen once they accept", nickname);

            Ok(())
        }

        "trade_accept" => {
            if args.len() != 2 {
                println!("Usage: /trade_accept <nickname>");
                return Ok(());
            }

            // Process nickname
            let Some(nickname) = args.get(1) else {
                eprintln!("Failed to parse nickname");
                return Ok(());
            };
            let Some(peer_id_str) = chat_state.nicknames.get_key_from_value(nickname) else {
                eprintln!("Nickname not found");
                return Ok(());
            };
            let Ok(peerid) = PeerId::from_str(&peer_id_str) else {
                eprintln!("Failed to parse retrieved nickname");
                return Ok(());
            };

            let Some(trade_request) = chat_state.incoming_trades.get(&peerid.to_string()) else {
                eprintln!("You don't have a trade request from this user");
                return Ok(());
            };

            // Check the requested file exists. This should have already been checked, but just incase
            let Some(requested_file) = file_store.get_file(&trade_request.requested_file) else {
                eprintln!("The requested file doesn't exist. Something has gone wrong.");
                return Ok(());
            };

            // Fetch the metadata
            let Some(metadata) = file_store.get_metadata(&trade_request.requested_file) else {
                eprintln!("Failed to get the metadata of the requested file.");
                return Ok(());
            };

            let response = FileResponse{
                file: requested_file,
                metadata: metadata.clone()
            };

            swarm.behaviour_mut().file_transfer.send_request(&peerid, response);

            Ok(())
        }

        default => {
            println!("Command not recognized: {}", default);
            Ok(())
        }
    }
}

/// Splits a command string into its different parts
/// Double quoted strings gets captured as a whole but without the quotes
/// i.e. 'NICK "super man"' will return ['NICK', 'super man']
fn split_string(input: &str) -> Vec<String> {
    let re = Regex::new(r#""([^"]*)"|\S+"#).unwrap();
    re.captures_iter(input)
        // I don't understand this, but the conditions can't be flipped around
        .filter_map(|cap| {
            if let Some(matched) = cap.get(1) {
                Some(matched.as_str().to_string())
            } else if let Some(matched) = cap.get(0) {
                Some(matched.as_str().to_string())
            } else {
                None
            }
        })
        .collect()
}
