use std::error::Error;
use std::collections::HashMap;
use libp2p::kad::{self, store::MemoryStore};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::io;
use std::io::{ Write, stdout };

use crate::events::SwapBytesBehaviour;

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peerid: String,
    pub nickname: String,
}

pub struct ChatState {
    pub pending_messages: HashMap<kad::QueryId, (PeerId, Vec<u8>)>,
}

impl ChatState {
    pub fn new() -> ChatState {
        ChatState {
            pending_messages: HashMap::new()
        }
    }
}

pub fn add_peer_to_store(
    store: &mut kad::Behaviour<MemoryStore>,
    peerid: String,
    nickname: String
) -> Result<(), Box<dyn Error>>  {
    let peerid = peerid.clone();
    let nickname = nickname.clone();
    let peer_info = PeerInfo {
        peerid: peerid.clone(),
        nickname: nickname.clone()
    };
    let peer_info_bytes = serde_cbor::to_vec(&peer_info)?;

    // Save peerid -> data relation
    let record = kad::Record {
        key: kad::RecordKey::new(&peerid),
        value: peer_info_bytes.clone(),
        publisher: None,
        expires: None,
    };
    store
        .put_record(record, kad::Quorum::One)
        .expect("Failed to store peer.");

    // Save nickname -> data relation
    let record = kad::Record {
        key: kad::RecordKey::new(&nickname),
        value: peer_info_bytes,
        publisher: None,
        expires: None,
    };
    store
        .put_record(record, kad::Quorum::One)
        .expect("Failed to store peer.");

    return Ok(());
}

pub async fn prompt_for_nickname(
    stdin: &mut io::Lines<io::BufReader<io::Stdin>>,
    swarm: &mut libp2p::Swarm<SwapBytesBehaviour>
) {
    let mut nickname = String::new();
    let mut is_valid = false;

    while !is_valid {
        // Ask for input, print + flush() makes it be on one line.
        print!("Enter a nickname: ");
        stdout().flush().unwrap();

        match stdin.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    nickname = trimmed.to_string();
                    is_valid = true;
                } else {
                    println!("Nickname cannot be empty. Please enter a valid nickname.");
                }
            }
            Ok(None) => {
                println!("No input received. Please try again.");
            }
            Err(e) => {
                println!("Error reading input: {}. Please try again.", e);
            }
        }
    }
    set_nickname(swarm, nickname);
}

pub fn set_nickname(swarm: &mut libp2p::Swarm<SwapBytesBehaviour>, nickname: String) {
    let peer_id = swarm.local_peer_id().to_string();
    // Try and make 
    add_peer_to_store(
        &mut swarm.behaviour_mut().kademlia,
        peer_id.clone(),
        nickname.clone() + "." + &peer_id[47..],
    ).expect("Couldn't add nickname to store");
    println!("Nickname set to '{}'", nickname)
}