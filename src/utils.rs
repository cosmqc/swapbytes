use libp2p::gossipsub::IdentTopic;
use libp2p::kad;
use libp2p::Multiaddr;
use libp2p::PeerId;
use libp2p::Swarm;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{stdout, Write};
use tokio::io;

use crate::events::SwapBytesBehaviour;
use crate::files::FileMetadata;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeerInfo {
    pub peerid: String,
    pub nickname: String,
}

pub struct NicknameMap {
    inner: HashMap<String, String>
}

impl NicknameMap {
    pub fn new() -> Self {
        NicknameMap {
            inner: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.inner.insert(key, value);
    }

    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        let poss_value = self.inner.get(key);
        match poss_value {
            Some(value) => value,
            None => key,
        }
    }

    pub fn get_key_from_value(&self, value: &str) -> Option<String> {
        for (key, val) in &self.inner {
            if *val == value {
                return Some(key.clone());
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRequest {
    pub requested_file: String,
    pub offered_file: FileMetadata,
    pub nickname: String,
}

pub struct ChatState {
    pub pending_keys: HashSet<kad::QueryId>,
    pub nicknames: NicknameMap,
    pub current_topic: IdentTopic,
    pub incoming_trades: HashMap<String, TradeRequest>,
    pub outgoing_trades: HashMap<String, TradeRequest>,
    pub nickname: String,
    pub rendezvous: PeerId
}

impl ChatState {
    pub fn new(nickname: String) -> ChatState {
        ChatState {
            pending_keys: HashSet::new(),
            nicknames: NicknameMap::new(),
            current_topic: IdentTopic::new("chat"),
            incoming_trades: HashMap::new(),
            outgoing_trades: HashMap::new(),
            nickname,
            rendezvous: "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN".parse::<PeerId>().unwrap()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicknameUpdate(pub String);

/// Prompts the user for a nickname until it gets a valid one, then sets it with a confirmation message
pub async fn prompt_for_nickname(
    stdin: &mut io::Lines<io::BufReader<io::Stdin>>,
    swarm: &mut Swarm<SwapBytesBehaviour>
) -> String {
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
    let new_nickname = process_nickname(swarm, &nickname);
    println!("Nickname set to '{}'", new_nickname);
    new_nickname
}

// Sets the nickname to the given value
pub fn process_nickname(swarm: &mut libp2p::Swarm<SwapBytesBehaviour>, nickname: &str) -> String {
    let peerid = swarm.local_peer_id().to_string();
    nickname.to_owned() + "." + &peerid[47..]
}
