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

use crate::files::{LocalFileStore, FileRequest};
use crate::events::SwapBytesBehaviour;
use crate::utils::{self, add_peer_to_store, prompt_for_nickname, ChatState};

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
        swarm
            .behaviour_mut()
            .chat
            .gossipsub
            .publish(current_topic, line.as_bytes())
            .expect("Failed to send message");
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
                prompt_for_nickname(stdin, swarm, chat_state).await;
                return Ok(());
            };

            // If given nickname is empty, enter prompt loop
            if nickname.trim().is_empty() {
                println!("Nickname cannot be empty. Please enter a valid nickname.");
                prompt_for_nickname(stdin, swarm, chat_state).await;
                return Ok(());
            }

            // Otherwise, set nickname to whatevers given
            utils::set_nickname(swarm, Some(nickname.clone()), chat_state);

            Ok(())
        }

        "get_peer" => {
            if args.len() < 2 || args[1].len() == 0 {
                println!("not enough arguments");
                return Ok(());
            }
            let title = args[1].clone();
            let key = kad::RecordKey::new(&title);
            swarm.behaviour_mut().kademlia.get_record(key);
            return Ok(());
        }

        "add_peer" => {
            if args.len() < 3 || args[1].len() == 0 {
                println!("Not enough arguments");
                return Ok(());
            }
            add_peer_to_store(
                &mut swarm.behaviour_mut().kademlia,
                PeerId::from_str(&args[1].to_string())?,
                args[2].clone(),
                chat_state,
            ).expect("Failed to add peer to store");

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
                        println!("Uploaded and shared metadata for file: {}", filename);
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

        "get_file_metadata" => {
            if args.len() < 2 || args[1].len() == 0 {
                println!("Usage: /get_file_metadata <file_hash>");
                return Ok(());
            }

            let file_hash = args[1].clone();

            // Query the DHT for the file metadata
            let key = kad::RecordKey::new(&format!("file::{}", file_hash));
            swarm.behaviour_mut().kademlia.get_record(key.clone());

            Ok(())
        }

        "list_files" => {
            let peers: Vec<PeerId> = swarm.connected_peers().cloned().collect();
            for peer_id in peers {
                let key = kad::RecordKey::new(&format!("file_index::{}", peer_id));
                swarm.behaviour_mut().kademlia.get_record(key);
            }
            Ok(())
        }

        "request_file" => {
            let Some(peer_id_str) = args.get(1) else {
                eprintln!("Failed to get peerid");
                return Ok(());
            };

            let Ok(peer_id) = PeerId::from_str(peer_id_str) else {
                eprintln!("Invalid peerid");
                return Ok(());
            };

            let Some(filename) = args.get(2) else {
                eprintln!("Failed to get filename");
                return Ok(());
            };

            println!("{:?}", swarm.connected_peers().collect::<Vec<_>>());
            swarm.behaviour_mut().request_response.send_request(&peer_id, FileRequest(filename.clone()));

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
