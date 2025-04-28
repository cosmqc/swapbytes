use libp2p::gossipsub::IdentTopic;
use libp2p::kad;
use libp2p::{PeerId, Swarm};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{stdout, Write};
use tokio::io;

use crate::events::SwapBytesBehaviour;

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

pub struct ChatState {
    pub pending_keys: HashSet<kad::QueryId>,
    pub nicknames: NicknameMap,
    pub current_topic: IdentTopic,
    pub nickname: String
}

impl ChatState {
    pub fn new(nickname: String) -> ChatState {
        ChatState {
            pending_keys: HashSet::new(),
            nicknames: NicknameMap::new(),
            current_topic: IdentTopic::new("chat"),
            nickname
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
    println!("Nickname set to '{}'", nickname);
    process_nickname(swarm, &nickname)
}

// Sets the nickname to the given value
pub fn process_nickname(swarm: &mut libp2p::Swarm<SwapBytesBehaviour>, nickname: &str) -> String {
    let peerid = swarm.local_peer_id().to_string();
    nickname.to_owned() + "." + &peerid[47..]
}
