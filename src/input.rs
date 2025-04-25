use std::error::Error;
use libp2p::gossipsub::IdentTopic;
use libp2p::kad;
use regex::Regex;
use crate::utils::add_peer_to_store;
use crate::events::SwapBytesBehaviour;
use libp2p::swarm::Swarm;

pub fn handle_input_line(
    swarm: &mut Swarm<SwapBytesBehaviour>,
    line: String,
    current_topic: IdentTopic
) -> Result<(), Box<dyn Error>> {
    let args = split_string(&line);
    let cmd = if let Some(cmd) = args.get(0) {
        cmd
    } else {
        println!("No command given");
        return Ok(());
    };

    match cmd.as_str() {
        "GET_PEER" => { 
            if args.len() < 2 || args[1].len() == 0 {
                println!("not enough arguments");
                return Ok(());
            }
            let title = args[1].clone();
            let key = kad::RecordKey::new(&title);
            swarm.behaviour_mut().kademlia.get_record(key);
            return Ok(());
        },

        "ADD_PEER" => {
            if args.len() < 3 || args[1].len() == 0 {
                println!("Not enough arguments");
                return Ok(());
            }
            let _ = add_peer_to_store(&mut swarm.behaviour_mut().kademlia, args[1].clone(), args[2].clone());

            Ok(())
        },
        
        _ => {
            swarm.behaviour_mut().chat.gossipsub.publish(current_topic, line.as_bytes())
                .expect("Failed to send message");

            Ok(())
        },
    }
}

fn split_string(input: &str) -> Vec<String> {
    let re = Regex::new(r#""([^"]*)"|\S+"#).unwrap();
    re.captures_iter(input)
        .map(|cap| cap.get(0).unwrap().as_str().to_string())
        .collect()
}