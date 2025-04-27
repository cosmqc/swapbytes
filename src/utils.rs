use libp2p::gossipsub::IdentTopic;
use libp2p::kad::{self, store::MemoryStore};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::io::{stdout, Write};
use tokio::io;

use crate::events::SwapBytesBehaviour;

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peerid: String,
    pub nickname: String,
}

pub struct ChatState {
    pub pending_messages: HashMap<kad::QueryId, (PeerId, Vec<u8>)>,
    pub handled_keys: HashSet<String>,
    pub peer_to_nickname: HashMap<String, String>,
    pub nickname_to_peer: HashMap<String, String>,
    pub current_topic: IdentTopic,
}

impl ChatState {
    pub fn new() -> ChatState {
        ChatState {
            pending_messages: HashMap::new(),
            handled_keys: HashSet::new(),
            peer_to_nickname: HashMap::new(),
            nickname_to_peer: HashMap::new(),
            current_topic: IdentTopic::new("chat"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicknameUpdate {
    pub peer_id: String,
    pub nickname: String,
}

/// Adds a (peerid, nickname) pair to the DHT.  
pub fn add_peer_to_store(
    store: &mut kad::Behaviour<MemoryStore>,
    peerid: PeerId,
    nickname: String,
    chat_state: &mut ChatState
) -> Result<(), Box<dyn Error>> {
    let peerid = peerid.clone();
    let nickname = nickname.clone();
    let peer_info = PeerInfo {
        peerid: peerid.to_string(),
        nickname: nickname.clone(),
    };
    let peer_info_bytes = serde_cbor::to_vec(&peer_info)?;

    // Save peerid -> data relation
    let record = kad::Record {
        key: kad::RecordKey::new(&format!("peer::{}", peerid)),
        value: peer_info_bytes.clone(),
        publisher: Some(peerid),
        expires: None,
    };
    store
        .put_record(record, kad::Quorum::One)
        .expect("Failed to store peer w peerid key.");

    // Save nickname -> data relation
    let record = kad::Record {
        key: kad::RecordKey::new(&format!("peer::{}", nickname)),
        value: peer_info_bytes.clone(),
        publisher: Some(peerid),
        expires: None,
    };
    store
        .put_record(record, kad::Quorum::One)
        .expect("Failed to store peer w nickname key.");

    chat_state.peer_to_nickname.insert(peerid.to_string(), nickname.clone());
    chat_state.nickname_to_peer.insert(nickname, peerid.to_string());

    return Ok(());
}

/// Prompts the user for a nickname until it gets a valid one, then sets it with a confirmation message
pub async fn prompt_for_nickname(
    stdin: &mut io::Lines<io::BufReader<io::Stdin>>,
    swarm: &mut libp2p::Swarm<SwapBytesBehaviour>,
    chat_state: &mut ChatState
) {
    let mut nickname = String::new();
    let mut is_valid = false;

    while !is_valid {
        // Ask for input, print + flush() makes it be on one line.
        print!("Enter a nickname: ");
        stdout().flush().unwrap();

        match stdin.next_line().await {
            Ok(Some(line)) if !line.trim().is_empty() => {
                nickname = line.trim().to_string();
                is_valid = true;
            }
            Ok(Some(_)) => println!("Nickname cannot be empty. Please enter a valid nickname."),
            Ok(None) => println!("No input received. Please try again."),
            Err(e) => println!("Error reading input: {e}. Please try again."),
        }
        
    }
    let new_nickname = set_nickname(swarm, Some(nickname), chat_state);
    println!("Nickname set to '{}'", new_nickname)
}

/// Sets the nickname to the given value and broadcasts it, if no nickname given then just broadcasts the current one
pub fn set_nickname(swarm: &mut libp2p::Swarm<SwapBytesBehaviour>, nickname: Option<String>, chat_state: &mut ChatState) -> String{
    let peer_id = swarm.local_peer_id().clone();
    let new_nickname = if nickname.is_some() {
        nickname.unwrap() + "." + &peer_id.to_string()[47..]
    } else {
        chat_state.peer_to_nickname.get(&peer_id.to_string()).unwrap_or(
            &"Failed to get own nickname".to_string()
        ).to_string()
    };

    let update = NicknameUpdate {
        peer_id: peer_id.to_string(),
        nickname: new_nickname.clone(),
    };
    
    let msg = serde_cbor::to_vec(&update).expect("Failed to serialize nickname");
    let _ = swarm
        .behaviour_mut()
        .chat
        .gossipsub
        .publish(chat_state.current_topic.clone(), msg);

    // Update kad store so new people can get nicknames on demand
    add_peer_to_store(
        &mut swarm.behaviour_mut().kademlia,
        peer_id,
        new_nickname.clone(),
        chat_state
    ).expect("Couldn't add nickname to store");

    new_nickname
}
